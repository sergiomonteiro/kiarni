use super::*;

fn collection() -> Collection {
    Collection {
        name: "tasks".into(),
        id: 1,
    }
}

fn fixture(indexed: bool) -> (tempfile::TempDir, Database) {
    let temp = tempfile::tempdir().unwrap();
    let db = create(&temp.path().join("store")).unwrap();
    let write = db.begin_write().unwrap();
    write
        .open_table(METADATA)
        .unwrap()
        .insert("next_collection_id", 3)
        .unwrap();
    for (id, name) in [(1, "tasks"), (2, "other")] {
        write
            .open_table(COLLECTIONS)
            .unwrap()
            .insert(name, id)
            .unwrap();
        write
            .open_table(STATS)
            .unwrap()
            .insert(id, (name, 0, 0))
            .unwrap();
    }
    write.commit().unwrap();
    let mut bodies = vec![
        br#"{"_id":"a","kind":"red","n":9007199254740993}"#.to_vec(),
        br#"{"_id":"b","kind":"red"}"#.to_vec(),
        br#"{"_id":"c","kind":null}"#.to_vec(),
        br#"{"_id":"d","kind":"\u0000"}"#.to_vec(),
    ];
    // A maximum Unicode scalar followed by text must remain inside the prefix
    // range; an upper bound of the single scalar would silently omit this value.
    bodies.push(
        serde_json::to_vec(&serde_json::json!({"_id":"e","kind":"\u{10ffff}suffix"})).unwrap(),
    );
    for body in bodies {
        let input = DocumentInput::parse(&body, DEFAULT_MAX_DOCUMENT_BYTES)
            .unwrap()
            .prepare_insert()
            .unwrap();
        mutations::apply(
            &db,
            &collection(),
            input.id(),
            mutations::Mutation::Insert(&input),
            |_| Ok(()),
        )
        .unwrap();
    }
    if indexed {
        maintain(&db, &collection(), Action::Build("kind"), |_| Ok(())).unwrap();
    }
    (temp, db)
}

fn has_index(db: &Database) -> bool {
    db.begin_read()
        .unwrap()
        .open_table(CATALOG)
        .unwrap()
        .get(1)
        .unwrap()
        .is_some()
}
fn corrupt_count(db: &Database) {
    let write = db.begin_write().unwrap();
    write
        .open_table(COUNTS)
        .unwrap()
        .insert((1, "red"), 99)
        .unwrap();
    write.commit().unwrap();
}
fn points() -> [Point; 7] {
    [
        Point::Cleared,
        Point::Progress,
        Point::Built,
        Point::Ready,
        Point::Audited,
        Point::BeforeCommit,
        Point::Committed,
    ]
}

#[test]
fn build_rebuild_and_drop_audit_and_preserve_old_snapshots() {
    let (temp, db) = fixture(false);
    audit(&db, &collection()).unwrap();
    let previous = db.begin_read().unwrap();
    maintain(&db, &collection(), Action::Build("kind"), |_| {
        audit(&db, &collection()).unwrap();
        Ok(())
    })
    .unwrap();
    assert!(
        previous
            .open_table(CATALOG)
            .unwrap()
            .get(1)
            .unwrap()
            .is_none()
    );
    audit(&db, &collection()).unwrap();
    let read = db.begin_read().unwrap();
    assert_eq!(
        read.open_table(COUNTS)
            .unwrap()
            .get((1, "red"))
            .unwrap()
            .unwrap()
            .value(),
        2
    );
    assert!(
        read.open_table(ENTRIES)
            .unwrap()
            .get((1, "\u{10ffff}suffix", "e"))
            .unwrap()
            .is_some()
    );
    assert!(
        read.open_table(ENTRIES)
            .unwrap()
            .get((1, "\0", "d"))
            .unwrap()
            .is_some()
    );
    assert!(maintain(&db, &collection(), Action::Build("kind"), |_| Ok(())).is_err());
    corrupt_count(&db);
    assert_eq!(audit(&db, &collection()), Err(Error::InvalidLayout));
    maintain(&db, &collection(), Action::Rebuild, |_| Ok(())).unwrap();
    audit(&db, &collection()).unwrap();
    maintain(&db, &collection(), Action::Drop, |_| Ok(())).unwrap();
    assert!(!has_index(&db));
    audit(&db, &collection()).unwrap();
    // Old snapshot retains its index and count across rebuild and drop.
    assert_eq!(
        read.open_table(COUNTS)
            .unwrap()
            .get((1, "red"))
            .unwrap()
            .unwrap()
            .value(),
        2
    );
    assert_eq!(
        db.begin_read()
            .unwrap()
            .open_table(STATS)
            .unwrap()
            .get(2)
            .unwrap()
            .unwrap()
            .value(),
        ("other", 0, 0)
    );
    drop(read);
    drop(previous);
    drop(db);
    audit(&open(&temp.path().join("store")).unwrap(), &collection()).unwrap();
}

#[test]
fn audit_detects_missing_extra_wrong_counts_and_stats_and_rebuild_repairs() {
    for case in 0..8 {
        let (_temp, db) = fixture(true);
        let write = db.begin_write().unwrap();
        match case {
            0 => {
                write
                    .open_table(ENTRIES)
                    .unwrap()
                    .remove((1, "red", "a"))
                    .unwrap();
            }
            1 => {
                write
                    .open_table(ENTRIES)
                    .unwrap()
                    .insert((1, "red", "absent"), ())
                    .unwrap();
            }
            2 => {
                write
                    .open_table(ENTRIES)
                    .unwrap()
                    .insert((1, "wrong", "a"), ())
                    .unwrap();
            }
            3 => {
                write
                    .open_table(COUNTS)
                    .unwrap()
                    .insert((1, "red"), 99)
                    .unwrap();
            }
            4 => {
                write
                    .open_table(COUNTS)
                    .unwrap()
                    .insert((1, "extra"), 1)
                    .unwrap();
            }
            5 => {
                write
                    .open_table(STATS)
                    .unwrap()
                    .insert(1, ("tasks", 1, 0))
                    .unwrap();
            }
            6 => {
                write
                    .open_table(COUNTS)
                    .unwrap()
                    .insert((1, "red"), 0)
                    .unwrap();
            }
            7 => {
                write
                    .open_table(COUNTS)
                    .unwrap()
                    .remove((1, "red"))
                    .unwrap();
            }
            _ => unreachable!(),
        }
        write.commit().unwrap();
        assert_eq!(
            audit(&db, &collection()),
            Err(Error::InvalidLayout),
            "case {case}"
        );
        maintain(&db, &collection(), Action::Rebuild, |_| Ok(())).unwrap();
        audit(&db, &collection()).unwrap();
    }
}

#[test]
fn failure_windows_preserve_previous_state_and_report_uncertainty() {
    for action in 0..3 {
        for failed in points() {
            let (_temp, db) = fixture(action != 0);
            if action == 1 {
                corrupt_count(&db);
            }
            let action = match action {
                0 => Action::Build("kind"),
                1 => Action::Rebuild,
                _ => Action::Drop,
            };
            let result = maintain(&db, &collection(), action, |point| {
                if point == failed {
                    Err(Error::Storage)
                } else {
                    Ok(())
                }
            });
            assert_eq!(
                result,
                Err(if failed == Point::Committed {
                    Failure::CommitUncertain
                } else {
                    Failure::Aborted(Error::Storage)
                })
            );
            if matches!(action, Action::Rebuild) && failed != Point::Committed {
                assert_eq!(audit(&db, &collection()), Err(Error::InvalidLayout));
                assert_eq!(
                    db.begin_read()
                        .unwrap()
                        .open_table(COUNTS)
                        .unwrap()
                        .get((1, "red"))
                        .unwrap()
                        .unwrap()
                        .value(),
                    99
                );
            } else {
                audit(&db, &collection()).unwrap();
            }
            assert_eq!(
                has_index(&db),
                match action {
                    Action::Build(_) => failed == Point::Committed,
                    Action::Rebuild => true,
                    Action::Drop => failed != Point::Committed,
                }
            );
        }
    }
}

#[test]
fn maintenance_is_scoped_to_collection_identity() {
    let (_temp, db) = fixture(false);
    let other = Collection {
        name: "other".into(),
        id: 2,
    };
    let prepared = DocumentInput::parse(br#"{"_id":"a","kind":"red"}"#, DEFAULT_MAX_DOCUMENT_BYTES)
        .unwrap()
        .prepare_insert()
        .unwrap();
    mutations::apply(
        &db,
        &other,
        prepared.id(),
        mutations::Mutation::Insert(&prepared),
        |_| Ok(()),
    )
    .unwrap();
    maintain(&db, &other, Action::Build("kind"), |_| Ok(())).unwrap();
    for action in [Action::Build("kind"), Action::Rebuild, Action::Drop] {
        maintain(&db, &collection(), action, |_| Ok(())).unwrap();
        audit(&db, &other).unwrap();
        audit(&db, &collection()).unwrap();
        assert_eq!(
            db.begin_read()
                .unwrap()
                .open_table(COUNTS)
                .unwrap()
                .get((2, "red"))
                .unwrap()
                .unwrap()
                .value(),
            1
        );
        assert!(
            db.begin_read()
                .unwrap()
                .open_table(ENTRIES)
                .unwrap()
                .get((2, "red", "a"))
                .unwrap()
                .is_some()
        );
    }
    let stale = Collection {
        name: "tasks".into(),
        id: 2,
    };
    assert_eq!(
        maintain(&db, &stale, Action::Build("kind"), |_| Ok(())),
        Err(Failure::Aborted(Error::CollectionMissing))
    );
    audit(&db, &other).unwrap();
}

#[test]
fn budgets_invalid_documents_and_generation_abort_without_publication() {
    for case in 0..5 {
        let (_temp, db) = fixture(case == 3);
        let write = db.begin_write().unwrap();
        match case {
            0 => {
                let mut docs = write.open_table(DOCUMENTS).unwrap();
                for n in 0..10001 {
                    let id = format!("x{n}");
                    let json = format!("{{\"_id\":\"{id}\"}}");
                    docs.insert((1, id.as_str()), json.as_bytes()).unwrap();
                }
            }
            1 => {
                let mut docs = write.open_table(DOCUMENTS).unwrap();
                for n in 0..9 {
                    let id = format!("x{n}");
                    let json = format!(
                        "{{\"_id\":\"{id}\",\"padding\":\"{}\"}}",
                        "x".repeat(950000)
                    );
                    docs.insert((1, id.as_str()), json.as_bytes()).unwrap();
                }
            }
            2 => {
                write
                    .open_table(DOCUMENTS)
                    .unwrap()
                    .insert((1, "bad"), b"{}".as_slice())
                    .unwrap();
            }
            3 => {
                let mut entries = write.open_table(ENTRIES).unwrap();
                for n in 0..10001 {
                    entries
                        .insert((1, "extra", format!("x{n}").as_str()), ())
                        .unwrap();
                }
            }
            4 => {
                write
                    .open_table(COLLECTIONS)
                    .unwrap()
                    .insert("tasks", 2)
                    .unwrap();
            }
            _ => unreachable!(),
        }
        write.commit().unwrap();
        let action = if case == 3 {
            Action::Rebuild
        } else {
            Action::Build("kind")
        };
        assert!(maintain(&db, &collection(), action, |_| Ok(())).is_err());
        assert_eq!(has_index(&db), case == 3);
        if case == 3 {
            assert!(
                db.begin_read()
                    .unwrap()
                    .open_table(ENTRIES)
                    .unwrap()
                    .get((1, "extra", "x0"))
                    .unwrap()
                    .is_some()
            );
        }
        // Invalid definitions fail before changing the catalog.
        assert!(maintain(&db, &collection(), Action::Build("_id"), |_| Ok(())).is_err());
    }
}

#[test]
#[ignore = "subprocess fixture controlled by kill/restart parent"]
fn crash_worker() {
    use std::io::{Read, Write};
    let root = std::env::var_os("KIARNI_V2_LIFECYCLE_ROOT").unwrap();
    let requested = std::env::var("KIARNI_V2_LIFECYCLE_POINT").unwrap();
    let mode = std::env::var("KIARNI_V2_LIFECYCLE_MODE").unwrap();
    let db = open(Path::new(&root)).unwrap();
    let action = match mode.as_str() {
        "build" => Action::Build("kind"),
        "drop" => Action::Drop,
        _ => Action::Rebuild,
    };
    maintain(&db, &collection(), action, |point| {
        if format!("{point:?}") == requested {
            println!("LIFECYCLE_CHECKPOINT");
            std::io::stdout().flush().unwrap();
            // Parent holds stdin open. EOF on parent exit terminates the worker,
            // instead of leaving an orphan parked forever.
            let _ = std::io::stdin().read_exact(&mut [0u8; 1]);
            std::process::exit(90);
        }
        Ok(())
    })
    .unwrap();
    panic!("requested checkpoint was not reached");
}

#[test]
fn exact_document_and_byte_budgets_build_then_reject_one_more() {
    for byte_budget in [false, true] {
        let (_temp, db) = fixture(false);
        let write = db.begin_write().unwrap();
        {
            let mut docs = write.open_table(DOCUMENTS).unwrap();
            if byte_budget {
                let mut existing = 0u64;
                for row in docs.range((1, "")..(2, "")).unwrap() {
                    existing += row.unwrap().1.value().len() as u64;
                }
                let mut remaining = MAX_BYTES - existing;
                let mut n = 0;
                while remaining > 0 {
                    let id = format!("bulk{n}");
                    let shell = format!("{{\"_id\":\"{id}\",\"padding\":\"\"}}");
                    let bytes = remaining.min(950000) as usize;
                    assert!(bytes >= shell.len());
                    let json = format!(
                        "{{\"_id\":\"{id}\",\"padding\":\"{}\"}}",
                        "x".repeat(bytes - shell.len())
                    );
                    assert_eq!(json.len(), bytes);
                    docs.insert((1, id.as_str()), json.as_bytes()).unwrap();
                    remaining -= bytes as u64;
                    n += 1;
                }
            } else {
                for n in 0..9995 {
                    let id = format!("bulk{n}");
                    let json = format!("{{\"_id\":\"{id}\"}}");
                    docs.insert((1, id.as_str()), json.as_bytes()).unwrap();
                }
            }
        }
        write.commit().unwrap();
        maintain(&db, &collection(), Action::Build("kind"), |_| Ok(())).unwrap();
        audit(&db, &collection()).unwrap();
        let read = db.begin_read().unwrap();
        let (_, count, bytes) = read
            .open_table(STATS)
            .unwrap()
            .get(1)
            .unwrap()
            .unwrap()
            .value();
        if byte_budget {
            assert_eq!(bytes, MAX_BYTES);
        } else {
            assert_eq!(count, MAX_DOCUMENTS);
        }
        drop(read);
        let prepared = DocumentInput::parse(
            br#"{"_id":"beyond","kind":"red"}"#,
            DEFAULT_MAX_DOCUMENT_BYTES,
        )
        .unwrap()
        .prepare_insert()
        .unwrap();
        mutations::apply(
            &db,
            &collection(),
            prepared.id(),
            mutations::Mutation::Insert(&prepared),
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(
            maintain(&db, &collection(), Action::Rebuild, |_| Ok(())),
            Err(Failure::Aborted(Error::QueryLimit))
        );
        audit(&db, &collection()).unwrap();
        assert!(has_index(&db));
    }
}

#[test]
fn kill_restart_recovers_build_rebuild_drop_and_interrupted_repair() {
    use std::{
        io::{BufRead, BufReader},
        process::{Command, Stdio},
        sync::mpsc,
        time::Duration,
    };
    for mode in ["build", "rebuild", "drop", "repair"] {
        for point in points() {
            let (temp, db) = fixture(mode != "build");
            if mode == "repair" {
                corrupt_count(&db);
            }
            drop(db);
            let root = temp.path().join("store");
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--ignored",
                    "--exact",
                    "format_v2::lifecycle::tests::crash_worker",
                    "--nocapture",
                ])
                .env("KIARNI_V2_LIFECYCLE_ROOT", &root)
                .env("KIARNI_V2_LIFECYCLE_POINT", format!("{point:?}"))
                .env("KIARNI_V2_LIFECYCLE_MODE", mode)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            let stdout = child.stdout.take().unwrap();
            let (tx, rx) = mpsc::sync_channel(1);
            let reader = std::thread::spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    if line.is_ok_and(|line| line.contains("LIFECYCLE_CHECKPOINT")) {
                        let _ = tx.send(());
                        break;
                    }
                }
            });
            let reached = rx.recv_timeout(Duration::from_secs(15));
            let killed = child.kill();
            let status = child.wait().unwrap();
            reader.join().unwrap();
            reached.unwrap_or_else(|error| panic!("{mode}/{point:?}: {error}"));
            killed.expect("worker must still be alive at the checkpoint");
            assert!(!status.success());
            let db = open(&root).unwrap();
            if mode == "repair" && point != Point::Committed {
                assert_eq!(audit(&db, &collection()), Err(Error::InvalidLayout));
                assert_eq!(
                    db.begin_read()
                        .unwrap()
                        .open_table(COUNTS)
                        .unwrap()
                        .get((1, "red"))
                        .unwrap()
                        .unwrap()
                        .value(),
                    99
                );
            } else {
                audit(&db, &collection()).unwrap();
            }
            assert_eq!(
                has_index(&db),
                match mode {
                    "build" => point == Point::Committed,
                    "drop" => point != Point::Committed,
                    _ => true,
                }
            );
        }
    }
}
