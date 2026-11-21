use super::*;
use lifecycle::{Action, audit, maintain};
use mutations::{Mutation, apply};

fn insert(db: &Database, collection: &Collection) {
    let input = DocumentInput::parse(
        br#"{"_id":"a","status":"old","n":9007199254740993}"#,
        DEFAULT_MAX_DOCUMENT_BYTES,
    )
    .unwrap()
    .prepare_insert()
    .unwrap();
    apply(
        None,
        db,
        collection,
        input.id(),
        Mutation::Insert(&input),
        |_| Ok(()),
    )
    .unwrap();
}
fn setup(mode: &str) -> (tempfile::TempDir, Database) {
    let temp = tempfile::tempdir().unwrap();
    let db = create(&temp.path().join("store")).unwrap();
    if mode != "create" {
        let collection = create_collection(None, &db, "tasks", |_| Ok(())).unwrap();
        maintain(None, &db, &collection, Action::Build("status"), |_| Ok(())).unwrap();
        if !matches!(mode, "drop" | "insert") {
            insert(&db, &collection);
        }
    }
    (temp, db)
}
fn points() -> [Point; 5] {
    [
        Point::Catalog,
        Point::Statistics,
        Point::Secondary,
        Point::BeforeCommit,
        Point::Committed,
    ]
}
fn present(db: &Database) -> bool {
    db.begin_read()
        .unwrap()
        .open_table(COLLECTIONS)
        .unwrap()
        .get("tasks")
        .unwrap()
        .is_some()
}
fn next(db: &Database) -> u64 {
    db.begin_read()
        .unwrap()
        .open_table(METADATA)
        .unwrap()
        .get("next_collection_id")
        .unwrap()
        .unwrap()
        .value()
}

#[test]
fn recreated_collection_has_new_identity_and_rejects_old_handles() {
    let (temp, db) = setup("insert");
    let old = Collection {
        name: "tasks".into(),
        id: 1,
    };
    insert(&db, &old);
    apply(None, &db, &old, "a", Mutation::Delete, |_| Ok(())).unwrap();
    let snapshot = db.begin_read().unwrap();
    drop_collection(None, &db, &old, |_| Ok(())).unwrap();
    let new = create_collection(None, &db, "tasks", |_| Ok(())).unwrap();
    assert_eq!(new.id, 2);
    assert_eq!(next(&db), 3);
    let read = db.begin_read().unwrap();
    assert!(read.open_table(CATALOG).unwrap().get(1).unwrap().is_none());
    assert!(read.open_table(CATALOG).unwrap().get(2).unwrap().is_none());
    assert!(read.open_table(STATS).unwrap().get(1).unwrap().is_none());
    assert_eq!(read.open_table(ENTRIES).unwrap().len().unwrap(), 0);
    assert_eq!(read.open_table(COUNTS).unwrap().len().unwrap(), 0);
    assert_eq!(
        snapshot
            .open_table(COLLECTIONS)
            .unwrap()
            .get("tasks")
            .unwrap()
            .unwrap()
            .value(),
        1
    );
    assert!(
        snapshot
            .open_table(CATALOG)
            .unwrap()
            .get(1)
            .unwrap()
            .is_some()
    );
    assert_eq!(
        apply(None, &db, &old, "a", Mutation::Delete, |_| Ok(())),
        Err(Failure::Aborted(Error::CollectionMissing))
    );
    assert_eq!(
        drop_collection(None, &db, &old, |_| Ok(())),
        Err(Failure::Aborted(Error::CollectionMissing))
    );
    assert_eq!(
        maintain(None, &db, &old, Action::Rebuild, |_| Ok(())),
        Err(Failure::Aborted(Error::CollectionMissing))
    );
    assert_eq!(audit(&db, &old), Err(Error::CollectionMissing));
    insert(&db, &new);
    audit(&db, &new).unwrap();
    maintain(None, &db, &new, Action::Build("status"), |_| Ok(())).unwrap();
    audit(&db, &new).unwrap();
    drop(read);
    drop(snapshot);
    drop(db);
    let db = open(&temp.path().join("store")).unwrap();
    audit(&db, &new).unwrap();
    assert_eq!(next(&db), 3);
}

#[test]
fn errors_do_not_delete_documents_or_adopt_orphaned_data() {
    for case in 0..7 {
        let (_temp, db) = setup(if case == 0 { "replace" } else { "drop" });
        let old = Collection {
            name: "tasks".into(),
            id: 1,
        };
        if case == 0 {
            assert_eq!(
                drop_collection(None, &db, &old, |_| Ok(())),
                Err(Failure::Aborted(Error::CollectionNotEmpty))
            );
            audit(&db, &old).unwrap();
        } else {
            let write = db.begin_write().unwrap();
            match case {
                1 => {
                    write
                        .open_table(STATS)
                        .unwrap()
                        .insert(1, ("tasks", 1, 0))
                        .unwrap();
                }
                2 => {
                    write
                        .open_table(COUNTS)
                        .unwrap()
                        .insert((1, "orphan"), 1)
                        .unwrap();
                }
                3 => {
                    write
                        .open_table(ENTRIES)
                        .unwrap()
                        .insert((1, "orphan", "a"), ())
                        .unwrap();
                }
                4 => {
                    write
                        .open_table(CATALOG)
                        .unwrap()
                        .insert(1, b"invalid".as_slice())
                        .unwrap();
                }
                5 => {
                    write.delete_table(STATS).unwrap();
                }
                6 => {
                    write
                        .open_table(METADATA)
                        .unwrap()
                        .insert("next_collection_id", 1)
                        .unwrap();
                }
                _ => unreachable!(),
            }
            write.commit().unwrap();
            assert!(drop_collection(None, &db, &old, |_| Ok(())).is_err());
        }
        assert!(present(&db));
        assert!(create_collection(None, &db, "tasks", |_| Ok(())).is_err());
    }
    let (_temp, db) = setup("create");
    assert!(matches!(
        create_collection(None, &db, "", |_| Ok(())),
        Err(Failure::Aborted(Error::InvalidName))
    ));
    let write = db.begin_write().unwrap();
    write
        .open_table(METADATA)
        .unwrap()
        .insert("next_collection_id", u64::MAX)
        .unwrap();
    write.commit().unwrap();
    assert!(matches!(
        create_collection(None, &db, "tasks", |_| Ok(())),
        Err(Failure::Aborted(Error::CollectionIdsExhausted))
    ));
    assert!(!present(&db));
    assert_eq!(next(&db), u64::MAX);
    let write = db.begin_write().unwrap();
    write
        .open_table(METADATA)
        .unwrap()
        .insert("next_collection_id", 1)
        .unwrap();
    write
        .open_table(COUNTS)
        .unwrap()
        .insert((1, "orphan"), 1)
        .unwrap();
    write.commit().unwrap();
    assert!(create_collection(None, &db, "tasks", |_| Ok(())).is_err());
    assert_eq!(next(&db), 1);
    assert!(!present(&db));
}

#[test]
fn collection_failures_preserve_counter_and_catalog_atomically() {
    for mode in ["create", "drop"] {
        for failed in points() {
            let (_temp, db) = setup(mode);
            let result = if mode == "create" {
                create_collection(None, &db, "tasks", |p| {
                    if p == failed {
                        Err(Error::Storage)
                    } else {
                        Ok(())
                    }
                })
                .map(|_| ())
            } else {
                drop_collection(
                    None,
                    &db,
                    &Collection {
                        name: "tasks".into(),
                        id: 1,
                    },
                    |p| {
                        if p == failed {
                            Err(Error::Storage)
                        } else {
                            Ok(())
                        }
                    },
                )
            };
            assert_eq!(
                result,
                Err(if failed == Point::Committed {
                    Failure::CommitUncertain
                } else {
                    Failure::Aborted(Error::Storage)
                })
            );
            let committed = failed == Point::Committed;
            assert_eq!(
                present(&db),
                if mode == "create" {
                    committed
                } else {
                    !committed
                }
            );
            assert_eq!(
                next(&db),
                if mode == "create" && !committed { 1 } else { 2 }
            );
            validate(&db).unwrap();
            if present(&db) {
                audit(
                    &db,
                    &Collection {
                        name: "tasks".into(),
                        id: 1,
                    },
                )
                .unwrap();
            } else {
                assert!(
                    db.begin_read()
                        .unwrap()
                        .open_table(STATS)
                        .unwrap()
                        .get(1)
                        .unwrap()
                        .is_none()
                );
                assert!(
                    db.begin_read()
                        .unwrap()
                        .open_table(CATALOG)
                        .unwrap()
                        .get(1)
                        .unwrap()
                        .is_none()
                );
            }
        }
    }
}

#[test]
fn concurrent_duplicate_creation_allocates_only_one_generation() {
    let (_temp, db) = setup("create");
    let outcomes = std::thread::scope(|scope| {
        let a = scope.spawn(|| create_collection(None, &db, "tasks", |_| Ok(())));
        let b = scope.spawn(|| create_collection(None, &db, "tasks", |_| Ok(())));
        [a.join().unwrap(), b.join().unwrap()]
    });
    assert_eq!(outcomes.iter().filter(|v| v.is_ok()).count(), 1);
    assert_eq!(
        outcomes
            .iter()
            .filter(|v| matches!(v, Err(Failure::Aborted(Error::CollectionExists))))
            .count(),
        1
    );
    assert_eq!(next(&db), 2);
    validate(&db).unwrap();
}

fn pause(point: impl std::fmt::Debug, requested: &str) -> Result<(), Error> {
    use std::io::{Read, Write};
    if format!("{point:?}") == requested {
        println!("GENERATION_CHECKPOINT");
        std::io::stdout().flush().unwrap();
        let _ = std::io::stdin().read_exact(&mut [0u8; 1]);
        std::process::exit(90);
    }
    Ok(())
}

#[test]
#[ignore = "subprocess fixture for candidate collection and mutation crashes"]
fn crash_worker() {
    let root = std::env::var_os("KIARNI_V2_GENERATION_ROOT").unwrap();
    let point = std::env::var("KIARNI_V2_GENERATION_POINT").unwrap();
    let mode = std::env::var("KIARNI_V2_GENERATION_MODE").unwrap();
    let db = open(Path::new(&root)).unwrap();
    let collection = Collection {
        name: "tasks".into(),
        id: 1,
    };
    match mode.as_str() {
        "create" => {
            create_collection(None, &db, "tasks", |p| pause(p, &point)).unwrap();
        }
        "drop" => {
            drop_collection(None, &db, &collection, |p| pause(p, &point)).unwrap();
        }
        _ => {
            let input = DocumentInput::parse(
                br#"{"_id":"a","status":"new","n":9007199254740993}"#,
                DEFAULT_MAX_DOCUMENT_BYTES,
            )
            .unwrap()
            .prepare_insert()
            .unwrap();
            let mutation = match mode.as_str() {
                "insert" => Mutation::Insert(&input),
                "replace" => Mutation::Update(
                    kiarni_engine::writes::Operation::Replace,
                    br#"{"status":"new","n":9007199254740993}"#,
                ),
                "patch" => Mutation::Update(
                    kiarni_engine::writes::Operation::MergePatch,
                    br#"{"status":"new"}"#,
                ),
                "delete" => Mutation::Delete,
                _ => panic!("unknown worker mode"),
            };
            apply(None, &db, &collection, "a", mutation, |p| pause(p, &point)).unwrap();
        }
    }
    panic!("requested checkpoint not reached");
}

#[test]
fn kill_restart_preserves_generations_and_mutation_atomicity() {
    use std::{
        io::{BufRead, BufReader},
        process::{Command, Stdio},
        sync::mpsc,
        time::Duration,
    };
    for mode in ["create", "drop", "insert", "replace", "patch", "delete"] {
        let points: Vec<String> = if matches!(mode, "create" | "drop") {
            points().map(|v| format!("{v:?}")).to_vec()
        } else {
            use mutations::Checkpoint::*;
            [
                Document,
                OldEntry,
                NewEntry,
                Statistics,
                BeforeCommit,
                Committed,
            ]
            .map(|v| format!("{v:?}"))
            .to_vec()
        };
        for point in points {
            let (temp, db) = setup(mode);
            drop(db);
            let root = temp.path().join("store");
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--ignored",
                    "--exact",
                    "format_v2::collections::tests::crash_worker",
                    "--nocapture",
                ])
                .env("KIARNI_V2_GENERATION_ROOT", &root)
                .env("KIARNI_V2_GENERATION_POINT", &point)
                .env("KIARNI_V2_GENERATION_MODE", mode)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            let (tx, rx) = mpsc::sync_channel(1);
            let stdout = child.stdout.take().unwrap();
            let reader = std::thread::spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    if line.is_ok_and(|v| v.contains("GENERATION_CHECKPOINT")) {
                        let _ = tx.send(());
                        break;
                    }
                }
            });
            let reached = rx.recv_timeout(Duration::from_secs(15));
            let killed = child.kill();
            let status = child.wait().unwrap();
            reader.join().unwrap();
            reached.unwrap_or_else(|error| panic!("{mode}/{point}: {error}"));
            killed.unwrap();
            assert!(!status.success());
            let db = open(&root).unwrap();
            let committed = point == "Committed";
            assert_eq!(
                present(&db),
                match mode {
                    "create" => committed,
                    "drop" => !committed,
                    _ => true,
                }
            );
            assert_eq!(
                next(&db),
                if mode == "create" && !committed { 1 } else { 2 }
            );
            if present(&db) {
                let collection = Collection {
                    name: "tasks".into(),
                    id: 1,
                };
                audit(&db, &collection).unwrap();
                let read = db.begin_read().unwrap();
                let docs = read.open_table(DOCUMENTS).unwrap();
                let document = docs.get((1, "a")).unwrap();
                let expected = match mode {
                    "create" | "drop" => None,
                    "insert" => committed.then_some("new"),
                    "delete" => (!committed).then_some("old"),
                    _ => Some(if committed { "new" } else { "old" }),
                };
                assert_eq!(document.as_ref().map(|v|serde_json::from_slice::<serde_json::Value>(v.value()).unwrap()["status"].as_str().unwrap().to_owned()),expected.map(str::to_owned));
                if let Some(document) = document {
                    assert!(
                        std::str::from_utf8(document.value())
                            .unwrap()
                            .contains("9007199254740993")
                    );
                }
            } else {
                assert!(
                    db.begin_read()
                        .unwrap()
                        .open_table(STATS)
                        .unwrap()
                        .get(1)
                        .unwrap()
                        .is_none()
                );
                assert!(
                    db.begin_read()
                        .unwrap()
                        .open_table(CATALOG)
                        .unwrap()
                        .get(1)
                        .unwrap()
                        .is_none()
                );
            }
        }
    }
}
