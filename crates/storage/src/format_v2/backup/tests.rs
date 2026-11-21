use super::*;
use collections::{create_collection, drop_collection};
use lifecycle::{Action, maintain};
use mutations::{Mutation, apply};

pub(in crate::format_v2) fn fixture(large: bool) -> (tempfile::TempDir, Database, Collection) {
    let temp = tempfile::tempdir().unwrap();
    let db = create(&temp.path().join("source")).unwrap();
    let collection = create_collection(None, &db, "indexed", |_| Ok(())).unwrap();
    create_collection(None, &db, "empty", |_| Ok(())).unwrap();
    let plain = create_collection(None, &db, "plain", |_| Ok(())).unwrap();
    let removed = create_collection(None, &db, "removed", |_| Ok(())).unwrap();
    drop_collection(None, &db, &removed, |_| Ok(())).unwrap();
    let input = DocumentInput::parse(br#"{"_id":"x","status":null}"#, DEFAULT_MAX_DOCUMENT_BYTES)
        .unwrap()
        .prepare_insert()
        .unwrap();
    apply(
        None,
        &db,
        &plain,
        input.id(),
        Mutation::Insert(&input),
        |_| Ok(()),
    )
    .unwrap();
    let write = db.begin_write().unwrap();
    {
        let mut docs = write.open_table(DOCUMENTS).unwrap();
        docs.insert((1,"d000"),br#"{ "_id":"d000", "status":"old", "n":9007199254740993, "price":1.234567890123456789 }"#.as_slice()).unwrap();
        if large {
            for n in 1..600 {
                let id = format!("d{n:03}");
                let json = format!("{{\"_id\":\"{id}\",\"status\":\"s{}\"}}", n % 8);
                docs.insert((1, id.as_str()), json.as_bytes()).unwrap();
            }
            for n in 0..6 {
                let id = format!("large{n}");
                let json = format!(
                    "{{\"_id\":\"{id}\",\"status\":\"large\",\"padding\":\"{}\"}}",
                    "x".repeat(900000)
                );
                docs.insert((1, id.as_str()), json.as_bytes()).unwrap();
            }
        }
    }
    write.commit().unwrap();
    maintain(None, &db, &collection, Action::Build("status"), |_| Ok(())).unwrap();
    (temp, db, collection)
}

pub(in crate::format_v2) fn equal(left: &ReadTransaction, right: &ReadTransaction) {
    macro_rules! check {
        ($table:expr) => {{
            let l = left.open_table($table).unwrap();
            let r = right.open_table($table).unwrap();
            assert_eq!(l.len().unwrap(), r.len().unwrap());
            for (a, b) in l.iter().unwrap().zip(r.iter().unwrap()) {
                let (ak, av) = a.unwrap();
                let (bk, bv) = b.unwrap();
                assert_eq!(ak.value(), bk.value());
                assert_eq!(av.value(), bv.value());
            }
        }};
    }
    check!(METADATA);
    check!(COLLECTIONS);
    check!(DOCUMENTS);
    check!(CATALOG);
    check!(ENTRIES);
    check!(STATS);
    check!(COUNTS);
}

#[test]
fn multiple_batches_copy_one_snapshot_and_restore_without_touching_source() {
    let (temp, db, collection) = fixture(true);
    let read = db.begin_read().unwrap();
    let output = temp.path().join("backup");
    let mut batches = 0;
    let summary = export(&read, &output, |point| {
        if point == Point::Batch {
            batches += 1;
            assert!(!output.join("format.json").exists());
            if batches == 1 {
                apply(
                    None,
                    &db,
                    &collection,
                    "d000",
                    Mutation::Update(
                        kiarni_engine::writes::Operation::MergePatch,
                        br#"{"status":"later"}"#,
                    ),
                    |_| Ok(()),
                )
                .unwrap();
                create_collection(None, &db, "later", |_| Ok(())).unwrap();
            }
        }
        Ok(())
    })
    .unwrap();
    assert!(batches >= 4);
    assert_eq!(summary.collections, 3);
    assert_eq!(summary.documents, 607);
    assert!(summary.document_bytes > 5_000_000);
    let copy = open(&output).unwrap();
    equal(&read, &copy.begin_read().unwrap());
    drop(copy);
    let original_db = fs::read(output.join("data.redb")).unwrap();
    let original_manifest = fs::read(output.join("format.json")).unwrap();
    let restored = temp.path().join("restored");
    assert_eq!(restore(&output, &restored, |_| Ok(())).unwrap(), summary);
    assert_eq!(fs::read(output.join("data.redb")).unwrap(), original_db);
    assert_eq!(
        fs::read(output.join("format.json")).unwrap(),
        original_manifest
    );
    let target = open(&restored).unwrap();
    equal(&read, &target.begin_read().unwrap());
    assert_eq!(
        create_collection(None, &target, "next", |_| Ok(()))
            .unwrap()
            .id,
        5
    );
    assert_eq!(
        create_collection(None, &db, "next", |_| Ok(())).unwrap().id,
        6
    );
}

#[test]
fn empty_and_unindexed_stores_copy_and_existing_destinations_are_refused() {
    let temp = tempfile::tempdir().unwrap();
    let db = create(&temp.path().join("source")).unwrap();
    create_collection(None, &db, "plain", |_| Ok(())).unwrap();
    let read = db.begin_read().unwrap();
    let out = temp.path().join("out");
    assert_eq!(export(&read, &out, |_| Ok(())).unwrap().documents, 0);
    let before = fs::read(out.join("data.redb")).unwrap();
    assert!(export(&read, &out, |_| Ok(())).is_err());
    assert!(restore(&out, &out.join("nested"), |_| Ok(())).is_err());
    assert!(!out.join("nested").exists());
    assert_eq!(fs::read(out.join("data.redb")).unwrap(), before);
    let target = temp.path().join("restore");
    restore(&out, &target, |_| Ok(())).unwrap();
    assert!(restore(&out, &target, |_| Ok(())).is_err());
    equal(&read, &open(&target).unwrap().begin_read().unwrap());
}

#[test]
fn corrupt_sources_are_not_exported_or_restored() {
    for case in 0..9 {
        let (temp, db, _) = fixture(false);
        let write = db.begin_write().unwrap();
        match case {
            0 => {
                write
                    .open_table(ENTRIES)
                    .unwrap()
                    .remove((1, "old", "d000"))
                    .unwrap();
            }
            1 => {
                write
                    .open_table(ENTRIES)
                    .unwrap()
                    .insert((1, "old", "ghost"), ())
                    .unwrap();
            }
            2 => {
                write
                    .open_table(COUNTS)
                    .unwrap()
                    .insert((1, "old"), 99)
                    .unwrap();
            }
            3 => {
                write
                    .open_table(CATALOG)
                    .unwrap()
                    .insert(1, b"invalid".as_slice())
                    .unwrap();
            }
            4 => {
                write
                    .open_table(STATS)
                    .unwrap()
                    .insert(1, ("indexed", 0, 0))
                    .unwrap();
            }
            5 => {
                write
                    .open_table(DOCUMENTS)
                    .unwrap()
                    .insert((99, "ghost"), br#"{"_id":"ghost"}"#.as_slice())
                    .unwrap();
            }
            6 => {
                write
                    .open_table(ENTRIES)
                    .unwrap()
                    .insert((99, "x", "ghost"), ())
                    .unwrap();
            }
            7 => {
                write
                    .open_table(COUNTS)
                    .unwrap()
                    .insert((99, "x"), 1)
                    .unwrap();
            }
            8 => {
                write
                    .open_table(COLLECTIONS)
                    .unwrap()
                    .insert("duplicate", 1)
                    .unwrap();
            }
            _ => unreachable!(),
        }
        write.commit().unwrap();
        let output = temp.path().join("out");
        assert_eq!(
            export(&db.begin_read().unwrap(), &output, |_| Ok(())),
            Err(CopyFailure::Incomplete(Error::InvalidLayout))
        );
        assert!(!output.exists());
        drop(db);
        let source = temp.path().join("source");
        let before = fs::read(source.join("data.redb")).unwrap();
        assert_eq!(
            restore(&source, &output, |_| Ok(())),
            Err(CopyFailure::Incomplete(Error::InvalidLayout))
        );
        assert_eq!(fs::read(source.join("data.redb")).unwrap(), before);
        assert!(!output.exists());
    }
}

#[test]
fn failure_before_manifest_never_makes_a_partial_copy_readable() {
    for failed in [
        Point::Batch,
        Point::Audited,
        Point::BeforeManifest,
        Point::Published,
    ] {
        let (temp, db, _) = fixture(false);
        let output = temp.path().join("out");
        assert_eq!(
            export(&db.begin_read().unwrap(), &output, |point| {
                if point == failed {
                    Err(Error::Storage)
                } else {
                    Ok(())
                }
            }),
            Err(if failed == Point::Published {
                CopyFailure::PublicationUncertain
            } else {
                CopyFailure::Incomplete(Error::Storage)
            })
        );
        if failed == Point::Published {
            let copy = open(&output).unwrap();
            audit_snapshot(&copy.begin_read().unwrap()).unwrap();
        } else {
            assert!(!output.join("format.json").exists());
            assert!(open(&output).is_err());
        }
        assert!(export(&db.begin_read().unwrap(), &output, |_| Ok(())).is_err());
    }
}

#[test]
fn restore_refuses_live_or_incompatible_sources_before_creating_destination() {
    let (temp, db, _) = fixture(false);
    let source = temp.path().join("source");
    let output = temp.path().join("out");
    assert!(restore(&source, &output, |_| Ok(())).is_err());
    assert!(!output.exists());
    drop(db);
    restore(&source, &output, |_| Ok(())).unwrap();
    let old = temp.path().join("format1");
    fs::create_dir(&old).unwrap();
    fs::write(
        old.join("format.json"),
        include_bytes!("../../../tests/fixtures/format-v1.json"),
    )
    .unwrap();
    fs::write(old.join("data.redb"), b"old development data").unwrap();
    let before = fs::read(old.join("data.redb")).unwrap();
    let destination = temp.path().join("wrong-format");
    assert_eq!(
        restore(&old, &destination, |_| Ok(())),
        Err(CopyFailure::Incomplete(Error::UnsupportedFormat))
    );
    assert_eq!(fs::read(old.join("data.redb")).unwrap(), before);
    assert!(!destination.exists());
}

#[test]
#[ignore = "subprocess fixture for candidate snapshot copy"]
fn crash_worker() {
    use std::io::{Read, Write};
    let source = std::env::var_os("KIARNI_V2_COPY_SOURCE").unwrap();
    let target = std::env::var_os("KIARNI_V2_COPY_TARGET").unwrap();
    let requested = std::env::var("KIARNI_V2_COPY_POINT").unwrap();
    restore(Path::new(&source), Path::new(&target), |point| {
        if format!("{point:?}") == requested {
            println!("COPY_CHECKPOINT");
            std::io::stdout().flush().unwrap();
            let _ = std::io::stdin().read_exact(&mut [0u8; 1]);
            std::process::exit(90);
        }
        Ok(())
    })
    .unwrap();
    panic!("checkpoint not reached");
}

#[test]
fn killed_restore_keeps_artifact_intact_and_gates_publication() {
    use std::{
        io::{BufRead, BufReader},
        process::{Command, Stdio},
        sync::mpsc,
        time::Duration,
    };
    for point in [
        Point::Batch,
        Point::Audited,
        Point::BeforeManifest,
        Point::Published,
    ] {
        let (temp, db, _) = fixture(true);
        let source = temp.path().join("artifact");
        let summary = export(&db.begin_read().unwrap(), &source, |_| Ok(())).unwrap();
        let before = fs::read(source.join("data.redb")).unwrap();
        let manifest = fs::read(source.join("format.json")).unwrap();
        let target = temp.path().join("restored");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "format_v2::backup::tests::crash_worker",
                "--nocapture",
            ])
            .env("KIARNI_V2_COPY_SOURCE", &source)
            .env("KIARNI_V2_COPY_TARGET", &target)
            .env("KIARNI_V2_COPY_POINT", format!("{point:?}"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let (tx, rx) = mpsc::sync_channel(1);
        let stdout = child.stdout.take().unwrap();
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if line.is_ok_and(|v| v.contains("COPY_CHECKPOINT")) {
                    let _ = tx.send(());
                    break;
                }
            }
        });
        let reached = rx.recv_timeout(Duration::from_secs(15));
        let killed = child.kill();
        let status = child.wait().unwrap();
        reader.join().unwrap();
        reached.unwrap();
        killed.unwrap();
        assert!(!status.success());
        assert_eq!(fs::read(source.join("data.redb")).unwrap(), before);
        assert_eq!(fs::read(source.join("format.json")).unwrap(), manifest);
        if point == Point::Published {
            assert_eq!(
                audit_snapshot(&open(&target).unwrap().begin_read().unwrap()).unwrap(),
                summary
            );
        } else {
            assert!(!target.join("format.json").exists());
            assert!(open(&target).is_err());
        }
        assert!(restore(&source, &target, |_| Ok(())).is_err());
    }
}
