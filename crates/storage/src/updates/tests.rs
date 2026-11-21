use super::*;
use crate::{COLLECTIONS, DOCUMENTS, METADATA};
use crate::{Collection, DocumentInput};
use kiarni_core::{DEFAULT_MAX_DOCUMENT_BYTES, DocumentError};
use kiarni_engine::writes;

fn fixture() -> (tempfile::TempDir, Store, Collection, Collection) {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::create(&temp.path().join("store")).unwrap();
    let a = store.create_collection("a").unwrap();
    let b = store.create_collection("b").unwrap();
    for collection in [&a, &b] {
        let input = DocumentInput::parse(br#"{"_id":"doc","v":0}"#, 1024)
            .unwrap()
            .prepare_insert()
            .unwrap();
        store.insert_document(collection, &input).unwrap();
    }
    (temp, store, a, b)
}
fn patch<'a>(collection: &'a str, body: &'a [u8]) -> Update<'a> {
    Update {
        collection,
        id: "doc",
        operation: Operation::MergePatch,
        body,
    }
}
fn value(store: &Store, collection: &Collection) -> serde_json::Value {
    serde_json::from_slice(&store.get_document(collection, "doc").unwrap().unwrap()).unwrap()
}

#[test]
fn ordered_mixed_updates_retain_individual_json_and_commit_together() {
    let (temp, store, a, b) = fixture();
    let updates = [
        patch("a", br#"{"v":1}"#),
        Update {
            collection: "b",
            id: "doc",
            operation: Operation::Replace,
            body: br#"{"v":7}"#,
        },
        patch("a", br#"{"v":2}"#),
        patch("a", b"{}"),
        patch("a", b"bad json"),
        patch("missing", b"{}"),
        Update {
            id: "missing",
            ..patch("b", b"{}")
        },
        patch("b", br#"{"_id":"other"}"#),
    ];
    let receipt = store
        .update_group_with_hook(&updates, 1024, |checkpoint| {
            if checkpoint == Checkpoint::Staged {
                assert_eq!(value(&store, &a)["v"], 0);
                assert_eq!(value(&store, &b)["v"], 0);
                assert_eq!(store.fixture_documents(&a, None)?.len(), 1);
            }
            Ok(())
        })
        .unwrap();
    assert!(receipt.committed);
    assert_eq!(receipt.outcomes.len(), 8);
    for (index, expected) in [(0, 1), (1, 7), (2, 2), (3, 2)] {
        let prepared = receipt.outcomes[index].as_ref().unwrap();
        let json: serde_json::Value = serde_json::from_slice(prepared.json()).unwrap();
        assert_eq!(json["v"], expected);
        assert_eq!(json["_id"], "doc");
        assert_eq!(prepared.changed(), index != 3);
    }
    assert!(matches!(receipt.outcomes[4], Err(Error::Write(_))));
    assert!(matches!(receipt.outcomes[5], Err(Error::CollectionMissing)));
    assert!(matches!(receipt.outcomes[6], Err(Error::MissingDocument)));
    assert!(matches!(
        receipt.outcomes[7],
        Err(Error::Write(writes::Error::ImmutableId))
    ));
    assert_eq!(value(&store, &a)["v"], 2);
    assert_eq!(value(&store, &b)["v"], 7);
    drop(store);
    let reopened = Store::open(&temp.path().join("store")).unwrap();
    assert_eq!(value(&reopened, &a)["v"], 2);
    assert_eq!(value(&reopened, &b)["v"], 7);
}

#[test]
fn no_change_group_aborts_without_commit_and_preserves_raw_json() {
    let (_temp, store, a, _) = fixture();
    let original = store.get_document(&a, "doc").unwrap().unwrap();
    let receipt = store
        .update_group_with_hook(
            &[
                patch("a", b"{}"),
                patch("a", b"invalid"),
                patch("absent", b"{}"),
            ],
            10,
            |_| panic!("no-op must not commit"),
        )
        .unwrap();
    assert!(!receipt.committed);
    assert_eq!(receipt.outcomes[0].as_ref().unwrap().json(), original);
    assert!(!receipt.outcomes[0].as_ref().unwrap().changed());
    assert_eq!(store.get_document(&a, "doc").unwrap().unwrap(), original);
}

#[test]
fn group_and_document_limits_are_checked_without_mutation() {
    let (_temp, store, a, _) = fixture();
    assert!(matches!(
        store.update_group(&[], 1024),
        Err(GroupError::InvalidGroup)
    ));
    let too_many: Vec<_> = (0..33).map(|_| patch("a", b"{}")).collect();
    assert!(matches!(
        store.update_group(&too_many, 1024),
        Err(GroupError::InvalidGroup)
    ));
    let too_large = vec![b' '; MAX_BODY_BYTES + 1];
    assert!(matches!(
        store.update_group(&[patch("a", &too_large)], 1024),
        Err(GroupError::InvalidGroup)
    ));
    assert!(matches!(
        store.update_group(&[patch("a", b"{}")], 0),
        Err(GroupError::Aborted(Error::InvalidLimit))
    ));
    let oversized = vec![b' '; DEFAULT_MAX_DOCUMENT_BYTES + 1];
    let receipt = store
        .update_group(
            &[patch("a", &oversized), patch("a", br#"{"v":1}"#)],
            DEFAULT_MAX_DOCUMENT_BYTES,
        )
        .unwrap();
    assert!(matches!(
        receipt.outcomes[0],
        Err(Error::Write(writes::Error::Document(
            DocumentError::DocumentTooLarge
        )))
    ));
    assert!(receipt.outcomes[1].is_ok());
    assert_eq!(value(&store, &a)["v"], 1);
    let receipt = store
        .update_group(&[patch("a", br#"{"v":2}"#)], 10)
        .unwrap();
    assert!(!receipt.committed);
    assert!(matches!(
        receipt.outcomes[0],
        Err(Error::Write(writes::Error::Document(
            DocumentError::DocumentTooLarge
        )))
    ));
    assert_eq!(value(&store, &a)["v"], 1);
}

#[test]
fn faults_before_and_after_commit_never_return_success_receipts() {
    let (_temp, store, a, b) = fixture();
    let commit_count = |text: &str| -> u64 {
        text.lines()
            .find_map(|line| {
                line.strip_prefix("kiarni_storage_commits_total{outcome=\"confirmed\"} ")
            })
            .unwrap()
            .parse()
            .unwrap()
    };
    let before_commits = commit_count(&store.prometheus_metrics());
    let updates = [patch("a", br#"{"v":1}"#), patch("b", br#"{"v":2}"#)];
    assert!(matches!(
        store.update_group_with_hook(&updates, 1024, |_| Err(Error::Storage)),
        Err(GroupError::Aborted(Error::Storage))
    ));
    assert_eq!(value(&store, &a)["v"], 0);
    assert_eq!(value(&store, &b)["v"], 0);
    assert!(matches!(
        store.update_group_with_hook(&updates, 1024, |point| {
            if point == Checkpoint::Committed {
                Err(Error::Storage)
            } else {
                Ok(())
            }
        }),
        Err(GroupError::CommitUncertain)
    ));
    assert_eq!(value(&store, &a)["v"], 1);
    assert_eq!(value(&store, &b)["v"], 2);
    let metrics = store.prometheus_metrics();
    assert_eq!(commit_count(&metrics), before_commits + 1);
    assert!(metrics.contains("kiarni_storage_group_receipts_total{commit=\"confirmed\"} 0\n"));
    assert!(metrics.contains("kiarni_storage_commits_total{outcome=\"uncertain\"} 0\n"));
}

#[test]
fn stored_corruption_aborts_changes_already_staged_for_other_documents() {
    let (_temp, store, a, b) = fixture();
    let write = store.db.begin_write().unwrap();
    {
        let mut documents = write.open_table(DOCUMENTS).unwrap();
        documents
            .insert((b.id, "doc"), br#"{"_id":"wrong","v":0}"#.as_slice())
            .unwrap();
    }
    write.commit().unwrap();
    assert!(matches!(
        store.update_group(
            &[patch("a", br#"{"v":1}"#), patch("b", br#"{"v":2}"#)],
            1024
        ),
        Err(GroupError::Aborted(Error::InvalidLayout))
    ));
    assert_eq!(value(&store, &a)["v"], 0);
}

#[test]
fn missing_schema_is_not_recreated_and_invalid_metadata_is_fatal() {
    let (_temp, store, _, _) = fixture();
    let write = store.db.begin_write().unwrap();
    write.delete_table(DOCUMENTS).unwrap();
    write.commit().unwrap();
    assert!(matches!(
        store.update_group(&[patch("a", b"{}")], 1024),
        Err(GroupError::Aborted(Error::InvalidLayout))
    ));
    assert!(store.check_readable().is_err());

    let (_temp, store, _, _) = fixture();
    let write = store.db.begin_write().unwrap();
    {
        let mut metadata = write.open_table(METADATA).unwrap();
        metadata.insert("format_version", 99).unwrap();
    }
    write.commit().unwrap();
    assert!(matches!(
        store.update_group(&[patch("a", b"{}")], 1024),
        Err(GroupError::Aborted(Error::InvalidLayout))
    ));

    let (_temp, store, _, _) = fixture();
    let write = store.db.begin_write().unwrap();
    {
        let mut collections = write.open_table(COLLECTIONS).unwrap();
        collections.insert("b", 0).unwrap();
    }
    write.commit().unwrap();
    assert!(matches!(
        store.update_group(&[patch("a", br#"{"v":1}"#), patch("b", b"{}")], 1024),
        Err(GroupError::Aborted(Error::InvalidLayout))
    ));
    assert_eq!(value(&store, &store.collection("a").unwrap())["v"], 0);
}

#[test]
fn names_are_resolved_in_transaction_after_drop_and_recreate() {
    let (_temp, store, a, b) = fixture();
    store.delete_document(&a, "doc").unwrap();
    store.drop_collection(&a).unwrap();
    let recreated = store.create_collection("a").unwrap();
    let input = DocumentInput::parse(br#"{"_id":"doc","v":10}"#, 1024)
        .unwrap()
        .prepare_insert()
        .unwrap();
    store.insert_document(&recreated, &input).unwrap();
    let receipt = store
        .update_group(
            &[
                patch("a", br#"{"v":11}"#),
                patch("bad/name", b"{}"),
                Update {
                    id: "bad/id",
                    ..patch("a", b"{}")
                },
            ],
            1024,
        )
        .unwrap();
    assert!(receipt.committed);
    assert!(matches!(receipt.outcomes[1], Err(Error::InvalidName)));
    assert!(matches!(
        receipt.outcomes[2],
        Err(Error::Write(writes::Error::InvalidRouteId))
    ));
    assert_eq!(value(&store, &recreated)["v"], 11);
    assert_eq!(value(&store, &b)["v"], 0);
    assert!(matches!(
        store.get_document(&a, "doc"),
        Err(Error::CollectionMissing)
    ));
}

#[test]
fn concurrent_groups_do_not_interleave_partial_updates() {
    let (_temp, store, a, _) = fixture();
    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        let one = scope.spawn(|| {
            barrier.wait();
            store
                .update_group(
                    &[patch("a", br#"{"one":1}"#), patch("a", br#"{"one":2}"#)],
                    1024,
                )
                .unwrap()
        });
        let two = scope.spawn(|| {
            barrier.wait();
            store
                .update_group(
                    &[patch("a", br#"{"two":1}"#), patch("a", br#"{"two":2}"#)],
                    1024,
                )
                .unwrap()
        });
        for (receipt, own, other) in [
            (one.join().unwrap(), "one", "two"),
            (two.join().unwrap(), "two", "one"),
        ] {
            assert!(receipt.committed);
            assert!(receipt.outcomes.iter().all(Result::is_ok));
            let first: serde_json::Value =
                serde_json::from_slice(receipt.outcomes[0].as_ref().unwrap().json()).unwrap();
            let last: serde_json::Value =
                serde_json::from_slice(receipt.outcomes[1].as_ref().unwrap().json()).unwrap();
            assert_eq!(first[own], 1);
            assert_eq!(last[own], 2);
            assert_eq!(first[other], last[other]);
        }
    });
    let final_value = value(&store, &a);
    assert_eq!(final_value["one"], 2);
    assert_eq!(final_value["two"], 2);
}

#[test]
fn maximum_group_preserves_all_thirty_two_individual_results() {
    let (_temp, store, a, _) = fixture();
    let bodies: Vec<_> = (1..=MAX_OPERATIONS)
        .map(|number| format!(r#"{{"v":{number}}}"#).into_bytes())
        .collect();
    let updates: Vec<_> = bodies.iter().map(|body| patch("a", body)).collect();
    let receipt = store.update_group(&updates, 1024).unwrap();
    assert!(receipt.committed);
    assert_eq!(receipt.outcomes.len(), 32);
    for (index, outcome) in receipt.outcomes.iter().enumerate() {
        let result: serde_json::Value =
            serde_json::from_slice(outcome.as_ref().unwrap().json()).unwrap();
        assert_eq!(result["v"], index + 1);
    }
    assert_eq!(value(&store, &a)["v"], 32);
}

#[test]
fn captured_transitions_follow_each_intermediate_state_and_skip_noops() {
    let (_temp, store, _, _) = fixture();
    let updates = [
        patch("a", br#"{"v":1}"#),
        patch("a", br#"{"v":2}"#),
        patch("a", b"{}"),
        patch("missing", b"{}"),
        patch("b", br#"{"v":9}"#),
    ];
    let mut captured = store
        .update_group_captured(&updates, 1024, &[true, true, true, true, false])
        .unwrap();
    assert!(captured.receipt.committed);
    for (index, (before, after)) in [(0, (0, 1)), (1, (1, 2))] {
        let result = captured.receipt.outcomes[index]
            .as_mut()
            .unwrap()
            .shared_json();
        let transition = captured.transitions[index].as_ref().unwrap();
        assert!(Arc::ptr_eq(&result, &transition.after));
        let old: serde_json::Value = serde_json::from_slice(&transition.before).unwrap();
        let new: serde_json::Value = serde_json::from_slice(&transition.after).unwrap();
        assert_eq!(old["v"], before);
        assert_eq!(new["v"], after);
    }
    assert!(captured.transitions[2..].iter().all(Option::is_none));
    assert!(matches!(
        store.update_group_captured(&updates, 1024, &[]),
        Err(GroupError::InvalidGroup)
    ));
}

#[test]
fn delete_capture_returns_the_removed_state_only_after_commit() {
    let (_temp, store, a, _) = fixture();
    let before = store.get_document(&a, "doc").unwrap().unwrap();
    let removed = store.delete_document_captured(&a, "doc").unwrap();
    assert_eq!(&*removed, before);
    assert!(store.get_document(&a, "doc").unwrap().is_none());
    assert!(matches!(
        store.delete_document_captured(&a, "doc"),
        Err(GroupError::Aborted(Error::MissingDocument))
    ));
}

#[test]
#[ignore = "subprocess fixture for kill/restart"]
fn crash_worker() {
    use std::io::Write;
    let root = std::env::var_os("KIARNI_STORAGE_GROUP_ROOT").unwrap();
    let target = std::env::var("KIARNI_STORAGE_GROUP_POINT").unwrap();
    let store = Store::open(std::path::Path::new(&root)).unwrap();
    store
        .update_group_with_hook(
            &[patch("a", br#"{"v":1}"#), patch("b", br#"{"v":2}"#)],
            1024,
            |point| {
                if (point == Checkpoint::Staged && target == "staged")
                    || (point == Checkpoint::Committed && target == "committed")
                {
                    println!("GROUP_CHECKPOINT");
                    std::io::stdout().flush().unwrap();
                    loop {
                        std::thread::park();
                    }
                }
                Ok(())
            },
        )
        .unwrap();
}

#[test]
fn kill_restart_recovers_both_collections_atomically() {
    use std::{
        io::{BufRead, BufReader},
        process::{Command, Stdio},
        sync::mpsc,
        time::Duration,
    };
    for point in ["staged", "committed"] {
        let (temp, store, a, b) = fixture();
        drop(store);
        let root = temp.path().join("store");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "updates::tests::crash_worker",
                "--nocapture",
            ])
            .env("KIARNI_STORAGE_GROUP_ROOT", &root)
            .env("KIARNI_STORAGE_GROUP_POINT", point)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::sync_channel(1);
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if line.unwrap().contains("GROUP_CHECKPOINT") {
                    let _ = tx.send(());
                    break;
                }
            }
        });
        let checkpoint = rx.recv_timeout(Duration::from_secs(10));
        let _ = child.kill();
        child.wait().unwrap();
        reader.join().unwrap();
        checkpoint.unwrap();
        let reopened = Store::open(&root).unwrap();
        assert_eq!(
            value(&reopened, &a)["v"],
            if point == "committed" { 1 } else { 0 }
        );
        assert_eq!(
            value(&reopened, &b)["v"],
            if point == "committed" { 2 } else { 0 }
        );
    }
}
