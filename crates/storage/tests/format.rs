use std::{fs, path::Path};

use kiarni_storage::{Error, Store};

fn open_error(root: &Path) -> Error {
    Store::open(root).err().expect("must reject")
}

#[test]
fn generic_documents_primary_keys_and_catalog_survive_reopen() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("data");
    let first;
    let json =
        br#"{ "_id":"same", "n":123456789012345678901234567890, "v":[null,true], "e":1e10001 }"#;
    {
        let store = Store::create(&root).unwrap();
        first = store.create_collection("orders").unwrap();
        let second = store.create_collection("other").unwrap();
        store.insert_fixture(&first, json).unwrap();
        store.insert_fixture(&second, br#"{"_id":"same"}"#).unwrap();
        assert_eq!(store.insert_fixture(&first, json), Err(Error::DuplicateId));
        assert_eq!(
            store.drop_collection(&first),
            Err(Error::CollectionNotEmpty)
        );
        assert_eq!(
            store.get_fixture(&second, "same").unwrap().unwrap(),
            br#"{"_id":"same"}"#
        );
        assert_eq!(
            store.create_collection("orders"),
            Err(Error::CollectionExists)
        );
        assert_eq!(store.collection("missing"), Err(Error::CollectionMissing));
    }
    let store = Store::open(&root).unwrap();
    assert_eq!(store.collection("orders").unwrap(), first);
    assert_eq!(store.get_fixture(&first, "same").unwrap().unwrap(), json);
    assert_eq!(
        store.drop_collection(&first),
        Err(Error::CollectionNotEmpty)
    );
}

#[test]
fn recreate_has_new_identity_and_stale_handles_are_refused() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("data");
    let store = Store::create(&root).unwrap();
    let old = store.create_collection("orders").unwrap();
    store.drop_collection(&old).unwrap();
    drop(store);
    let store = Store::open(&root).unwrap();
    let new = store.create_collection("orders").unwrap();
    assert_ne!(old, new);
    assert_eq!(
        store.get_fixture(&old, "doc"),
        Err(Error::CollectionMissing)
    );
    assert_eq!(store.drop_collection(&old), Err(Error::CollectionMissing));
    assert_eq!(
        store.insert_fixture(&old, br#"{"_id":"doc"}"#),
        Err(Error::CollectionMissing)
    );
    assert_eq!(store.get_fixture(&new, "doc").unwrap(), None);
    store.insert_fixture(&new, br#"{"_id":"doc"}"#).unwrap();
}

#[test]
fn unsupported_or_invalid_manifest_never_opens_or_changes_engine_file() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("data");
    drop(Store::create(&root).unwrap());
    let original = fs::read(root.join("data.redb")).unwrap();
    for (manifest, expected) in [
        (
            br#"{"magic":"kiarni","version":1,"engine":"redb-4.5.0"}"#.as_slice(),
            Error::UnsupportedFormat,
        ),
        (
            br#"{"magic":"kiarni","version":0,"engine":"redb-4.5.0"}"#,
            Error::UnsupportedFormat,
        ),
        (
            br#"{"magic":"other","version":1,"engine":"redb-4.5.0"}"#,
            Error::UnsupportedFormat,
        ),
        (
            br#"{"magic":"kiarni","version":1,"engine":"redb-future"}"#,
            Error::UnsupportedFormat,
        ),
        (
            br#"{"magic":"kiarni","version":1,"version":1,"engine":"redb-4.5.0"}"#,
            Error::InvalidManifest,
        ),
        (
            br#"{"magic":"kiarni","version":1,"engine":"redb-4.5.0","unknown":true}"#,
            Error::InvalidManifest,
        ),
        (b"{}", Error::InvalidManifest),
        (b"{", Error::InvalidManifest),
        (b"[]", Error::InvalidManifest),
    ] {
        fs::write(root.join("format.json"), manifest).unwrap();
        assert_eq!(open_error(&root), expected);
        assert_eq!(fs::read(root.join("data.redb")).unwrap(), original);
        assert_eq!(fs::read(root.join("format.json")).unwrap(), manifest);
    }
    fs::write(root.join("format.json"), vec![b' '; 1025]).unwrap();
    assert_eq!(open_error(&root), Error::InvalidManifest);
    assert_eq!(fs::read(root.join("data.redb")).unwrap(), original);
    // Invalid engine bytes make ordering observable: unsupported format wins.
    fs::write(root.join("data.redb"), b"not a redb database").unwrap();
    fs::write(
        root.join("format.json"),
        br#"{"magic":"kiarni","version":99,"engine":"redb-4.5.0"}"#,
    )
    .unwrap();
    assert_eq!(open_error(&root), Error::UnsupportedFormat);
    assert_eq!(
        fs::read(root.join("data.redb")).unwrap(),
        b"not a redb database"
    );
}

#[test]
fn missing_or_partial_initialization_never_bootstraps_or_overwrites() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("data");
    assert_eq!(open_error(&root), Error::Io);
    assert!(!root.exists());
    fs::create_dir(&root).unwrap();
    assert_eq!(open_error(&root), Error::Io);
    assert!(Store::create(&root).is_err());
    assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
    fs::write(
        root.join("data.redb"),
        b"legacy or interrupted initialization",
    )
    .unwrap();
    assert_eq!(open_error(&root), Error::Io);
    assert_eq!(
        fs::read(root.join("data.redb")).unwrap(),
        b"legacy or interrupted initialization"
    );
    fs::write(
        root.join("format.json"),
        br#"{"magic":"kiarni","version":2,"engine":"redb-4.5.0"}"#,
    )
    .unwrap();
    fs::remove_file(root.join("data.redb")).unwrap();
    assert_eq!(open_error(&root), Error::Io);
    assert!(!root.join("data.redb").exists());
}

#[test]
fn validation_and_writer_lock_reject_without_implicit_resources() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("data");
    let store = Store::create(&root).unwrap();
    assert_eq!(open_error(&root), Error::Storage);
    for name in ["", "../orders", "orders.name", "á", &"a".repeat(129)] {
        assert_eq!(store.create_collection(name), Err(Error::InvalidName));
        assert_eq!(store.collection(name), Err(Error::InvalidName));
    }
    let collection = store.create_collection("valid_Name-1").unwrap();
    for json in [
        b"{}".as_slice(),
        b"[]",
        br#"{"_id":"a","_id":"a"}"#,
        br#"{"_id":null}"#,
    ] {
        assert_eq!(
            store.insert_fixture(&collection, json),
            Err(Error::InvalidDocument)
        );
    }
    store.drop_collection(&collection).unwrap();
}

#[cfg(unix)]
#[test]
fn created_files_are_private_and_final_symlinks_are_refused() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("data");
    drop(Store::create(&root).unwrap());
    for (path, expected) in [
        (&root, 0o700),
        (&root.join("format.json"), 0o600),
        (&root.join("data.redb"), 0o600),
    ] {
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            expected
        );
    }
    let alias = temp.path().join("alias");
    symlink(&root, &alias).unwrap();
    assert_eq!(open_error(&alias), Error::InvalidLayout);
    let moved = root.join("moved");
    fs::rename(root.join("data.redb"), &moved).unwrap();
    symlink(&moved, root.join("data.redb")).unwrap();
    assert_eq!(open_error(&root), Error::InvalidLayout);
}

#[cfg(unix)]
struct Worker(std::process::Child);
#[cfg(unix)]
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[cfg(unix)]
#[test]
fn committed_catalog_and_document_survive_killed_writer_and_lock_release() {
    use std::process::{Command, Stdio};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("data");
    drop(Store::create(&root).unwrap());
    let mut child = Worker(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "format_worker", "--ignored"])
            .env("KIARNI_TEST_FORMAT_ROOT", &root)
            .env("KIARNI_TEST_FORMAT_SIGNAL", temp.path().join("signal"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
    while !temp.path().join("signal").exists() {
        assert!(child.0.try_wait().unwrap().is_none());
        assert!(
            std::time::Instant::now() < deadline,
            "worker checkpoint deadline"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(open_error(&root), Error::Storage);
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    let original_manifest = fs::read(root.join("format.json")).unwrap();
    let dirty_database = fs::read(root.join("data.redb")).unwrap();
    fs::write(
        root.join("format.json"),
        br#"{"magic":"kiarni","version":1,"engine":"redb-4.5.0"}"#,
    )
    .unwrap();
    assert_eq!(open_error(&root), Error::UnsupportedFormat);
    assert_eq!(fs::read(root.join("data.redb")).unwrap(), dirty_database);
    fs::write(root.join("format.json"), original_manifest).unwrap();
    let store = Store::open(&root).unwrap();
    let collection = store.collection("committed").unwrap();
    assert_eq!(
        store.get_fixture(&collection, "doc").unwrap().unwrap(),
        br#"{"_id":"doc","n":1e10001}"#
    );
}

#[test]
fn concurrent_drop_and_insert_are_serialized_with_catalog_checks() {
    use std::sync::Barrier;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("data");
    let store = Store::create(&root).unwrap();
    let collection = store.create_collection("orders").unwrap();
    let barrier = Barrier::new(2);
    let (drop_result, insert_result) = std::thread::scope(|scope| {
        let dropping = scope.spawn(|| {
            barrier.wait();
            store.drop_collection(&collection)
        });
        let inserting = scope.spawn(|| {
            barrier.wait();
            store.insert_fixture(&collection, br#"{"_id":"doc"}"#)
        });
        (dropping.join().unwrap(), inserting.join().unwrap())
    });
    match (drop_result, insert_result) {
        (Ok(()), Err(Error::CollectionMissing)) => {
            assert_eq!(store.collection("orders"), Err(Error::CollectionMissing))
        }
        (Err(Error::CollectionNotEmpty), Ok(())) => {
            assert!(store.get_fixture(&collection, "doc").unwrap().is_some())
        }
        other => panic!("invalid transaction outcome: {other:?}"),
    }
}

#[cfg(unix)]
#[test]
#[ignore = "subprocess worker for persisted format recovery"]
fn format_worker() {
    let Some(root) = std::env::var_os("KIARNI_TEST_FORMAT_ROOT") else {
        return;
    };
    let store = Store::open(Path::new(&root)).unwrap();
    let collection = store.create_collection("committed").unwrap();
    store
        .insert_fixture(&collection, br#"{"_id":"doc","n":1e10001}"#)
        .unwrap();
    fs::write(
        std::env::var_os("KIARNI_TEST_FORMAT_SIGNAL").unwrap(),
        b"COMMITTED",
    )
    .unwrap();
    loop {
        std::thread::park();
    }
}
