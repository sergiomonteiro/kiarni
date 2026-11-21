use super::*;
use kiarni_storage::{IndexedMaintenance, Store};
use redb::{ReadableDatabase, TableDefinition};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

fn config(path: &Path) -> Config {
    kiarni_config::resolve(&[("--data-dir", path.to_str().unwrap())], &[], None).unwrap()
}
fn fixture(indexed: bool) -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("data");
    let root = Root::lock(&path).unwrap();
    let (store, _) = root.start(crate::now().unwrap()).unwrap();
    let collection = store.create_collection("items").unwrap();
    for n in 0..16 {
        store
            .insert_fixture(
                &collection,
                format!(
                    r#"{{"_id":"d{n:02}","status":"{}"}}"#,
                    if n == 0 { "wanted" } else { "bulk" }
                )
                .as_bytes(),
            )
            .unwrap();
    }
    if indexed {
        store.create_index(&collection, "status").unwrap();
    }
    temp
}
fn auth_bytes(path: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut paths = vec![path.join("bootstrap.json")];
    for dir in [path.join("auth"), path.with_file_name("data.credentials")] {
        paths.extend(
            fs::read_dir(dir)
                .unwrap()
                .map(|entry| entry.unwrap().path()),
        );
    }
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let bytes = fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect()
}
fn corrupt(path: &Path) {
    let db = redb::Database::open(path.join("store/data.redb")).unwrap();
    let write = db.begin_write().unwrap();
    write
        .open_table(TableDefinition::<(u64, &str, &str), ()>::new(
            "secondary_entries",
        ))
        .unwrap()
        .remove((1, "wanted", "d00"))
        .unwrap();
    write
        .open_table(TableDefinition::<(u64, &str), u64>::new("secondary_counts"))
        .unwrap()
        .remove((1, "wanted"))
        .unwrap();
    write.commit().unwrap();
    db.close().unwrap();
}
fn field(path: &Path) -> Option<String> {
    let store = Store::open(&path.join("store")).unwrap();
    store
        .index_field(&store.collection("items").unwrap())
        .unwrap()
}

#[test]
fn repair_uses_restricted_open_preserves_revoked_auth_and_never_repairs_schema() {
    let temp = fixture(true);
    let path = temp.path().join("data");
    kiarni_auth::activation::Store::open(&path, None)
        .unwrap()
        .revoke_all()
        .unwrap();
    let before_auth = auth_bytes(&path);
    corrupt(&path);
    assert!(Store::open(&path.join("store")).is_err());
    assert!(manage_index(config(&path), IndexCommand::Audit).is_err());
    assert!(
        manage_index(
            config(&path),
            IndexCommand::Create {
                collection: "items",
                field: "other"
            }
        )
        .is_err()
    );
    manage_index(
        config(&path),
        IndexCommand::Rebuild {
            collection: "items",
        },
    )
    .unwrap();
    assert_eq!(
        manage_index(config(&path), IndexCommand::Audit)
            .unwrap()
            .unwrap()
            .documents,
        16
    );
    assert_eq!(field(&path).as_deref(), Some("status"));
    corrupt(&path);
    manage_index(
        config(&path),
        IndexCommand::Drop {
            collection: "items",
        },
    )
    .unwrap();
    assert_eq!(field(&path), None);
    assert!(auth_bytes(&path) == before_auth);
    let db = redb::Database::open(path.join("store/data.redb")).unwrap();
    let write = db.begin_write().unwrap();
    write
        .delete_table(TableDefinition::<(u64, &str, &str), ()>::new(
            "secondary_entries",
        ))
        .unwrap();
    write.commit().unwrap();
    db.close().unwrap();
    assert!(
        manage_index(
            config(&path),
            IndexCommand::Rebuild {
                collection: "items"
            }
        )
        .is_err()
    );
    assert!(manage_index(config(&path), IndexCommand::Audit).is_err());
    let db = redb::Database::open(path.join("store/data.redb")).unwrap();
    let read = db.begin_read().unwrap();
    assert!(
        read.open_table(TableDefinition::<(u64, &str, &str), ()>::new(
            "secondary_entries"
        ))
        .is_err()
    );
    assert_eq!(
        read.open_table(TableDefinition::<(u64, &str), &[u8]>::new("documents"))
            .unwrap()
            .get((1, "d00"))
            .unwrap()
            .unwrap()
            .value(),
        br#"{"_id":"d00","status":"wanted"}"#
    );
    assert!(auth_bytes(&path) == before_auth);
}

#[test]
fn bounded_maintenance_aborts_without_losing_documents_or_previous_index() {
    let temp = fixture(false);
    let path = temp.path().join("data");
    let before_auth = auth_bytes(&path);
    {
        let store = Store::open(&path.join("store")).unwrap();
        let collection = store.collection("items").unwrap();
        for n in 0..9 {
            store
                .insert_fixture(
                    &collection,
                    format!(
                        r#"{{"_id":"large{n}","status":"bulk","payload":"{}"}}"#,
                        "x".repeat(1_000_000)
                    )
                    .as_bytes(),
                )
                .unwrap();
        }
    }
    assert_eq!(
        manage_index(
            config(&path),
            IndexCommand::Create {
                collection: "items",
                field: "status"
            }
        )
        .unwrap_err()
        .to_string(),
        "index_maintenance_limit"
    );
    assert_eq!(field(&path), None);
    {
        let store = Store::open(&path.join("store")).unwrap();
        store
            .delete_document(&store.collection("items").unwrap(), "large8")
            .unwrap();
    }
    manage_index(
        config(&path),
        IndexCommand::Create {
            collection: "items",
            field: "status",
        },
    )
    .unwrap();
    {
        let store = Store::open(&path.join("store")).unwrap();
        store
            .insert_fixture(
                &store.collection("items").unwrap(),
                format!(
                    r#"{{"_id":"large8","status":"bulk","payload":"{}"}}"#,
                    "x".repeat(1_000_000)
                )
                .as_bytes(),
            )
            .unwrap();
    }
    for command in [
        IndexCommand::Rebuild {
            collection: "items",
        },
        IndexCommand::Drop {
            collection: "items",
        },
    ] {
        assert_eq!(
            manage_index(config(&path), command)
                .unwrap_err()
                .to_string(),
            "index_maintenance_limit"
        );
        assert_eq!(field(&path).as_deref(), Some("status"));
        assert_eq!(
            manage_index(config(&path), IndexCommand::Audit)
                .unwrap()
                .unwrap()
                .documents,
            25
        );
    }
    assert!(auth_bytes(&path) == before_auth);
}

#[test]
fn incomplete_installation_is_never_bootstrapped_by_index_administration() {
    for phase in ["initializing_store", "store_ready", "initializing_auth"] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("data");
        let root = Root::lock(&path).unwrap();
        let record = format!(r#"{{"version":2,"store_format":2,"phase":"{phase}"}}"#);
        fs::write(path.join("bootstrap.json"), record.as_bytes()).unwrap();
        fs::set_permissions(
            path.join("bootstrap.json"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        drop(root);
        for command in [
            IndexCommand::Audit,
            IndexCommand::Create {
                collection: "items",
                field: "status",
            },
            IndexCommand::Rebuild {
                collection: "items",
            },
            IndexCommand::Drop {
                collection: "items",
            },
        ] {
            assert_eq!(
                manage_index(config(&path), command)
                    .unwrap_err()
                    .to_string(),
                "installation_not_initialized"
            );
        }
        assert_eq!(
            fs::read(path.join("bootstrap.json")).unwrap(),
            record.as_bytes()
        );
        assert!(!path.join("store").exists());
        assert!(!path.join("auth").exists());
        assert!(!temp.path().join("data.credentials").exists());
    }
}

#[test]
fn precommit_failure_is_unchanged_and_postcommit_response_failure_is_uncertain() {
    let temp = fixture(false);
    let path = temp.path().join("data");
    let before_auth = auth_bytes(&path);
    for wanted in [Point::Opened, Point::Committed] {
        let error = run(
            config(&path),
            IndexCommand::Create {
                collection: "items",
                field: "status",
            },
            |point| {
                if point == wanted {
                    return Err("injected_failure".into());
                }
                Ok(())
            },
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            if wanted == Point::Opened {
                "injected_failure"
            } else {
                "commit_uncertain"
            }
        );
        assert_eq!(field(&path).is_some(), wanted == Point::Committed);
        manage_index(config(&path), IndexCommand::Audit).unwrap();
    }
    assert!(auth_bytes(&path) == before_auth);
}

struct Worker(Child);
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "subprocess worker for offline index administration"]
fn worker() {
    let Some(path) = std::env::var_os("KIARNI_TEST_INDEX_ROOT") else {
        return;
    };
    let path = PathBuf::from(path);
    let command = match std::env::var("KIARNI_TEST_INDEX_OPERATION")
        .unwrap()
        .as_str()
    {
        "create" => IndexCommand::Create {
            collection: "items",
            field: "status",
        },
        "rebuild" => IndexCommand::Rebuild {
            collection: "items",
        },
        "drop" => IndexCommand::Drop {
            collection: "items",
        },
        _ => IndexCommand::Audit,
    };
    let wanted = if std::env::var("KIARNI_TEST_INDEX_POINT").unwrap() == "opened" {
        Point::Opened
    } else {
        Point::Committed
    };
    run(config(&path), command, |point| {
        if point == wanted {
            fs::write(
                std::env::var_os("KIARNI_TEST_INDEX_SIGNAL").unwrap(),
                b"ready",
            )?;
            loop {
                thread::park();
            }
        }
        Ok(())
    })
    .unwrap();
}

#[test]
fn root_lock_survives_until_close_and_kill_recovery_preserves_each_commit_boundary() {
    for operation in ["create", "rebuild", "drop", "audit"] {
        for point in ["opened", "committed"] {
            if operation == "audit" && point == "committed" {
                continue;
            }
            let temp = fixture(operation != "create");
            let path = temp.path().join("data");
            let before_auth = auth_bytes(&path);
            if operation == "rebuild" {
                corrupt(&path);
            }
            let signal = temp.path().join("signal");
            let mut child = Worker(
                Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", "indexes::tests::worker", "--ignored"])
                    .env("KIARNI_TEST_INDEX_ROOT", &path)
                    .env("KIARNI_TEST_INDEX_OPERATION", operation)
                    .env("KIARNI_TEST_INDEX_POINT", point)
                    .env("KIARNI_TEST_INDEX_SIGNAL", &signal)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap(),
            );
            let deadline = Instant::now() + Duration::from_secs(8);
            while !signal.exists() {
                assert!(
                    child.0.try_wait().unwrap().is_none(),
                    "worker exited: {operation}/{point}"
                );
                assert!(
                    Instant::now() < deadline,
                    "checkpoint deadline: {operation}/{point}"
                );
                thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(
                Root::lock_existing(&path).err().unwrap().to_string(),
                "data_directory_locked"
            );
            child.0.kill().unwrap();
            child.0.wait().unwrap();
            if operation == "rebuild" && point == "opened" {
                assert!(manage_index(config(&path), IndexCommand::Audit).is_err());
                manage_index(
                    config(&path),
                    IndexCommand::Rebuild {
                        collection: "items",
                    },
                )
                .unwrap();
            }
            assert_eq!(
                manage_index(config(&path), IndexCommand::Audit)
                    .unwrap()
                    .unwrap()
                    .documents,
                16
            );
            let expected = match operation {
                "create" => point == "committed",
                "drop" => point == "opened",
                _ => true,
            };
            let maintenance = IndexedMaintenance::open(&path.join("store")).unwrap();
            assert_eq!(
                maintenance
                    .index_field(&maintenance.collection("items").unwrap())
                    .unwrap()
                    .is_some(),
                expected
            );
            assert!(auth_bytes(&path) == before_auth);
        }
    }
}
