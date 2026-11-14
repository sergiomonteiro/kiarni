use std::{
    collections::BTreeSet,
    sync::{Arc, Barrier, mpsc},
    thread,
    time::Duration,
};

use kiarni_core::{DEFAULT_MAX_DOCUMENT_BYTES, DocumentInput, PreparedInsert};
use kiarni_storage_spike::{DOCUMENTS, Error, STATUS, Store, export_snapshot};
use redb::{Durability, ReadTransaction, ReadableDatabase, ReadableTable, ReadableTableMetadata};

fn candidate(id: &str, status: &str) -> PreparedInsert {
    DocumentInput::parse(
        format!(r#"{{"_id":"{id}","status":"{status}"}}"#).as_bytes(),
        DEFAULT_MAX_DOCUMENT_BYTES,
    )
    .unwrap()
    .prepare_insert()
    .unwrap()
}

#[test]
fn generated_id_is_persisted_and_invalid_index_inputs_leave_no_rows() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("db.redb")).unwrap();
    for json in [
        br#"{"status":null}"#.as_slice(),
        br#"{}"#,
        br#"{"status":1}"#,
    ] {
        assert_eq!(
            store.insert_json("orders", json),
            Err(Error::InvalidDocument)
        );
    }
    assert_consistent(&store.db.begin_read().unwrap(), 0);
    let id = store.insert_json("orders", br#"{"status":"new"}"#).unwrap();
    assert_eq!(id.len(), 32);
    let json = store.get("orders", &id).unwrap().unwrap();
    let document = DocumentInput::parse(&json, DEFAULT_MAX_DOCUMENT_BYTES).unwrap();
    assert_eq!(document.id(), Some(id.as_str()));
    assert_consistent(&store.db.begin_read().unwrap(), 1);
}

fn assert_consistent(read: &ReadTransaction, expected: usize) {
    let documents = read.open_table(DOCUMENTS).unwrap();
    let index = read.open_table(STATUS).unwrap();
    let mut expected_index = BTreeSet::new();
    for entry in documents.iter().unwrap() {
        let (key, json) = entry.unwrap();
        let (collection, id) = key.value();
        let input = DocumentInput::parse(json.value(), DEFAULT_MAX_DOCUMENT_BYTES).unwrap();
        assert_eq!(input.id(), Some(id));
        let status: String = serde_json::from_str(input.fields()["status"].get()).unwrap();
        expected_index.insert((collection.to_owned(), status, id.to_owned()));
    }
    let actual_index: BTreeSet<_> = index
        .iter()
        .unwrap()
        .map(|entry| {
            let (key, _) = entry.unwrap();
            let (collection, status, id) = key.value();
            (collection.to_owned(), status.to_owned(), id.to_owned())
        })
        .collect();
    assert_eq!(documents.len().unwrap(), expected as u64);
    assert_eq!(actual_index, expected_index);
}

#[test]
fn mutations_conflicts_and_indexes_survive_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.redb");
    {
        let store = Store::open(&path).unwrap();
        store.insert("orders", &candidate("a", "new")).unwrap();
        store.insert("other", &candidate("a", "other")).unwrap();
        assert_eq!(
            store.insert("orders", &candidate("a", "bad")),
            Err(Error::DuplicateId)
        );
        store.replace("orders", &candidate("a", "paid")).unwrap();
        assert_eq!(
            store.replace("orders", &candidate("missing", "paid")),
            Err(Error::MissingDocument)
        );
        store.insert("orders", &candidate("b", "new")).unwrap();
        store.delete("orders", "b").unwrap();
        assert_eq!(store.delete("orders", "b"), Err(Error::MissingDocument));
        assert_consistent(&store.db.begin_read().unwrap(), 2);
    }
    let store = Store::open(&path).unwrap();
    assert_consistent(&store.db.begin_read().unwrap(), 2);
    assert_eq!(
        store.get("orders", "a").unwrap().unwrap(),
        candidate("a", "paid").json()
    );
    assert_eq!(store.get("orders", "b").unwrap(), None);
}

#[test]
fn rollback_between_document_and_index_leaves_neither() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("db.redb")).unwrap();
    let document = candidate("abort", "new");
    {
        let mut write = store.db.begin_write().unwrap();
        write.set_durability(Durability::Immediate).unwrap();
        write
            .open_table(DOCUMENTS)
            .unwrap()
            .insert(("orders", document.id()), document.json())
            .unwrap();
        // Inject failure before opening/updating the secondary index.
        write.abort().unwrap();
    }
    assert_consistent(&store.db.begin_read().unwrap(), 0);
}

#[test]
fn reader_snapshot_does_not_block_writer_and_backup_restores_that_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("db.redb")).unwrap());
    store.insert("orders", &candidate("a", "new")).unwrap();
    let snapshot = store.db.begin_read().unwrap();
    let (tx, rx) = mpsc::sync_channel(1);
    let writer_store = Arc::clone(&store);
    let writer = thread::spawn(move || {
        writer_store
            .replace("orders", &candidate("a", "paid"))
            .unwrap();
        writer_store
            .insert("orders", &candidate("b", "new"))
            .unwrap();
        tx.send(()).unwrap();
    });
    rx.recv_timeout(Duration::from_secs(10))
        .expect("writer blocked by read snapshot");
    writer.join().unwrap();
    assert_consistent(&snapshot, 1);
    assert_consistent(&store.db.begin_read().unwrap(), 2);
    let backup = dir.path().join("backup.redb");
    export_snapshot(&snapshot, &backup).unwrap();
    // Existing destination is never overwritten.
    assert_eq!(export_snapshot(&snapshot, &backup), Err(Error::Storage));
    let restored = Store::open(&backup).unwrap();
    assert_consistent(&restored.db.begin_read().unwrap(), 1);
    assert_eq!(
        restored.get("orders", "a").unwrap().unwrap(),
        candidate("a", "new").json()
    );
    assert_eq!(restored.get("orders", "b").unwrap(), None);
}

#[test]
fn concurrent_writers_have_one_winner_for_the_same_id() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("db.redb")).unwrap());
    let barrier = Arc::new(Barrier::new(4));
    let mut workers = Vec::new();
    for _ in 0..4 {
        let store = Arc::clone(&store);
        let barrier = Arc::clone(&barrier);
        workers.push(thread::spawn(move || {
            barrier.wait();
            store.insert("orders", &candidate("same", "new"))
        }));
    }
    let results: Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|r| **r == Err(Error::DuplicateId))
            .count(),
        3
    );
    assert_consistent(&store.db.begin_read().unwrap(), 1);
}

#[test]
fn bounded_scan_matches_the_secondary_index() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("db.redb")).unwrap();
    for id in 0..20 {
        store
            .insert(
                "orders",
                &candidate(
                    &format!("id{id:02}"),
                    if id % 2 == 0 { "paid" } else { "new" },
                ),
            )
            .unwrap();
    }
    let snapshot = store.db.begin_read().unwrap();
    let documents = snapshot.open_table(DOCUMENTS).unwrap();
    let index = snapshot.open_table(STATUS).unwrap();
    let scanned: Vec<_> = documents
        .iter()
        .unwrap()
        .filter_map(|entry| {
            let (key, value) = entry.unwrap();
            let parsed = DocumentInput::parse(value.value(), DEFAULT_MAX_DOCUMENT_BYTES).unwrap();
            (parsed.fields()["status"].get() == "\"paid\"").then(|| key.value().1.to_owned())
        })
        .take(5)
        .collect();
    let indexed: Vec<_> = index
        .range(("orders", "paid", "")..=("orders", "paid", "~"))
        .unwrap()
        .take(5)
        .map(|entry| entry.unwrap().0.value().2.to_owned())
        .collect();
    assert_eq!(scanned, indexed);
    assert_eq!(indexed.len(), 5);
    assert_consistent(&snapshot, 20);
}
