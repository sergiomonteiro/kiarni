use kiarni_engine::query::Query;
use kiarni_storage_spike::secondary::{Checkpoint, Error, INDEX, Mutation, Store};
use redb::Durability;
use std::{
    sync::{Arc, Barrier, mpsc},
    time::{Duration, Instant},
};

fn query(filter: serde_json::Value) -> Query {
    Query::parse(&serde_json::to_vec(&serde_json::json!({"from":"orders","filter":filter,"limit":25,"project":["status","n"]})).unwrap()).unwrap()
}
fn eq(value: serde_json::Value) -> Query {
    query(serde_json::json!({"op":"eq","field":"status","value":value}))
}
fn same(control: &Store, indexed: &Store, q: &Query) {
    let reference = control.query(q, true).map(|out| out.documents);
    assert_eq!(reference, indexed.query(q, true).map(|out| out.documents));
    assert_eq!(reference, indexed.query(q, false).map(|out| out.documents));
}
fn doc(id: &str, status: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({"_id":id,"status":status})).unwrap()
}

#[test]
fn partial_string_index_matches_engine_and_falls_back_without_restricting_json() {
    let dir = tempfile::tempdir().unwrap();
    let control = Store::create(&dir.path().join("control"), false).unwrap();
    let indexed = Store::create(&dir.path().join("indexed"), true).unwrap();
    let mut documents = vec![
        br#"{"_id":"a","status":"open","n":9007199254740993,"rank":2}"#.to_vec(),
        br#"{"_id":"b","status":"closed","n":1.0,"rank":1}"#.to_vec(),
        br#"{"_id":"missing","n":0}"#.to_vec(),
        br#"{"_id":"null","status":null}"#.to_vec(),
        br#"{"_id":"number","status":1}"#.to_vec(),
        br#"{"_id":"boolean","status":true}"#.to_vec(),
        br#"{"_id":"array","status":["open"]}"#.to_vec(),
        br#"{"_id":"object","status":{"x":1}}"#.to_vec(),
        br#"{"_id":"escaped","status":"\u006fpen","rank":3}"#.to_vec(),
        br#"{"_id":"empty","status":""}"#.to_vec(),
        br#"{"_id":"unsupported","status":"closed","n":1e10001}"#.to_vec(),
    ];
    for (id, value) in [
        ("control", "a\0b".into()),
        ("unicode", "é".repeat(64)),
        ("long", "é".repeat(65)),
        ("composed", "é".into()),
        ("decomposed", "e\u{301}".into()),
    ] {
        documents.push(doc(id, &value));
    }
    for json in &documents {
        let id = serde_json::from_slice::<serde_json::Value>(json).unwrap()["_id"]
            .as_str()
            .unwrap()
            .to_owned();
        for store in [&control, &indexed] {
            store.mutate("orders", &id, Mutation::Insert(json)).unwrap();
        }
    }
    for store in [&control, &indexed] {
        store
            .mutate("other", "a", Mutation::Insert(&doc("a", "open")))
            .unwrap();
    }
    for value in [
        serde_json::json!("open"),
        serde_json::json!("a\0b"),
        serde_json::json!("closed"),
        serde_json::json!(""),
        serde_json::json!(null),
        serde_json::json!(true),
        serde_json::json!(1),
        serde_json::json!("é"),
        serde_json::json!("e\u{301}"),
        serde_json::json!("é".repeat(64)),
        serde_json::json!("é".repeat(65)),
    ] {
        same(&control, &indexed, &eq(value));
    }
    for raw in [
        r#"{"from":"orders","filter":{"op":"eq","field":"status","value":"open"},"sort":[{"field":"rank","direction":"desc"}],"project":["n"],"limit":1}"#,
        r#"{"from":"orders","id":"a","filter":{"op":"eq","field":"status","value":"open"},"limit":1}"#,
        r#"{"from":"orders","filter":{"op":"exists","field":"status"},"limit":25}"#,
        r#"{"from":"orders","filter":{"op":"not","arg":{"op":"eq","field":"status","value":"open"}},"limit":25}"#,
        r#"{"from":"orders","filter":{"op":"all","args":[{"op":"eq","field":"status","value":"open"},{"op":"gte","field":"rank","value":2}]},"limit":2}"#,
        r#"{"from":"orders","sort":[{"field":"n","direction":"asc"}],"limit":25}"#,
        r#"{"from":"other","filter":{"op":"eq","field":"status","value":"open"},"limit":25}"#,
    ] {
        same(&control, &indexed, &Query::parse(raw.as_bytes()).unwrap());
    }
    let result = indexed
        .query(&eq(serde_json::json!("open")), false)
        .unwrap();
    assert_eq!(result.plan, "status_eq");
    assert_eq!(result.examined, 2);
    assert!(result.documents.iter().any(|value| {
        std::str::from_utf8(value)
            .unwrap()
            .contains("9007199254740993")
    }));
    assert_eq!(
        indexed
            .query(&eq(serde_json::json!("é".repeat(65))), false)
            .unwrap()
            .plan,
        "scan"
    );
    assert_eq!(
        indexed.snapshot().unwrap().audit().unwrap(),
        documents.len() + 1
    );
}

#[test]
fn mutations_cover_index_membership_noops_conflicts_and_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("control");
    let ip = dir.path().join("indexed");
    let control = Store::create(&cp, false).unwrap();
    let indexed = Store::create(&ip, true).unwrap();
    for store in [&control, &indexed] {
        store
            .mutate("orders", "a", Mutation::Insert(&doc("a", "open")))
            .unwrap();
    }
    let mutations = vec![
        (false, br#"{"status":"paid"}"#.to_vec()),
        (false, br#"{"status":null}"#.to_vec()),
        (true, br#"{"status":null}"#.to_vec()),
        (true, br#"{"status":17}"#.to_vec()),
        (true, br#"{"status":["open"]}"#.to_vec()),
        (true, doc("a", &"x".repeat(129))),
        (true, doc("a", "open")),
    ];
    for (replace, body) in &mutations {
        for store in [&control, &indexed] {
            store
                .mutate(
                    "orders",
                    "a",
                    if *replace {
                        Mutation::Replace(body)
                    } else {
                        Mutation::Patch(body)
                    },
                )
                .unwrap();
        }
        for value in [
            serde_json::json!("open"),
            serde_json::json!("paid"),
            serde_json::json!(null),
            serde_json::json!(17),
            serde_json::json!("x".repeat(129)),
        ] {
            same(&control, &indexed, &eq(value));
        }
        assert_eq!(indexed.snapshot().unwrap().audit().unwrap(), 1);
    }
    for store in [&control, &indexed] {
        let mut phases = 0;
        assert!(
            !store
                .mutate_with_hook("orders", "a", Mutation::Patch(b"{}"), &mut |_| {
                    phases += 1;
                    Ok(())
                })
                .unwrap()
        );
        assert_eq!(phases, 0);
        assert_eq!(
            store.mutate("orders", "a", Mutation::Insert(&doc("a", "bad"))),
            Err(Error::Duplicate)
        );
        assert_eq!(
            store.mutate("orders", "missing", Mutation::Replace(b"{}")),
            Err(Error::Missing)
        );
        assert!(
            store
                .mutate("orders", "a", Mutation::Patch(br#"{"_id":"a"}"#))
                .is_err()
        );
        assert!(
            store
                .mutate(
                    "orders",
                    "a",
                    Mutation::Replace(br#"{"status":"x","status":"y"}"#)
                )
                .is_err()
        );
        store.mutate("orders", "a", Mutation::Delete).unwrap();
        assert_eq!(
            store.mutate("orders", "a", Mutation::Delete),
            Err(Error::Missing)
        );
    }
    drop(control);
    drop(indexed);
    let control = Store::open(&cp).unwrap();
    let indexed = Store::open(&ip).unwrap();
    same(&control, &indexed, &eq(serde_json::json!("open")));
    assert_eq!(indexed.snapshot().unwrap().audit().unwrap(), 0);
}

#[test]
fn transaction_abort_at_every_maintenance_stage_restores_documents_and_index() {
    for indexed in [false, true] {
        for delete in [false, true] {
            let phases = if indexed {
                vec![
                    Checkpoint::Document,
                    Checkpoint::OldIndexRemoved,
                    Checkpoint::NewIndexWritten,
                    Checkpoint::BeforeCommit,
                    Checkpoint::Committed,
                ]
            } else {
                vec![
                    Checkpoint::Document,
                    Checkpoint::BeforeCommit,
                    Checkpoint::Committed,
                ]
            };
            for phase in phases {
                let dir = tempfile::tempdir().unwrap();
                let store = Store::create(&dir.path().join("db"), indexed).unwrap();
                store
                    .mutate("orders", "a", Mutation::Insert(&doc("a", "open")))
                    .unwrap();
                assert_eq!(
                    store.mutate_with_hook(
                        "orders",
                        "a",
                        if delete {
                            Mutation::Delete
                        } else {
                            Mutation::Patch(br#"{"status":"paid"}"#)
                        },
                        &mut |point| {
                            let snapshot = store.snapshot()?;
                            snapshot.audit()?;
                            assert_eq!(
                                snapshot
                                    .query(&eq(serde_json::json!("open")), false)?
                                    .documents
                                    .len(),
                                usize::from(point != Checkpoint::Committed)
                            );
                            if point == phase {
                                Err(Error::Injected)
                            } else {
                                Ok(())
                            }
                        }
                    ),
                    Err(Error::Injected)
                );
                store.snapshot().unwrap().audit().unwrap();
                let open = store
                    .query(&eq(serde_json::json!("open")), false)
                    .unwrap()
                    .documents
                    .len();
                let paid = store
                    .query(&eq(serde_json::json!("paid")), false)
                    .unwrap()
                    .documents
                    .len();
                assert_eq!(open, usize::from(phase != Checkpoint::Committed));
                assert_eq!(paid, usize::from(phase == Checkpoint::Committed && !delete));
            }
        }
    }
}

#[test]
fn one_snapshot_keeps_index_and_documents_consistent_during_concurrent_writes() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::create(&dir.path().join("db"), true).unwrap());
    for i in 0..4 {
        let id = format!("d{i}");
        store
            .mutate("orders", &id, Mutation::Insert(&doc(&id, "open")))
            .unwrap();
    }
    let old = store.snapshot().unwrap();
    let barrier = Arc::new(Barrier::new(5));
    let (tx, rx) = mpsc::sync_channel(4);
    let mut workers = vec![];
    for i in 0..4 {
        let store = Arc::clone(&store);
        let barrier = Arc::clone(&barrier);
        let tx = tx.clone();
        workers.push(std::thread::spawn(move || {
            barrier.wait();
            for n in 0..8 {
                let value = if n % 2 == 0 { "paid" } else { "open" };
                store
                    .mutate(
                        "orders",
                        &format!("d{i}"),
                        Mutation::Patch(
                            serde_json::to_string(&serde_json::json!({"status":value}))
                                .unwrap()
                                .as_bytes(),
                        ),
                    )
                    .unwrap();
            }
            tx.send(()).unwrap();
        }));
    }
    barrier.wait();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut done = 0;
    while done < 4 {
        assert!(Instant::now() < deadline);
        let snapshot = store.snapshot().unwrap();
        assert_eq!(snapshot.audit().unwrap(), 4);
        for value in ["open", "paid"] {
            let q = eq(serde_json::json!(value));
            assert_eq!(
                snapshot.query(&q, false).unwrap().documents,
                snapshot.query(&q, true).unwrap().documents
            );
        }
        done += rx.try_iter().count();
    }
    for worker in workers {
        worker.join().unwrap();
    }
    assert_eq!(
        old.query(&eq(serde_json::json!("open")), false)
            .unwrap()
            .documents
            .len(),
        4
    );
    assert!(
        old.query(&eq(serde_json::json!("paid")), false)
            .unwrap()
            .documents
            .is_empty()
    );
}

#[test]
fn audit_detects_missing_extra_and_wrong_backreferences_instead_of_claiming_integrity() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::create(&dir.path().join("db"), true).unwrap();
    store
        .mutate("orders", "a", Mutation::Insert(&doc("a", "open")))
        .unwrap();
    let mut write = store.db.begin_write().unwrap();
    write.set_durability(Durability::Immediate).unwrap();
    write
        .open_table(INDEX)
        .unwrap()
        .remove(("orders", "open", "a"))
        .unwrap();
    write.commit().unwrap();
    assert_eq!(store.snapshot().unwrap().audit(), Err(Error::Layout));
    // A missing entry cannot be discovered by an index-only lookup: lifecycle/audit is essential.
    assert!(
        store
            .query(&eq(serde_json::json!("open")), false)
            .unwrap()
            .documents
            .is_empty()
    );
    assert_eq!(
        store
            .query(&eq(serde_json::json!("open")), true)
            .unwrap()
            .documents
            .len(),
        1
    );
    assert_eq!(
        store.mutate("orders", "a", Mutation::Patch(br#"{"status":"paid"}"#)),
        Err(Error::Layout)
    );
    let write = store.db.begin_write().unwrap();
    write
        .open_table(INDEX)
        .unwrap()
        .insert(("orders", "open", "missing"), ())
        .unwrap();
    write.commit().unwrap();
    assert!(matches!(
        store.query(&eq(serde_json::json!("open")), false),
        Err(Error::Layout)
    ));
}

#[test]
fn create_never_overwrites_and_open_recovers_the_recorded_mode() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let store = Store::create(&path, false).unwrap();
    store
        .mutate("orders", "a", Mutation::Insert(&doc("a", "open")))
        .unwrap();
    drop(store);
    assert!(matches!(Store::create(&path, true), Err(Error::Storage)));
    let reopened = Store::open(&path).unwrap();
    assert_eq!(
        reopened
            .query(&eq(serde_json::json!("open")), false)
            .unwrap()
            .plan,
        "scan"
    );
    assert_eq!(reopened.snapshot().unwrap().audit().unwrap(), 1);
}
