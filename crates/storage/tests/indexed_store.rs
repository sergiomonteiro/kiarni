use kiarni_core::{DEFAULT_MAX_DOCUMENT_BYTES, DocumentInput};
use kiarni_engine::{query::Query, writes::Operation};
use kiarni_storage::{Error, IndexedAccess, Store, updates::Update};

fn insert(store: &Store, collection: &kiarni_storage::Collection, id: &str, status: &str) {
    let json = format!("{{\"_id\":\"{id}\",\"status\":\"{status}\",\"n\":9007199254740993}}");
    let input = DocumentInput::parse(json.as_bytes(), DEFAULT_MAX_DOCUMENT_BYTES)
        .unwrap()
        .prepare_insert()
        .unwrap();
    store.insert_document(collection, &input).unwrap();
}
fn q() -> Query {
    Query::parse(br#"{"from":"items","limit":100,"filter":{"op":"eq","field":"status","value":"wanted"},"project":["n","status"]}"#).unwrap()
}

#[test]
fn explicit_facade_indexes_mutations_groups_snapshots_and_export() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("indexed");
    let store = Store::create(&root).unwrap();
    let collection = store.create_collection("items").unwrap();
    for n in 0..16 {
        insert(
            &store,
            &collection,
            &format!("d{n:02}"),
            if n == 0 { "wanted" } else { "bulk" },
        );
    }
    assert_eq!(
        store.query(&collection, &q()).unwrap().plan,
        IndexedAccess::Scan
    );
    store.create_index(&collection, "status").unwrap();
    let initial = store.query(&collection, &q()).unwrap();
    assert_eq!(initial.plan, IndexedAccess::Secondary);
    assert_eq!(initial.examined, 1);
    assert_eq!(initial.documents.len(), 1);
    assert!(
        std::str::from_utf8(&initial.documents[0])
            .unwrap()
            .contains("9007199254740993")
    );
    let old = store.snapshot().unwrap();
    store
        .update_document(
            &collection,
            "d00",
            Operation::MergePatch,
            br#"{"status":"gone"}"#,
            DEFAULT_MAX_DOCUMENT_BYTES,
        )
        .unwrap();
    let updates = [
        Update {
            collection: "items",
            id: "d01",
            operation: Operation::MergePatch,
            body: br#"{"status":"wanted"}"#,
        },
        Update {
            collection: "items",
            id: "d02",
            operation: Operation::Replace,
            body: br#"{"status":"wanted","n":9007199254740993}"#,
        },
    ];
    let captured = store
        .update_group_captured(&updates, DEFAULT_MAX_DOCUMENT_BYTES, &[true, true])
        .unwrap();
    assert!(captured.receipt.committed);
    assert!(captured.transitions.iter().all(Option::is_some));
    assert_eq!(store.query(&collection, &q()).unwrap().documents.len(), 2);
    assert_eq!(
        old.query(&collection, &q()).unwrap().documents,
        initial.documents
    );
    assert!(
        !store
            .update_document(
                &collection,
                "d01",
                Operation::MergePatch,
                b"{}",
                DEFAULT_MAX_DOCUMENT_BYTES
            )
            .unwrap()
            .changed()
    );
    let noops = [Update {
        collection: "items",
        id: "d01",
        operation: Operation::MergePatch,
        body: b"{}",
    }];
    assert!(
        !store
            .update_group(&noops, DEFAULT_MAX_DOCUMENT_BYTES)
            .unwrap()
            .committed
    );
    let before = store.get_document(&collection, "d01").unwrap().unwrap();
    assert_eq!(
        store
            .delete_document_captured(&collection, "d01")
            .unwrap()
            .as_ref(),
        before
    );
    assert_eq!(store.query(&collection, &q()).unwrap().documents.len(), 1);
    assert_eq!(store.count_documents("items").unwrap().count, 15);
    assert!(store.count_documents("items").unwrap().exact);
    let backup = temp.path().join("backup");
    assert_eq!(old.export(&backup).unwrap().documents, 16);
    let restored = temp.path().join("restored");
    assert_eq!(
        Store::restore_store(&backup, &restored).unwrap().documents,
        16
    );
    let copy = Store::open(&restored).unwrap();
    assert_eq!(
        copy.query(&copy.collection("items").unwrap(), &q())
            .unwrap()
            .documents,
        initial.documents
    );
    let backup = temp.path().join("latest");
    assert_eq!(store.export_snapshot(&backup).unwrap().documents, 15);
    store.rebuild_index(&collection).unwrap();
    store.drop_index(&collection).unwrap();
    assert_eq!(
        store.query(&collection, &q()).unwrap().plan,
        IndexedAccess::Scan
    );
    store.create_index(&collection, "status").unwrap();
    store.check_readable().unwrap();
    assert_eq!(store.audit().unwrap().documents, 15);
    drop(old);
    drop(store);
    let store = Store::open(&root).unwrap();
    assert_eq!(
        store
            .query(&store.collection("items").unwrap(), &q())
            .unwrap()
            .documents
            .len(),
        1
    );
    store.audit().unwrap();
}

#[test]
fn generations_admission_and_default_store_remain_explicit() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::create(&temp.path().join("indexed")).unwrap();
    let collection = store.create_collection("items").unwrap();
    insert(&store, &collection, "a", "wanted");
    assert_eq!(
        store.drop_collection(&collection),
        Err(Error::CollectionNotEmpty)
    );
    assert!(matches!(
        store.update_document(&collection, "a", Operation::MergePatch, b"{}", 0),
        Err(Error::InvalidLimit)
    ));
    assert!(
        store
            .update_document(
                &collection,
                "a",
                Operation::MergePatch,
                br#"{"_id":"changed"}"#,
                DEFAULT_MAX_DOCUMENT_BYTES
            )
            .is_err()
    );
    assert_eq!(
        store.get_document(&collection, "a").unwrap().unwrap(),
        br#"{"_id":"a","n":9007199254740993,"status":"wanted"}"#
    );
    store.delete_document(&collection, "a").unwrap();
    store.create_index(&collection, "status").unwrap();
    store.drop_collection(&collection).unwrap();
    let new = store.create_collection("items").unwrap();
    assert_eq!(
        store.get_document(&collection, "a"),
        Err(Error::CollectionMissing)
    );
    assert!(store.query(&collection, &q()).is_err());
    assert_eq!(
        store.insert_document(
            &collection,
            &DocumentInput::parse(br#"{"_id":"a"}"#, DEFAULT_MAX_DOCUMENT_BYTES)
                .unwrap()
                .prepare_insert()
                .unwrap()
        ),
        Err(Error::CollectionMissing)
    );
    insert(&store, &new, "a", "wanted");
    assert_eq!(store.query(&new, &q()).unwrap().plan, IndexedAccess::Scan);
    store.create_collection("other").unwrap();
    let page = store.list_collections(1, None).unwrap();
    assert_eq!(page.collections, ["items"]);
    assert_eq!(page.next_after.as_deref(), Some("items"));
    assert_eq!(
        store
            .list_collections(1, page.next_after.as_deref())
            .unwrap()
            .collections,
        ["other"]
    );
    assert!(store.list_collections(0, None).is_err());
    assert!(store.collection("bad/name").is_err());
    let legacy = temp.path().join("legacy");
    std::fs::create_dir(&legacy).unwrap();
    std::fs::write(
        legacy.join("format.json"),
        include_bytes!("fixtures/format-v1.json"),
    )
    .unwrap();
    std::fs::write(legacy.join("data.redb"), b"old development data").unwrap();
    let before = std::fs::read(legacy.join("data.redb")).unwrap();
    assert!(matches!(
        Store::open(&legacy),
        Err(Error::UnsupportedFormat)
    ));
    assert_eq!(std::fs::read(legacy.join("data.redb")).unwrap(), before);
    assert!(Store::create(&legacy).is_err());
    assert_eq!(std::fs::read(legacy.join("data.redb")).unwrap(), before);
}

#[test]
fn facade_writers_and_snapshots_keep_index_and_documents_together() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::create(&temp.path().join("store")).unwrap();
    let collection = store.create_collection("items").unwrap();
    for n in 0..32 {
        insert(
            &store,
            &collection,
            &format!("d{n:02}"),
            if n == 0 { "wanted" } else { "bulk" },
        );
    }
    store.create_index(&collection, "status").unwrap();
    std::thread::scope(|scope| {
        for thread in 0..4 {
            let (store, collection) = (&store, &collection);
            scope.spawn(move || {
                for n in 0..12 {
                    let body = if n % 2 == 0 {
                        br#"{"status":"wanted"}"#.as_slice()
                    } else {
                        br#"{"status":"other"}"#
                    };
                    store
                        .update_document(
                            collection,
                            &format!("d{thread:02}"),
                            Operation::MergePatch,
                            body,
                            DEFAULT_MAX_DOCUMENT_BYTES,
                        )
                        .unwrap();
                }
            });
        }
        for _ in 0..60 {
            let snapshot = store.snapshot().unwrap();
            let result = snapshot.query(&collection, &q()).unwrap();
            assert_eq!(result.plan, IndexedAccess::Secondary);
            for json in result.documents {
                let value: serde_json::Value = serde_json::from_slice(&json).unwrap();
                let document = snapshot
                    .get_document(&collection, value["_id"].as_str().unwrap())
                    .unwrap()
                    .unwrap();
                let value: serde_json::Value = serde_json::from_slice(&document).unwrap();
                assert_eq!(value["status"], "wanted");
            }
        }
    });
    store.audit().unwrap();
}
