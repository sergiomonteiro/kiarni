#![cfg(unix)]
use kiarni_auth_spike::{
    Authority, Error, Keyset, MAX_LIFETIME,
    activation::{Checkpoint, Store},
};
use std::{
    fs,
    io::{BufRead, BufReader},
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};
use zeroize::Zeroizing;

fn root() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    root
}
fn header(path: &Path) -> Zeroizing<Vec<u8>> {
    let token = Zeroizing::new(fs::read(path).unwrap());
    let mut header = Zeroizing::new(b"Bearer ".to_vec());
    header.extend_from_slice(&token);
    header
}
fn private_write(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}
fn cli(mode: &str, data: &Path) -> serde_json::Value {
    let output = Command::new(env!("CARGO_BIN_EXE_auth-bootstrap-spike"))
        .arg(mode)
        .arg("--data-dir")
        .arg(data)
        .output()
        .unwrap();
    assert!(output.status.success(), "CLI failed with safe error code");
    assert!(output.stderr.is_empty());
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn minimal_serve_bootstraps_sibling_delivery_without_reissuing_on_restart_or_expiry() {
    let root = root();
    let data = root.path().join("data");
    let metadata = cli("serve", &data);
    assert_eq!(metadata["created"], true);
    assert_eq!(metadata["epoch"], 1);
    let path = PathBuf::from(metadata["token_file"].as_str().unwrap());
    assert_eq!(path.parent().unwrap(), root.path().join("data.credentials"));
    let header = header(&path);
    let store = Store::open(&data, None).unwrap();
    let startup = store.bootstrap_or_load(1000).unwrap();
    assert!(!startup.created);
    assert!(startup.token_file.is_none());
    assert!(
        Authority::new(startup.keyset, 1000)
            .authenticate(&[&header], 1000)
            .is_ok()
    );
    let expired = store.bootstrap_or_load(1000 + MAX_LIFETIME).unwrap();
    assert!(!expired.created);
    assert_eq!(
        Authority::new(expired.keyset, 1000 + MAX_LIFETIME)
            .authenticate(&[&header], 1000 + MAX_LIFETIME)
            .err(),
        Some(Error::Unauthorized)
    );
    assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
    for entry in fs::read_dir(data.join("auth")).unwrap() {
        let entry = entry.unwrap();
        let bytes = fs::read(entry.path()).unwrap();
        assert!(
            !bytes
                .windows(header.len() - 7)
                .any(|bytes| bytes == &header[7..])
        );
        assert_eq!(
            entry.metadata().unwrap().permissions().mode() & 0o7777,
            0o600
        );
    }
}

#[test]
fn completed_revocation_persists_across_a_fresh_process_and_local_rotation_restores_access() {
    let root = root();
    let data = root.path().join("data");
    let bootstrap = cli("serve", &data);
    let old = header(Path::new(bootstrap["token_file"].as_str().unwrap()));
    assert_eq!(cli("revoke", &data)["epoch"], 2);
    let reopened = cli("serve", &data);
    assert_eq!(reopened["epoch"], 2);
    assert_eq!(reopened["created"], false);
    assert!(reopened["token_file"].is_null());
    let store = Store::open(&data, None).unwrap();
    let authority = Authority::new(store.bootstrap_or_load(1000).unwrap().keyset, 1000);
    assert_eq!(
        authority.authenticate(&[&old], 1000).err(),
        Some(Error::Unauthorized)
    );
    let rotated = cli("rotate", &data);
    assert_eq!(rotated["epoch"], 3);
    store.refresh(&authority, 1000).unwrap();
    let new = header(Path::new(rotated["token_file"].as_str().unwrap()));
    assert!(authority.authenticate(&[&new], 1000).is_ok());
    assert_eq!(
        authority.authenticate(&[&old], 1000).err(),
        Some(Error::Unauthorized)
    );
}

#[test]
fn errors_before_and_after_intent_have_distinct_recovery_outcomes() {
    for (fault, committed) in [
        (Checkpoint::BeforeIntentRename, false),
        (Checkpoint::IntentRenamed, true),
        (Checkpoint::BeforeActiveRename, true),
    ] {
        let root = root();
        let data = root.path().join("data");
        let store = Store::open(&data, None).unwrap();
        let first = store.bootstrap_or_load(1000).unwrap();
        let old = header(first.token_file.as_ref().unwrap());
        let result = store.update_with_hook(None, &mut |point| {
            if point == fault {
                Err(Error::FileAccess)
            } else {
                Ok(())
            }
        });
        assert_eq!(
            result.err(),
            Some(if committed {
                Error::CommitUncertain
            } else {
                Error::FileAccess
            })
        );
        let restarted = Store::open(&data, None)
            .unwrap()
            .bootstrap_or_load(1000)
            .unwrap();
        assert_eq!(restarted.keyset.epoch(), if committed { 2 } else { 1 });
        assert_eq!(
            Authority::new(restarted.keyset, 1000)
                .authenticate(&[&old], 1000)
                .is_ok(),
            !committed
        );
    }
}

#[test]
fn orphan_snapshots_are_not_selected_and_corruption_never_regenerates_auth() {
    let root = root();
    let data = root.path().join("data");
    let store = Store::open(&data, None).unwrap();
    let first = store.bootstrap_or_load(1000).unwrap();
    let (unrelated, _) = Keyset::bootstrap(1000, 100).unwrap();
    private_write(
        &data.join("auth/auth-999999999.json"),
        &unrelated.to_json().unwrap(),
    );
    assert_eq!(store.bootstrap_or_load(1000).unwrap().keyset.epoch(), 1);
    private_write(&data.join("auth/intent.json"), b"{");
    assert_eq!(
        store.bootstrap_or_load(1000).err(),
        Some(Error::InvalidConfig)
    );
    assert!(first.token_file.unwrap().is_file());
    assert_eq!(
        fs::read_dir(root.path().join("data.credentials"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn missing_selection_after_initialization_fails_closed_and_reload_does_not_fall_back() {
    let root = root();
    let data = root.path().join("data");
    let store = Store::open(&data, None).unwrap();
    let first = store.bootstrap_or_load(1000).unwrap();
    let old = header(first.token_file.as_ref().unwrap());
    let authority = Authority::new(first.keyset, 1000);
    fs::remove_file(data.join("auth/intent.json")).unwrap();
    assert_eq!(
        store.bootstrap_or_load(1000).err(),
        Some(Error::BootstrapIncomplete)
    );
    fs::remove_file(data.join("auth/active.json")).unwrap();
    assert_eq!(
        store.bootstrap_or_load(1000).err(),
        Some(Error::BootstrapIncomplete)
    );
    assert_eq!(
        store.refresh(&authority, 1000),
        Err(Error::BootstrapIncomplete)
    );
    assert_eq!(
        authority.authenticate(&[&old], 1000).err(),
        Some(Error::Unavailable)
    );
}

#[test]
fn missing_active_is_repaired_from_the_explicit_revocation_intent() {
    let root = root();
    let data = root.path().join("data");
    let store = Store::open(&data, None).unwrap();
    let first = store.bootstrap_or_load(1000).unwrap();
    let old = header(first.token_file.as_ref().unwrap());
    store.revoke_all().unwrap();
    fs::remove_file(data.join("auth/active.json")).unwrap();
    let startup = store.bootstrap_or_load(1000).unwrap();
    assert!(startup.recovered);
    assert!(!startup.created);
    assert!(startup.token_file.is_none());
    assert_eq!(startup.keyset.epoch(), 2);
    assert_eq!(
        Authority::new(startup.keyset, 1000)
            .authenticate(&[&old], 1000)
            .err(),
        Some(Error::Unauthorized)
    );
}

#[test]
fn snapshot_digest_conflicting_active_and_insecure_lock_are_rejected() {
    for fault in ["snapshot", "active", "lock"] {
        let root = root();
        let data = root.path().join("data");
        let store = Store::open(&data, None).unwrap();
        store.bootstrap_or_load(1000).unwrap();
        let auth = data.join("auth");
        let expected = match fault {
            "snapshot" => {
                let pointer: serde_json::Value =
                    serde_json::from_slice(&fs::read(auth.join("active.json")).unwrap()).unwrap();
                private_write(
                    &auth.join(format!(
                        "state-{}.json",
                        pointer["sha256"].as_str().unwrap()
                    )),
                    br#"{"version":1,"epoch":1,"keys":[]}"#,
                );
                Error::InvalidConfig
            }
            "active" => {
                let mut pointer: serde_json::Value =
                    serde_json::from_slice(&fs::read(auth.join("active.json")).unwrap()).unwrap();
                pointer["epoch"] = 999.into();
                private_write(
                    &auth.join("active.json"),
                    &serde_json::to_vec(&pointer).unwrap(),
                );
                Error::EpochRollback
            }
            _ => {
                fs::set_permissions(auth.join("writer.lock"), fs::Permissions::from_mode(0o644))
                    .unwrap();
                Error::InsecurePath
            }
        };
        assert_eq!(store.bootstrap_or_load(1000).err(), Some(expected));
    }
}

#[test]
fn layout_rejects_nested_delivery_symlinks_and_insecure_existing_directories() {
    let root = root();
    let data = root.path().join("data");
    assert_eq!(
        Store::open(&data, Some(&data.join("credentials")))
            .err()
            .map(|e| e.code()),
        Some("insecure_auth_path")
    );
    let insecure = root.path().join("insecure");
    fs::create_dir(&insecure).unwrap();
    fs::set_permissions(&insecure, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        Store::open(&insecure, None).err().map(|e| e.code()),
        Some("insecure_auth_path")
    );
    assert_eq!(
        fs::metadata(&insecure).unwrap().permissions().mode() & 0o777,
        0o755
    );
    let link = root.path().join("link");
    symlink(&data, &link).unwrap();
    assert!(Store::open(&link, None).is_err());
}

struct Worker(Child);
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn worker(mode: &str, point: Checkpoint, data: &Path) -> Worker {
    let mut worker = Worker(
        Command::new(env!("CARGO_BIN_EXE_auth-bootstrap-spike"))
            .arg(mode)
            .arg("--data-dir")
            .arg(data)
            .arg("--stop-at")
            .arg(format!("{point:?}"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let stdout = worker.0.stdout.take().unwrap();
    let (tx, rx) = mpsc::sync_channel(1);
    let reader = thread::spawn(move || {
        let mut line = String::new();
        BufReader::new(stdout).read_line(&mut line).unwrap();
        let _ = tx.send(line);
    });
    assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), "READY\n");
    reader.join().unwrap();
    worker
}

#[test]
fn kill_during_initial_bootstrap_recovers_only_the_recorded_decision() {
    for point in [
        Checkpoint::BootstrapMarked,
        Checkpoint::TokenDelivered,
        Checkpoint::SnapshotDurable,
        Checkpoint::BeforeIntentRename,
        Checkpoint::IntentRenamed,
        Checkpoint::IntentDurable,
        Checkpoint::InitializedDurable,
        Checkpoint::BeforeActiveRename,
        Checkpoint::ActiveRenamed,
        Checkpoint::ActiveDurable,
        Checkpoint::Complete,
    ] {
        let root = root();
        let data = root.path().join("data");
        let mut child = worker("serve", point, &data);
        assert_eq!(
            Store::open(&data, None)
                .unwrap()
                .bootstrap_or_load(1000)
                .err(),
            Some(Error::Busy)
        );
        child.0.kill().unwrap();
        child.0.wait().unwrap();
        let restarted = Store::open(&data, None)
            .unwrap()
            .bootstrap_or_load(1000)
            .unwrap();
        assert_eq!(restarted.keyset.epoch(), 1);
        let authority = Authority::new(restarted.keyset, 1000);
        let delivery = root.path().join("data.credentials");
        let accepted = fs::read_dir(delivery)
            .unwrap()
            .filter(|entry| {
                let header = header(&entry.as_ref().unwrap().path());
                authority.authenticate(&[&header], 1000).is_ok()
            })
            .count();
        assert_eq!(accepted, 1);
    }
}

#[test]
fn kill_during_rotation_or_revocation_never_reactivates_a_committed_old_token() {
    for mode in ["rotate", "revoke"] {
        for (point, committed) in [
            (Checkpoint::SnapshotDurable, false),
            (Checkpoint::BeforeIntentRename, false),
            (Checkpoint::IntentRenamed, true),
            (Checkpoint::IntentDurable, true),
            (Checkpoint::BeforeActiveRename, true),
            (Checkpoint::ActiveRenamed, true),
            (Checkpoint::ActiveDurable, true),
            (Checkpoint::Complete, true),
        ] {
            let root = root();
            let data = root.path().join("data");
            let store = Store::open(&data, None).unwrap();
            let first = store.bootstrap_or_load(1000).unwrap();
            let old = header(first.token_file.as_ref().unwrap());
            let mut child = worker(mode, point, &data);
            child.0.kill().unwrap();
            child.0.wait().unwrap();
            let restarted = Store::open(&data, None)
                .unwrap()
                .bootstrap_or_load(1000)
                .unwrap();
            assert_eq!(restarted.keyset.epoch(), if committed { 2 } else { 1 });
            assert_eq!(
                Authority::new(restarted.keyset, 1000)
                    .authenticate(&[&old], 1000)
                    .is_ok(),
                !committed
            );
        }
    }
}
