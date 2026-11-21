//! Versioned redb storage, collection catalog and transactional documents.

use std::{fmt, fs, io::Write, path::Path};

use kiarni_core::{DEFAULT_MAX_DOCUMENT_BYTES, DocumentInput, PreparedInsert};
use redb::{
    Database, Durability, ReadableDatabase, ReadableTable, ReadableTableMetadata, TableDefinition,
    TableHandle,
};
use serde::Serialize;

#[cfg(all(unix, feature = "profiling"))]
pub mod profiling;

mod backup;
mod format;
pub use backup::ExportSummary;
mod counts;
mod format_v2;
pub use format_v2::{
    IndexedAccess, IndexedCopyError, IndexedMaintenance, IndexedQueryError, IndexedQueryResult,
    IndexedSnapshot, IndexedWriteError, QueryCandidates, QueryReadError,
};
mod telemetry;
pub use telemetry::STATE_BYTES as TELEMETRY_STATE_BYTES;
pub mod updates;
pub use counts::{COUNT_MAX_BYTES, COUNT_MAX_DOCUMENTS, DocumentCount};

const METADATA: TableDefinition<&str, u64> = TableDefinition::new("metadata");
const COLLECTIONS: TableDefinition<&str, u64> = TableDefinition::new("collections");
const DOCUMENTS: TableDefinition<(u64, &str), &[u8]> = TableDefinition::new("documents");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Io,
    InvalidManifest,
    UnsupportedFormat,
    InvalidLayout,
    Storage,
    InvalidName,
    InvalidLimit,
    QueryLimit,
    CollectionExists,
    CollectionMissing,
    CollectionNotEmpty,
    CollectionIdsExhausted,
    InvalidDocument,
    MissingDocument,
    Write(kiarni_engine::writes::Error),
    DuplicateId,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Io => "format_io_error",
            Self::InvalidManifest => "invalid_format_manifest",
            Self::UnsupportedFormat => "unsupported_format",
            Self::InvalidLayout => "invalid_storage_layout",
            Self::Storage => "storage_unavailable",
            Self::InvalidName => "invalid_collection_name",
            Self::InvalidLimit => "invalid_limit",
            Self::QueryLimit => "query_scan_limit",
            Self::CollectionExists => "collection_exists",
            Self::CollectionMissing => "collection_not_found",
            Self::CollectionNotEmpty => "collection_not_empty",
            Self::CollectionIdsExhausted => "collection_ids_exhausted",
            Self::InvalidDocument => "invalid_document",
            Self::MissingDocument => "document_not_found",
            Self::Write(error) => return error.fmt(f),
            Self::DuplicateId => "duplicate_id",
        })
    }
}

impl std::error::Error for Error {}

fn io<T>(result: std::io::Result<T>) -> Result<T, Error> {
    result.map_err(|_| Error::Io)
}

fn storage<T, E>(result: Result<T, E>) -> Result<T, Error> {
    result.map_err(|_| Error::Storage)
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

/// Identity includes generation; a dropped/recreated collection invalidates it.
#[derive(Debug, PartialEq, Eq)]
pub struct Collection {
    name: String,
    id: u64,
}

pub struct Store {
    db: Database,
    telemetry: telemetry::Metrics,
    // Drop after Database; do not close an auxiliary descriptor while engine locks live.
    telemetry_file: fs::File,
}

#[derive(Serialize)]
pub struct CollectionPage {
    pub collections: Vec<String>,
    pub next_after: Option<String>,
}

impl Store {
    #[cfg(test)]
    fn fixture_documents(
        &self,
        collection: &Collection,
        id: Option<&str>,
    ) -> Result<Vec<DocumentInput>, Error> {
        let mut ast = serde_json::json!({"from": collection.name, "limit": 1000});
        if let Some(id) = id {
            ast["id"] = serde_json::json!(id);
        }
        let query = kiarni_engine::query::Query::parse(
            &serde_json::to_vec(&ast).map_err(|_| Error::Storage)?,
        )
        .map_err(|_| Error::Storage)?;
        self.query_candidates(collection, &query)
            .map(|v| v.documents)
            .map_err(|failure| match failure.error {
                IndexedQueryError::Storage(error) => error,
                _ => Error::Storage,
            })
    }

    pub fn get_fixture(&self, collection: &Collection, id: &str) -> Result<Option<Vec<u8>>, Error> {
        if !valid_name(id) {
            return Err(Error::InvalidDocument);
        }
        let read = storage(self.db.begin_read())?;
        let collections = storage(read.open_table(COLLECTIONS))?;
        check_collection(&collections, collection)?;
        let documents = storage(read.open_table(DOCUMENTS))?;
        storage(documents.get((collection.id, id)))?
            .map(|value| {
                if value.value().len() > DEFAULT_MAX_DOCUMENT_BYTES {
                    return Err(Error::InvalidLayout);
                }
                Ok(value.value().to_vec())
            })
            .transpose()
    }
}

fn check_collection(
    table: &impl ReadableTable<&'static str, u64>,
    collection: &Collection,
) -> Result<(), Error> {
    if storage(table.get(collection.name.as_str()))?.map(|value| value.value())
        != Some(collection.id)
    {
        return Err(Error::CollectionMissing);
    }
    Ok(())
}

fn create_file(path: &Path) -> Result<fs::File, Error> {
    let mut options = fs::OpenOptions::new();
    options.create_new(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    io(options.open(path))
}

fn sync_dir(path: &Path) -> Result<(), Error> {
    io(io(fs::File::open(path))?.sync_all())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readers_observe_committed_snapshot_during_an_inflight_update() {
        use kiarni_engine::writes::Operation::MergePatch;
        let temp = tempfile::tempdir().unwrap();
        let store = Store::create(&temp.path().join("store")).unwrap();
        let collection = store.create_collection("orders").unwrap();
        let original = br#"{"_id":"doc","v":1}"#;
        store.insert_fixture(&collection, original).unwrap();
        store
            .update_before_commit(&collection, "doc", MergePatch, br#"{"v":2}"#, 1024, || {
                assert_eq!(store.get_document(&collection, "doc")?.unwrap(), original);
                let snapshot = store.fixture_documents(&collection, None)?;
                assert_eq!(snapshot[0].fields()["v"].get(), "1");
                Ok(())
            })
            .unwrap();
        assert_eq!(
            store.get_document(&collection, "doc").unwrap().unwrap(),
            br#"{"_id":"doc","v":2}"#
        );
    }

    #[test]
    fn readability_check_detects_schema_failure_after_open() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::create(&temp.path().join("store")).unwrap();
        store.check_readable().unwrap();
        let write = store.db.begin_write().unwrap();
        write.delete_table(DOCUMENTS).unwrap();
        write.commit().unwrap();
        assert!(store.check_readable().is_err());
    }

    #[test]
    fn query_scan_budgets_are_checked_before_loading_and_id_lookup_bypasses_scan() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::create(&temp.path().join("store")).unwrap();
        let many = store.create_collection("many").unwrap();
        let large = store.create_collection("large").unwrap();
        let write = store.db.begin_write().unwrap();
        {
            let mut documents = write.open_table(DOCUMENTS).unwrap();
            for number in 0..10001 {
                let id = format!("d{number:05}");
                let json = format!(r#"{{"_id":"{id}"}}"#);
                documents
                    .insert((many.id, id.as_str()), json.as_bytes())
                    .unwrap();
            }
            for id in ["a", "b", "c", "d", "e", "f", "g", "h", "i"] {
                let shell = format!(r#"{{"_id":"{id}","payload":""}}"#);
                let json = format!(
                    r#"{{"_id":"{id}","payload":"{}"}}"#,
                    "x".repeat(DEFAULT_MAX_DOCUMENT_BYTES - shell.len())
                );
                assert_eq!(json.len(), DEFAULT_MAX_DOCUMENT_BYTES);
                documents.insert((large.id, id), json.as_bytes()).unwrap();
            }
        }
        write.commit().unwrap();
        assert!(matches!(
            store.fixture_documents(&many, None),
            Err(Error::QueryLimit)
        ));
        assert!(matches!(
            store.fixture_documents(&large, None),
            Err(Error::QueryLimit)
        ));
        assert_eq!(
            store
                .fixture_documents(&many, Some("d10000"))
                .unwrap()
                .len(),
            1
        );
        assert_eq!(store.fixture_documents(&large, Some("i")).unwrap().len(), 1);
    }

    #[test]
    fn no_op_skips_commit_checkpoint_and_failed_update_rolls_back() {
        use kiarni_engine::writes::Operation::MergePatch;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("store");
        let store = Store::create(&path).unwrap();
        let collection = store.create_collection("orders").unwrap();
        let original = br#"{ "_id": "doc", "v": 1, "keep": "xxxxxxxxxxxxxxxxxxxxxxxxxxxx" }"#;
        store.insert_fixture(&collection, original).unwrap();
        let no_op = store
            .update_before_commit(&collection, "doc", MergePatch, b"{}", 20, || {
                panic!("no-op reached commit checkpoint")
            })
            .unwrap();
        assert!(!no_op.changed());
        assert_eq!(no_op.json(), original);
        let failed =
            store.update_before_commit(&collection, "doc", MergePatch, br#"{"v":2}"#, 1024, || {
                Err(Error::Storage)
            });
        assert!(matches!(failed, Err(Error::Storage)));
        assert_eq!(
            store.get_document(&collection, "doc").unwrap().unwrap(),
            original
        );
        let oversized = store.update_document(&collection, "doc", MergePatch, br#"{"v":2}"#, 20);
        assert!(matches!(
            oversized,
            Err(Error::Write(kiarni_engine::writes::Error::Document(
                kiarni_core::DocumentError::DocumentTooLarge
            )))
        ));
        drop(store);
        let store = Store::open(&path).unwrap();
        assert_eq!(
            store.get_document(&collection, "doc").unwrap().unwrap(),
            original
        );
    }

    #[test]
    fn concurrent_patches_read_current_state_inside_the_transaction() {
        use kiarni_engine::writes::Operation::MergePatch;
        let temp = tempfile::tempdir().unwrap();
        let store = Store::create(&temp.path().join("store")).unwrap();
        let collection = store.create_collection("orders").unwrap();
        store
            .insert_fixture(&collection, br#"{"_id":"doc"}"#)
            .unwrap();
        let barrier = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            let one = scope.spawn(|| {
                barrier.wait();
                store
                    .update_document(&collection, "doc", MergePatch, br#"{"a":1}"#, 1024)
                    .unwrap();
            });
            let two = scope.spawn(|| {
                barrier.wait();
                store
                    .update_document(&collection, "doc", MergePatch, br#"{"b":2}"#, 1024)
                    .unwrap();
            });
            one.join().unwrap();
            two.join().unwrap();
        });
        assert_eq!(
            store.get_document(&collection, "doc").unwrap().unwrap(),
            br#"{"_id":"doc","a":1,"b":2}"#
        );
    }

    #[test]
    fn exhausted_counter_and_duplicate_creation_do_not_commit_partial_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("data");
        let store = Store::create(&root).unwrap();
        let collection = store.create_collection("first").unwrap();
        assert_eq!(
            store.create_collection("first"),
            Err(Error::CollectionExists)
        );
        let read = store.db.begin_read().unwrap();
        assert_eq!(
            read.open_table(METADATA)
                .unwrap()
                .get("next_collection_id")
                .unwrap()
                .unwrap()
                .value(),
            collection.id + 1
        );
        drop(read);
        let write = store.db.begin_write().unwrap();
        write
            .open_table(METADATA)
            .unwrap()
            .insert("next_collection_id", u64::MAX)
            .unwrap();
        write.commit().unwrap();
        assert_eq!(
            store.create_collection("overflow"),
            Err(Error::CollectionIdsExhausted)
        );
        assert_eq!(store.collection("overflow"), Err(Error::CollectionMissing));
        assert_eq!(store.collection("first").unwrap(), collection);
        assert_eq!(
            store
                .db
                .begin_read()
                .unwrap()
                .open_table(METADATA)
                .unwrap()
                .get("next_collection_id")
                .unwrap()
                .unwrap()
                .value(),
            u64::MAX
        );
    }

    #[test]
    fn missing_tables_and_inconsistent_internal_version_are_not_initialized() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("data");
        let store = Store::create(&root).unwrap();
        let write = store.db.begin_write().unwrap();
        write.delete_table(DOCUMENTS).unwrap();
        write.commit().unwrap();
        drop(store);
        assert!(matches!(Store::open(&root), Err(Error::InvalidLayout)));
        let db = Database::open(root.join("data.redb")).unwrap();
        let read = db.begin_read().unwrap();
        assert!(read.open_table(DOCUMENTS).is_err());
        drop(read);
        let write = db.begin_write().unwrap();
        write.open_table(DOCUMENTS).unwrap();
        write
            .open_table(METADATA)
            .unwrap()
            .insert("format_version", 99)
            .unwrap();
        write.commit().unwrap();
        drop(db);
        assert!(matches!(Store::open(&root), Err(Error::InvalidLayout)));
    }
}
