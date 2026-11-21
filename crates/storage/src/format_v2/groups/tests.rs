use super::*;
use crate::format_v2::backup::tests::{equal, fixture};
use writes::Operation::{MergePatch, Replace};

fn patch<'a>(collection: &'a str, id: &'a str, body: &'a [u8]) -> Update<'a> {
    Update {
        collection,
        id,
        body,
        operation: MergePatch,
    }
}
fn status(db: &Database) -> String {
    let read = db.begin_read().unwrap();
    let docs = read.open_table(DOCUMENTS).unwrap();
    let value = docs.get((1, "d000")).unwrap().unwrap();
    serde_json::from_slice::<serde_json::Value>(value.value()).unwrap()["status"]
        .as_str()
        .unwrap()
        .into()
}
fn audit(db: &Database) {
    for (name, id) in [("indexed", 1), ("empty", 2), ("plain", 3)] {
        lifecycle::audit(
            db,
            &Collection {
                name: name.into(),
                id,
            },
        )
        .unwrap();
    }
}

#[test]
fn ordered_results_capture_intermediate_states_and_match_sequential_writes() {
    let (temp, db, _) = fixture(false);
    let (_peer, peer, _) = fixture(false);
    let updates = [
        patch("indexed", "d000", br#"{"status":"middle"}"#),
        patch("missing", "a", b"{}"),
        patch("indexed", "d000", br#"{"_id":"other"}"#),
        patch("indexed", "d000", br#"{"status":"final"}"#),
        Update {
            collection: "plain",
            id: "x",
            operation: Replace,
            body: br#"{"status":"changed"}"#,
        },
        patch("indexed", "d000", b"{}"),
        patch("indexed", "../bad", b"{}"),
        patch("!invalid", "a", b"{}"),
        patch("empty", "absent", b"{}"),
    ];
    let flags = [true; 9];
    let old = db.begin_read().unwrap();
    let reference = Store::create(&temp.path().join("format1-reference")).unwrap();
    for name in ["indexed", "empty", "plain"] {
        reference.create_collection(name).unwrap();
    }
    let write = reference.db.begin_write().unwrap();
    {
        let source = old.open_table(DOCUMENTS).unwrap();
        let mut target = write.open_table(DOCUMENTS).unwrap();
        for row in source.iter().unwrap() {
            let (key, value) = row.unwrap();
            target.insert(key.value(), value.value()).unwrap();
        }
    }
    write.commit().unwrap();
    let baseline = reference
        .update_group_captured(&updates, DEFAULT_MAX_DOCUMENT_BYTES, &flags)
        .unwrap();
    let result = apply(
        &db,
        &updates,
        DEFAULT_MAX_DOCUMENT_BYTES,
        Some(&flags),
        |point| {
            if point != Point::Committed {
                assert_eq!(status(&db), "old");
                audit(&db);
            }
            Ok(())
        },
    )
    .unwrap();
    assert!(result.receipt.committed);
    assert_eq!(result.receipt.committed, baseline.receipt.committed);
    for (candidate, existing) in result
        .receipt
        .outcomes
        .iter()
        .zip(&baseline.receipt.outcomes)
    {
        match (candidate, existing) {
            (Ok(a), Ok(b)) => {
                assert_eq!(a.changed(), b.changed());
                assert_eq!(a.json(), b.json());
            }
            (Err(a), Err(b)) => assert_eq!(a, b),
            _ => panic!("candidate changed per-operation result"),
        }
    }
    for (candidate, existing) in result.transitions.iter().zip(&baseline.transitions) {
        match (candidate, existing) {
            (Some(a), Some(b)) => {
                assert_eq!(a.before, b.before);
                assert_eq!(a.after, b.after);
            }
            (None, None) => {}
            _ => panic!("candidate changed transition capture"),
        }
    }
    assert_eq!(result.receipt.outcomes.len(), 9);
    for (index, error) in [
        (1, Error::CollectionMissing),
        (2, Error::Write(writes::Error::ImmutableId)),
        (6, Error::Write(writes::Error::InvalidRouteId)),
        (7, Error::InvalidName),
        (8, Error::MissingDocument),
    ] {
        assert_eq!(result.receipt.outcomes[index].as_ref().err(), Some(&error));
        assert!(result.transitions[index].is_none());
    }
    assert!(!result.receipt.outcomes[5].as_ref().unwrap().changed());
    assert!(result.transitions[5].is_none());
    let first = result.transitions[0].as_ref().unwrap();
    let second = result.transitions[3].as_ref().unwrap();
    assert_eq!(first.after, second.before);
    assert!(
        std::str::from_utf8(&first.before)
            .unwrap()
            .contains("9007199254740993")
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&first.after).unwrap()["status"],
        "middle"
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&second.after).unwrap()["status"],
        "final"
    );
    assert_eq!(
        result.receipt.outcomes[0].as_ref().unwrap().json(),
        first.after.as_ref()
    );
    for i in [0, 3, 4] {
        let update = &updates[i];
        let collection = Collection {
            name: update.collection.into(),
            id: if i == 4 { 3 } else { 1 },
        };
        mutations::apply(
            &peer,
            &collection,
            update.id,
            mutations::Mutation::Update(update.operation, update.body),
            |_| Ok(()),
        )
        .unwrap();
    }
    equal(&db.begin_read().unwrap(), &peer.begin_read().unwrap());
    audit(&db);
    assert_eq!(status(&db), "final");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(
            old.open_table(DOCUMENTS)
                .unwrap()
                .get((1, "d000"))
                .unwrap()
                .unwrap()
                .value()
        )
        .unwrap()["status"],
        "old"
    );
}

#[test]
fn bounds_and_noops_preserve_the_existing_group_contract() {
    let (_temp, db, _) = fixture(false);
    assert!(matches!(
        apply(&db, &[], DEFAULT_MAX_DOCUMENT_BYTES, None, |_| Ok(())),
        Err(GroupError::InvalidGroup)
    ));
    let updates: Vec<_> = (0..32).map(|_| patch("indexed", "d000", b"{}")).collect();
    let result = apply(&db, &updates, DEFAULT_MAX_DOCUMENT_BYTES, None, |_| {
        panic!("no-op group must not commit")
    })
    .unwrap();
    assert!(!result.receipt.committed);
    assert!(result.transitions.iter().all(Option::is_none));
    let too_many: Vec<_> = (0..33).map(|_| patch("indexed", "d000", b"{}")).collect();
    assert!(matches!(
        apply(&db, &too_many, DEFAULT_MAX_DOCUMENT_BYTES, None, |_| Ok(())),
        Err(GroupError::InvalidGroup)
    ));
    assert!(matches!(
        apply(
            &db,
            &updates,
            DEFAULT_MAX_DOCUMENT_BYTES,
            Some(&[true]),
            |_| Ok(())
        ),
        Err(GroupError::InvalidGroup)
    ));
    assert!(matches!(
        apply(&db, &updates, 0, None, |_| Ok(())),
        Err(GroupError::Aborted(Error::InvalidLimit))
    ));
    let body = vec![b' '; DEFAULT_MAX_DOCUMENT_BYTES];
    let eight: Vec<_> = (0..8).map(|_| patch("indexed", "d000", &body)).collect();
    let result = apply(&db, &eight, DEFAULT_MAX_DOCUMENT_BYTES, None, |_| {
        panic!("invalid inputs must not commit")
    })
    .unwrap();
    assert_eq!(result.receipt.outcomes.len(), 8);
    assert!(result.receipt.outcomes.iter().all(Result::is_err));
    assert!(!result.receipt.committed);
    let nine: Vec<_> = (0..9).map(|_| patch("indexed", "d000", &body)).collect();
    assert!(matches!(
        apply(&db, &nine, DEFAULT_MAX_DOCUMENT_BYTES, None, |_| Ok(())),
        Err(GroupError::InvalidGroup)
    ));
    audit(&db);
    assert_eq!(status(&db), "old");
}

#[test]
fn integrity_failure_after_an_update_rolls_back_the_whole_group() {
    for case in 0..4 {
        let (_temp, db, _) = fixture(false);
        let write = db.begin_write().unwrap();
        match case {
            0 => {
                write
                    .open_table(STATS)
                    .unwrap()
                    .insert(3, ("plain", 0, 0))
                    .unwrap();
            }
            1 => {
                write
                    .open_table(DOCUMENTS)
                    .unwrap()
                    .insert((3, "x"), b"invalid".as_slice())
                    .unwrap();
            }
            2 => {
                write
                    .open_table(COUNTS)
                    .unwrap()
                    .insert((1, "second"), u64::MAX)
                    .unwrap();
            }
            3 => {
                write.delete_table(ENTRIES).unwrap();
            }
            _ => unreachable!(),
        }
        write.commit().unwrap();
        let before = db.begin_read().unwrap();
        let second = if case == 2 {
            patch("indexed", "d000", br#"{"status":"second"}"#)
        } else {
            patch("plain", "x", br#"{"status":"new"}"#)
        };
        assert!(matches!(
            apply(
                &db,
                &[patch("indexed", "d000", br#"{"status":"first"}"#), second],
                DEFAULT_MAX_DOCUMENT_BYTES,
                Some(&[true, true]),
                |_| Ok(())
            ),
            Err(GroupError::Aborted(_))
        ));
        assert_eq!(status(&db), "old");
        if case != 3 {
            equal(&before, &db.begin_read().unwrap());
        } else {
            assert!(
                !db.begin_read()
                    .unwrap()
                    .list_tables()
                    .unwrap()
                    .any(|t| t.name() == "secondary_entries")
            );
        }
    }
}

fn points() -> [Point; 4] {
    [
        Point::Operation(0),
        Point::Operation(1),
        Point::Staged,
        Point::Committed,
    ]
}
fn pair() -> [Update<'static>; 2] {
    [
        patch("indexed", "d000", br#"{"status":"new"}"#),
        patch("plain", "x", br#"{"status":"new"}"#),
    ]
}

#[test]
fn injected_failures_expose_no_receipt_before_commit_and_keep_snapshot_atomicity() {
    for failed in points() {
        let (_temp, db, _) = fixture(false);
        let before = db.begin_read().unwrap();
        let result = apply(
            &db,
            &pair(),
            DEFAULT_MAX_DOCUMENT_BYTES,
            Some(&[true, true]),
            |p| {
                if p == failed {
                    Err(Error::Storage)
                } else {
                    Ok(())
                }
            },
        );
        assert!(
            matches!(result,Err(error) if error==if failed==Point::Committed {GroupError::CommitUncertain}else{GroupError::Aborted(Error::Storage)})
        );
        if failed != Point::Committed {
            equal(&before, &db.begin_read().unwrap());
        }
        assert_eq!(
            status(&db),
            if failed == Point::Committed {
                "new"
            } else {
                "old"
            }
        );
        audit(&db);
    }
}

#[test]
fn concurrent_groups_and_readers_observe_only_complete_groups() {
    let (_temp, db, _) = fixture(false);
    std::thread::scope(|scope| {
        for thread in 0..4 {
            let db = &db;
            scope.spawn(move || {
                for n in 0..12 {
                    let body =
                        serde_json::to_vec(&serde_json::json!({"status":format!("s{thread}-{n}")}))
                            .unwrap();
                    apply(
                        db,
                        &[patch("indexed", "d000", &body), patch("plain", "x", &body)],
                        DEFAULT_MAX_DOCUMENT_BYTES,
                        None,
                        |_| Ok(()),
                    )
                    .unwrap();
                }
            });
        }
        for _ in 0..100 {
            let read = db.begin_read().unwrap();
            let docs = read.open_table(DOCUMENTS).unwrap();
            let a: serde_json::Value =
                serde_json::from_slice(docs.get((1, "d000")).unwrap().unwrap().value()).unwrap();
            let b: serde_json::Value =
                serde_json::from_slice(docs.get((3, "x")).unwrap().unwrap().value()).unwrap();
            if a["status"] != "old" {
                assert_eq!(a["status"], b["status"]);
            }
        }
    });
    audit(&db);
}

#[test]
#[ignore = "subprocess fixture for candidate grouped commits"]
fn worker() {
    use std::io::{Read, Write};
    let root = std::env::var_os("KIARNI_V2_GROUP_ROOT").unwrap();
    let requested = std::env::var("KIARNI_V2_GROUP_POINT").unwrap();
    let db = open(Path::new(&root)).unwrap();
    apply(
        &db,
        &pair(),
        DEFAULT_MAX_DOCUMENT_BYTES,
        Some(&[true, true]),
        |p| {
            if format!("{p:?}") == requested {
                println!("GROUP_V2_CHECKPOINT");
                std::io::stdout().flush().unwrap();
                let _ = std::io::stdin().read_exact(&mut [0u8; 1]);
                std::process::exit(90);
            }
            Ok(())
        },
    )
    .unwrap();
    panic!("checkpoint not reached");
}

#[test]
fn killed_groups_recover_both_collections_with_their_index_and_counts() {
    use std::{
        io::{BufRead, BufReader},
        process::{Command, Stdio},
        sync::mpsc,
        time::Duration,
    };
    for point in points() {
        let (temp, db, _) = fixture(false);
        drop(db);
        let root = temp.path().join("source");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "format_v2::groups::tests::worker",
                "--nocapture",
            ])
            .env("KIARNI_V2_GROUP_ROOT", &root)
            .env("KIARNI_V2_GROUP_POINT", format!("{point:?}"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let (tx, rx) = mpsc::sync_channel(1);
        let stdout = child.stdout.take().unwrap();
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if line.is_ok_and(|v| v.contains("GROUP_V2_CHECKPOINT")) {
                    let _ = tx.send(());
                    break;
                }
            }
        });
        let reached = rx.recv_timeout(Duration::from_secs(15));
        let killed = child.kill();
        let exit = child.wait().unwrap();
        reader.join().unwrap();
        reached.unwrap();
        killed.unwrap();
        assert!(!exit.success());
        let db = open(&root).unwrap();
        audit(&db);
        assert_eq!(
            status(&db),
            if point == Point::Committed {
                "new"
            } else {
                "old"
            }
        );
        let read = db.begin_read().unwrap();
        let docs = read.open_table(DOCUMENTS).unwrap();
        let b: serde_json::Value =
            serde_json::from_slice(docs.get((3, "x")).unwrap().unwrap().value()).unwrap();
        assert_eq!(
            b["status"],
            if point == Point::Committed {
                serde_json::json!("new")
            } else {
                serde_json::Value::Null
            }
        );
    }
}
