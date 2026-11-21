#![cfg(unix)]

use kiarni_server::initialize_indexed_installation;
use kiarni_storage::{IndexedStore, Store};
use std::{fs, os::unix::fs::PermissionsExt, path::Path};

fn config(path: &Path) -> kiarni_config::Config {
    kiarni_config::resolve(&[("--data-dir", path.to_str().unwrap())], &[], None).unwrap()
}

#[test]
fn explicit_initialization_delivers_once_and_releases_the_engine_and_root() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("indexed");
    let first = initialize_indexed_installation(config(&path)).unwrap();
    assert_eq!(first.epoch, 1);
    let token = first.token_file.unwrap();
    assert_eq!(
        fs::metadata(&token).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let credential = fs::read(&token).unwrap();
    assert!(!credential.is_empty());
    assert!(
        IndexedStore::open(&path.join("store"))
            .unwrap()
            .audit()
            .is_ok()
    );
    assert!(Store::open(&path.join("store")).is_err());
    let second = initialize_indexed_installation(config(&path)).unwrap();
    assert_eq!(second.epoch, first.epoch);
    assert!(second.token_file.is_none());
    assert_eq!(fs::read(token).unwrap(), credential);
    let phase: serde_json::Value =
        serde_json::from_slice(&fs::read(path.join("bootstrap.json")).unwrap()).unwrap();
    assert_eq!(phase["phase"], "initialized");
    assert_eq!(phase["version"], 2);
    assert_eq!(phase["store_format"], 2);
}

#[test]
fn legacy_unrecognized_and_insecure_roots_do_not_receive_credentials() {
    let temp = tempfile::tempdir().unwrap();
    for case in ["legacy", "unrecognized", "insecure"] {
        let path = temp.path().join(case);
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        if case == "legacy" {
            drop(Store::create(&path.join("store")).unwrap());
            fs::write(
                path.join("bootstrap.json"),
                br#"{"version":1,"phase":"store_ready"}"#,
            )
            .unwrap();
            fs::set_permissions(
                path.join("bootstrap.json"),
                fs::Permissions::from_mode(0o600),
            )
            .unwrap();
        } else if case == "unrecognized" {
            fs::write(path.join("sentinel"), b"keep").unwrap();
        } else {
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let before = (case == "legacy").then(|| fs::read(path.join("store/data.redb")).unwrap());
        assert!(initialize_indexed_installation(config(&path)).is_err());
        assert!(!path.join("auth").exists());
        assert!(!temp.path().join(format!("{case}.credentials")).exists());
        if let Some(before) = before {
            assert_eq!(fs::read(path.join("store/data.redb")).unwrap(), before);
        }
        if case == "unrecognized" {
            assert_eq!(fs::read(path.join("sentinel")).unwrap(), b"keep");
        }
    }
}
