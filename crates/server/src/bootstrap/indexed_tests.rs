use super::*;
use kiarni_core::{DEFAULT_MAX_DOCUMENT_BYTES, DocumentInput};
use kiarni_engine::query::Query;
use kiarni_storage::{Error, IndexedAccess};
use std::{
    os::unix::fs::PermissionsExt,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

fn seed(store: &Store) {
    let collection = store.create_collection("items").unwrap();
    for n in 0..16 {
        let status = if n == 0 { "wanted" } else { "bulk" };
        let bytes = format!(r#"{{"_id":"d{n}","status":"{status}"}}"#);
        let input = DocumentInput::parse(bytes.as_bytes(), DEFAULT_MAX_DOCUMENT_BYTES)
            .unwrap()
            .prepare_insert()
            .unwrap();
        store.insert_document(&collection, &input).unwrap();
    }
    store.create_index(&collection, "status").unwrap();
}

fn assert_indexed(store: &Store) {
    let collection = store.collection("items").unwrap();
    let query = Query::parse(
        br#"{"from":"items","limit":10,"filter":{"op":"eq","field":"status","value":"wanted"}}"#,
    )
    .unwrap();
    let result = store.query(&collection, &query).unwrap();
    assert_eq!(result.plan, IndexedAccess::Secondary);
    assert_eq!(result.examined, 1);
    assert_eq!(result.documents.len(), 1);
    store.audit().unwrap();
}

#[test]
fn restored_store_gets_independent_credentials_and_revocation_is_not_reset() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let root = Root::lock(&source).unwrap();
    let (store, startup) = root.start(100).unwrap();
    seed(&store);
    let old_token = fs::read(startup.token_file.unwrap()).unwrap();
    let copied = temp.path().join("copied");
    let target = Root::lock(&copied).unwrap();
    store.export_snapshot(&copied.join("store")).unwrap();
    target.publish(Phase::StoreReady).unwrap();
    let (restored, first) = target.start(101).unwrap();
    assert!(first.created);
    assert_ne!(fs::read(first.token_file.unwrap()).unwrap(), old_token);
    assert_indexed(&restored);
    drop(restored);
    let active = fs::read(copied.join("auth/active.json")).unwrap();
    let (restored, expired) = target
        .start(102 + kiarni_auth::activation::DEFAULT_LIFETIME)
        .unwrap();
    assert!(!expired.created);
    assert!(expired.token_file.is_none());
    assert_eq!(fs::read(copied.join("auth/active.json")).unwrap(), active);
    drop(restored);
    let revoked = Auth::open(&copied, None).unwrap().revoke_all().unwrap();
    let (restored, resumed) = target.start(103).unwrap();
    assert_eq!(resumed.keyset.epoch(), revoked.keyset.epoch());
    assert!(resumed.token_file.is_none());
    assert!(!resumed.created);
    assert!(
        kiarni_auth::Authority::new(resumed.keyset, 103)
            .authenticate(&[], 103)
            .is_err()
    );
    assert_indexed(&restored);
    drop(restored);
    assert_indexed(&store);
}

#[test]
fn incomplete_index_or_missing_auth_cannot_authorize_new_delivery() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("corrupt");
    let root = Root::lock(&path).unwrap();
    let store = Store::create(&path.join("store")).unwrap();
    seed(&store);
    drop(store);
    root.publish(Phase::StoreReady).unwrap();
    let db = redb::Database::open(path.join("store/data.redb")).unwrap();
    let write = db.begin_write().unwrap();
    write
        .open_table(redb::TableDefinition::<(u64, &str, &str), ()>::new(
            "secondary_entries",
        ))
        .unwrap()
        .remove((1, "wanted", "d0"))
        .unwrap();
    write
        .open_table(redb::TableDefinition::<(u64, &str), u64>::new(
            "secondary_counts",
        ))
        .unwrap()
        .remove((1, "wanted"))
        .unwrap();
    write.commit().unwrap();
    db.close().unwrap();
    assert!(root.start(100).is_err());
    assert!(!path.join("auth").exists());
    assert!(!temp.path().join("corrupt.credentials").exists());
    assert!(matches!(
        Store::open(&path.join("store")),
        Err(Error::InvalidLayout)
    ));

    let path = temp.path().join("missing-auth");
    let root = Root::lock(&path).unwrap();
    drop(Store::create(&path.join("store")).unwrap());
    root.publish(Phase::InitializingAuth).unwrap();
    fs::create_dir(path.join("auth")).unwrap();
    fs::set_permissions(path.join("auth"), fs::Permissions::from_mode(0o700)).unwrap();
    assert!(root.start(100).is_err());
    let delivery = temp.path().join("missing-auth.credentials");
    assert!(!delivery.exists() || fs::read_dir(delivery).unwrap().count() == 0);
    assert!(!path.join("auth/active.json").exists());
}

struct KillOnDrop(Child);
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "subprocess worker for indexed bootstrap interruption tests"]
fn indexed_bootstrap_worker() {
    let Some(path) = std::env::var_os("KIARNI_TEST_INDEXED_ROOT") else {
        return;
    };
    let path = PathBuf::from(path);
    let point = std::env::var("KIARNI_TEST_INDEXED_POINT").unwrap();
    let signal = PathBuf::from(std::env::var_os("KIARNI_TEST_INDEXED_SIGNAL").unwrap());
    let pause = |at: &str| {
        if point == at {
            fs::write(&signal, b"ready").unwrap();
            loop {
                thread::park();
            }
        }
    };
    let root = Root::lock(&path).unwrap();
    root.publish(Phase::InitializingStore).unwrap();
    pause("initializing_store");
    let store = Store::create(&path.join("store")).unwrap();
    seed(&store);
    root.publish(Phase::StoreReady).unwrap();
    pause("store_ready");
    let auth = Auth::open(&path, None).unwrap();
    auth.prepare_initial_marker().unwrap();
    root.publish(Phase::InitializingAuth).unwrap();
    pause("initializing_auth");
    let _startup = auth.resume_prepared(100, false).unwrap();
    pause("auth_confirmed");
    root.publish(Phase::Initialized).unwrap();
    pause("initialized");
    drop(store);
}

#[test]
fn process_lock_and_kill_recovery_cover_each_bootstrap_phase() {
    for point in [
        "initializing_store",
        "store_ready",
        "initializing_auth",
        "auth_confirmed",
        "initialized",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("data");
        let signal = temp.path().join("signal");
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "bootstrap::indexed_tests::indexed_bootstrap_worker",
                "--ignored",
                "--nocapture",
            ])
            .env("KIARNI_TEST_INDEXED_ROOT", &path)
            .env("KIARNI_TEST_INDEXED_POINT", point)
            .env("KIARNI_TEST_INDEXED_SIGNAL", &signal)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut child = KillOnDrop(child);
        let deadline = Instant::now() + Duration::from_secs(8);
        while !signal.exists() {
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "worker exited: {point}"
            );
            assert!(Instant::now() < deadline, "checkpoint deadline: {point}");
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            Root::lock_existing(&path).err().unwrap().to_string(),
            "data_directory_locked"
        );
        child.0.kill().unwrap();
        child.0.wait().unwrap();
        let root = Root::lock_existing(&path).unwrap();
        let (store, startup) = root.start(101).unwrap();
        assert_eq!(
            startup.created,
            !matches!(point, "auth_confirmed" | "initialized")
        );
        assert!(root.phase().unwrap() == Some(Phase::Initialized));
        if point != "initializing_store" {
            assert_indexed(&store);
        }
        drop(store);
        let (store, startup) = root.start(102).unwrap();
        assert!(!startup.created);
        assert!(startup.token_file.is_none());
        store.audit().unwrap();
    }
}

#[test]
fn bootstrap_format_record_is_strict_and_checked_before_creation() {
    let invalid = [
        r#"{"version":2,"phase":"initializing_store"}"#,
        r#"{"version":1,"store_format":2,"phase":"initializing_store"}"#,
        r#"{"version":2,"store_format":1,"phase":"initializing_store"}"#,
        r#"{"version":3,"store_format":2,"phase":"initializing_store"}"#,
        r#"{"version":2,"store_format":2,"phase":"unknown"}"#,
        r#"{"version":2,"store_format":2,"phase":"initializing_store","extra":0}"#,
        r#"{"version":2,"store_format":2,"store_format":2,"phase":"initializing_store"}"#,
    ];
    for bytes in invalid
        .iter()
        .map(|v| v.as_bytes().to_vec())
        .chain([vec![b' '; 1025]])
    {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("data");
        let root = Root::lock(&path).unwrap();
        let record = path.join("bootstrap.json");
        fs::write(&record, &bytes).unwrap();
        fs::set_permissions(&record, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(root.start(100).is_err());
        assert!(root.start(100).is_err());
        assert_eq!(fs::read(&record).unwrap(), bytes);
        assert!(!path.join("store").exists());
        assert!(!path.join("auth").exists());
        assert!(!temp.path().join("data.credentials").exists());
    }
}
