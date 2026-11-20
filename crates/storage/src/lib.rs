//! Versioned redb storage, collection catalog and transactional documents.

use std::{fmt, fs, io::Read, io::Write, path::Path};

use kiarni_core::{DEFAULT_MAX_DOCUMENT_BYTES, DocumentInput, PreparedInsert};
use redb::{
    Database, Durability, ReadableDatabase, ReadableTable, ReadableTableMetadata, TableDefinition,
    TableHandle,
};
use serde::{Deserialize, Serialize};

#[cfg(all(unix, feature = "profiling"))]
pub mod profiling;

mod backup;
pub use backup::ExportSummary;
mod counts;
// Candidate format is exercised only by tests until its complete lifecycle exists.
#[cfg(test)]
mod format_v2;
mod telemetry;
pub use telemetry::STATE_BYTES as TELEMETRY_STATE_BYTES;
pub mod updates;
pub use counts::{COUNT_MAX_BYTES, COUNT_MAX_DOCUMENTS, DocumentCount};

const METADATA: TableDefinition<&str, u64> = TableDefinition::new("metadata");
const COLLECTIONS: TableDefinition<&str, u64> = TableDefinition::new("collections");
const DOCUMENTS: TableDefinition<(u64, &str), &[u8]> = TableDefinition::new("documents");
const MAX_MANIFEST: u64 = 1024;

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

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    magic: String,
    version: u64,
    engine: String,
}

fn supported_manifest() -> Manifest {
    Manifest {
        magic: "kiarni".into(),
        version: 1,
        engine: "redb-4.5.0".into(),
    }
}

fn validate_manifest(root: &Path) -> Result<(), Error> {
    if !io(fs::symlink_metadata(root))?.file_type().is_dir() {
        return Err(Error::InvalidLayout);
    }
    let path = root.join("format.json");
    if !io(fs::symlink_metadata(&path))?.file_type().is_file() {
        return Err(Error::InvalidManifest);
    }
    let mut bytes = Vec::new();
    io(io(fs::File::open(path))?
        .take(MAX_MANIFEST + 1)
        .read_to_end(&mut bytes))?;
    if bytes.len() as u64 > MAX_MANIFEST {
        return Err(Error::InvalidManifest);
    }
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|_| Error::InvalidManifest)?;
    if manifest.magic != "kiarni" || manifest.version != 1 || manifest.engine != "redb-4.5.0" {
        return Err(Error::UnsupportedFormat);
    }
    if !io(fs::symlink_metadata(root.join("data.redb")))?
        .file_type()
        .is_file()
    {
        return Err(Error::InvalidLayout);
    }
    Ok(())
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
    /// Verify the current read path without changing persistent state.
    pub fn check_readable(&self) -> Result<(), Error> {
        self.telemetry
            .measure(crate::telemetry::Operation::Health, || {
                let read = storage(self.db.begin_read())?;
                let metadata = storage(read.open_table(METADATA))?;
                let version = storage(metadata.get("format_version"))?.map(|value| value.value());
                let next = storage(metadata.get("next_collection_id"))?.map(|value| value.value());
                if version != Some(1) || next.is_none_or(|id| id == 0) {
                    return Err(Error::InvalidLayout);
                }
                storage(read.open_table(COLLECTIONS))?;
                storage(read.open_table(DOCUMENTS))?;
                Ok(())
            })
    }

    /// Load a bounded snapshot of one collection, or use its primary index.
    /// Limits apply to documents examined, before filtering/projection.
    pub fn query_documents(
        &self,
        collection: &Collection,
        id: Option<&str>,
    ) -> Result<Vec<DocumentInput>, Error> {
        self.telemetry
            .measure(crate::telemetry::Operation::QueryRead, || {
                if id.is_some_and(|id| !valid_name(id)) {
                    return Err(Error::InvalidDocument);
                }
                let read = storage(self.db.begin_read())?;
                let collections = storage(read.open_table(COLLECTIONS))?;
                check_collection(&collections, collection)?;
                let table = storage(read.open_table(DOCUMENTS))?;
                let mut output = Vec::new();
                let mut bytes = 0usize;
                let mut append = |key: &str, json: &[u8]| -> Result<(), Error> {
                    if output.len() >= 10000 || bytes.saturating_add(json.len()) > 8 * 1024 * 1024 {
                        return Err(Error::QueryLimit);
                    }
                    let input = DocumentInput::parse(json, DEFAULT_MAX_DOCUMENT_BYTES)
                        .map_err(|_| Error::InvalidLayout)?;
                    if input.id() != Some(key) {
                        return Err(Error::InvalidLayout);
                    }
                    bytes += json.len();
                    output.push(input);
                    Ok(())
                };
                if let Some(id) = id {
                    if let Some(value) = storage(table.get((collection.id, id)))? {
                        append(id, value.value())?;
                    }
                } else {
                    for entry in
                        storage(table.range((collection.id, "")..=(collection.id, "\u{10ffff}")))?
                    {
                        let (key, value) = storage(entry)?;
                        append(key.value().1, value.value())?;
                    }
                }
                Ok(output)
            })
    }

    /// Lookup, transformation, equality and replacement share one write transaction.
    pub fn update_document(
        &self,
        collection: &Collection,
        id: &str,
        operation: kiarni_engine::writes::Operation,
        body: &[u8],
        max_bytes: usize,
    ) -> Result<kiarni_engine::writes::PreparedWrite, Error> {
        self.update_before_commit(collection, id, operation, body, max_bytes, || Ok(()))
    }

    fn update_before_commit(
        &self,
        collection: &Collection,
        id: &str,
        operation: kiarni_engine::writes::Operation,
        body: &[u8],
        max_bytes: usize,
        before_commit: impl FnOnce() -> Result<(), Error>,
    ) -> Result<kiarni_engine::writes::PreparedWrite, Error> {
        self.telemetry
            .measure(crate::telemetry::Operation::Update, || {
                if !(1..=DEFAULT_MAX_DOCUMENT_BYTES).contains(&max_bytes) {
                    return Err(Error::InvalidLimit);
                }
                #[cfg(all(unix, feature = "profiling"))]
                let begin = profiling::BEGIN.start();
                let mut write = storage(self.db.begin_write())?;
                storage(write.set_durability(Durability::Immediate))?;
                #[cfg(all(unix, feature = "profiling"))]
                drop(begin);
                let candidate;
                {
                    #[cfg(all(unix, feature = "profiling"))]
                    let lookup = profiling::LOOKUP.start();
                    let collections = storage(write.open_table(COLLECTIONS))?;
                    check_collection(&collections, collection)?;
                    let mut documents = storage(write.open_table(DOCUMENTS))?;
                    let current = storage(documents.get((collection.id, id)))?
                        .ok_or(Error::MissingDocument)?;
                    if current.value().len() > DEFAULT_MAX_DOCUMENT_BYTES {
                        return Err(Error::InvalidLayout);
                    }
                    #[cfg(all(unix, feature = "profiling"))]
                    drop(lookup);
                    #[cfg(all(unix, feature = "profiling"))]
                    let transform = profiling::TRANSFORM.start();
                    candidate = kiarni_engine::writes::prepare(
                        operation,
                        id,
                        current.value(),
                        body,
                        max_bytes,
                    )
                    .map_err(Error::Write)?;
                    #[cfg(all(unix, feature = "profiling"))]
                    drop(transform);
                    drop(current);
                    if candidate.changed() {
                        #[cfg(all(unix, feature = "profiling"))]
                        let _stage = profiling::STAGE.start();
                        storage(documents.insert((collection.id, id), candidate.json()))?;
                    }
                }
                if candidate.changed() {
                    before_commit()?;
                    #[cfg(all(unix, feature = "profiling"))]
                    let _commit = profiling::COMMIT.start();
                    storage(self.commit_transaction(write))?;
                } else {
                    storage(write.abort())?;
                }
                Ok(candidate)
            })
    }

    pub fn delete_document(&self, collection: &Collection, id: &str) -> Result<(), Error> {
        self.delete_document_inner(collection, id, false)
            .map(|_| ())
            .map_err(|error| match error {
                updates::GroupError::Aborted(error) => error,
                _ => Error::Storage,
            })
    }
    pub fn delete_document_captured(
        &self,
        collection: &Collection,
        id: &str,
    ) -> Result<std::sync::Arc<[u8]>, updates::GroupError> {
        self.delete_document_inner(collection, id, true)?
            .ok_or(updates::GroupError::CommitUncertain)
    }
    fn delete_document_inner(
        &self,
        collection: &Collection,
        id: &str,
        capture: bool,
    ) -> Result<Option<std::sync::Arc<[u8]>>, updates::GroupError> {
        self.telemetry
            .measure(crate::telemetry::Operation::Delete, || {
                if !valid_name(id) {
                    return Err(Error::InvalidDocument.into());
                }
                let mut write = storage(self.db.begin_write())?;
                storage(write.set_durability(Durability::Immediate))?;
                let before;
                {
                    let collections = storage(write.open_table(COLLECTIONS))?;
                    check_collection(&collections, collection)?;
                    let mut documents = storage(write.open_table(DOCUMENTS))?;
                    {
                        let current = storage(documents.get((collection.id, id)))?
                            .ok_or(Error::MissingDocument)?;
                        let document =
                            DocumentInput::parse(current.value(), DEFAULT_MAX_DOCUMENT_BYTES)
                                .map_err(|_| Error::InvalidLayout)?;
                        if document.id() != Some(id) {
                            return Err(Error::InvalidLayout.into());
                        }
                        before = if capture {
                            Some(std::sync::Arc::from(current.value()))
                        } else {
                            None
                        };
                    }
                    storage(documents.remove((collection.id, id)))?;
                }
                self.commit_transaction(write)
                    .map_err(|_| updates::GroupError::CommitUncertain)?;
                Ok(before)
            })
    }

    /// One snapshot, lexical order, at most limit+1 entries read.
    pub fn list_collections(
        &self,
        limit: usize,
        after: Option<&str>,
    ) -> Result<CollectionPage, Error> {
        self.telemetry
            .measure(crate::telemetry::Operation::CatalogList, || {
                use std::ops::Bound::{Excluded, Unbounded};
                if !(1..=1000).contains(&limit) {
                    return Err(Error::InvalidLimit);
                }
                if after.is_some_and(|name| !valid_name(name)) {
                    return Err(Error::InvalidName);
                }
                let read = storage(self.db.begin_read())?;
                let table = storage(read.open_table(COLLECTIONS))?;
                let lower = after.map_or(Unbounded, Excluded);
                let mut names = Vec::with_capacity(limit + 1);
                for entry in storage(table.range::<&str>((lower, Unbounded)))?.take(limit + 1) {
                    let (key, _) = storage(entry)?;
                    names.push(key.value().to_owned());
                }
                let more = names.len() > limit;
                names.truncate(limit);
                let next_after = if more { names.last().cloned() } else { None };
                Ok(CollectionPage {
                    collections: names,
                    next_after,
                })
            })
    }

    /// Explicit initialization only; existing/partial directories are rejected.
    /// Errors may leave a partial directory, never silently reused by open().
    pub fn create(root: &Path) -> Result<Self, Error> {
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        io(builder.create(root))?;
        let path = root.join("data.redb");
        let file = create_file(&path)?;
        let backend = storage(redb::backends::FileBackend::new(file))?;
        let db = storage(
            Database::builder()
                .set_cache_size(8 * 1024 * 1024)
                .create_with_backend(backend),
        )?;
        let mut write = storage(db.begin_write())?;
        storage(write.set_durability(Durability::Immediate))?;
        {
            let mut metadata = storage(write.open_table(METADATA))?;
            storage(metadata.insert("format_version", 1))?;
            storage(metadata.insert("next_collection_id", 1))?;
            storage(write.open_table(COLLECTIONS))?;
            storage(write.open_table(DOCUMENTS))?;
        }
        storage(write.commit())?;
        storage(db.close())?;
        io(io(fs::File::open(&path))?.sync_all())?;
        sync_dir(root)?;
        let bytes =
            serde_json::to_vec(&supported_manifest()).map_err(|_| Error::InvalidManifest)?;
        let mut manifest = create_file(&root.join("format.json"))?;
        io(manifest.write_all(&bytes))?;
        io(manifest.sync_all())?;
        sync_dir(root)?;
        let parent = root
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        sync_dir(parent)?;
        Self::open(root)
    }

    pub fn open(root: &Path) -> Result<Self, Error> {
        validate_manifest(root)?; // Must precede any redb writer/recovery open.
        let db = storage(
            Database::builder()
                .set_cache_size(8 * 1024 * 1024)
                .open(root.join("data.redb")),
        )?;
        {
            let read = storage(db.begin_read())?;
            let mut names = storage(read.list_tables())?
                .map(|table| table.name().to_owned())
                .collect::<Vec<_>>();
            names.sort();
            if names != ["collections", "documents", "metadata"]
                || storage(read.list_multimap_tables())?.next().is_some()
            {
                return Err(Error::InvalidLayout);
            }
            let metadata = read
                .open_table(METADATA)
                .map_err(|_| Error::InvalidLayout)?;
            let version = storage(metadata.get("format_version"))?.map(|value| value.value());
            let next = storage(metadata.get("next_collection_id"))?.map(|value| value.value());
            if storage(metadata.len())? != 2 || version != Some(1) || next.is_none_or(|id| id == 0)
            {
                return Err(Error::InvalidLayout);
            }
            read.open_table(COLLECTIONS)
                .map_err(|_| Error::InvalidLayout)?;
            read.open_table(DOCUMENTS)
                .map_err(|_| Error::InvalidLayout)?;
        }
        Ok(Self {
            db,
            telemetry: telemetry::Metrics::default(),
            telemetry_file: io(fs::File::open(root.join("data.redb")))?,
        })
    }

    pub fn create_collection(&self, name: &str) -> Result<Collection, Error> {
        self.telemetry
            .measure(crate::telemetry::Operation::CollectionCreate, || {
                if !valid_name(name) {
                    return Err(Error::InvalidName);
                }
                let mut write = storage(self.db.begin_write())?;
                storage(write.set_durability(Durability::Immediate))?;
                let id;
                {
                    let mut collections = storage(write.open_table(COLLECTIONS))?;
                    if storage(collections.get(name))?.is_some() {
                        return Err(Error::CollectionExists);
                    }
                    let mut metadata = storage(write.open_table(METADATA))?;
                    id = storage(metadata.get("next_collection_id"))?
                        .ok_or(Error::InvalidLayout)?
                        .value();
                    let next = id.checked_add(1).ok_or(Error::CollectionIdsExhausted)?;
                    storage(metadata.insert("next_collection_id", next))?;
                    storage(collections.insert(name, id))?;
                }
                storage(self.commit_transaction(write))?;
                Ok(Collection {
                    name: name.into(),
                    id,
                })
            })
    }

    pub fn collection(&self, name: &str) -> Result<Collection, Error> {
        self.telemetry
            .measure(crate::telemetry::Operation::CatalogLookup, || {
                if !valid_name(name) {
                    return Err(Error::InvalidName);
                }
                let read = storage(self.db.begin_read())?;
                let collections = storage(read.open_table(COLLECTIONS))?;
                let id = storage(collections.get(name))?
                    .ok_or(Error::CollectionMissing)?
                    .value();
                Ok(Collection {
                    name: name.into(),
                    id,
                })
            })
    }

    pub fn drop_collection(&self, collection: &Collection) -> Result<(), Error> {
        self.telemetry
            .measure(crate::telemetry::Operation::CollectionDrop, || {
                let mut write = storage(self.db.begin_write())?;
                storage(write.set_durability(Durability::Immediate))?;
                {
                    let mut collections = storage(write.open_table(COLLECTIONS))?;
                    check_collection(&collections, collection)?;
                    let documents = storage(write.open_table(DOCUMENTS))?;
                    if storage(
                        documents.range((collection.id, "")..=(collection.id, "\u{10ffff}")),
                    )?
                    .next()
                    .transpose()
                    .map_err(|_| Error::Storage)?
                    .is_some()
                    {
                        return Err(Error::CollectionNotEmpty);
                    }
                    storage(collections.remove(collection.name.as_str()))?;
                }
                storage(self.commit_transaction(write))
            })
    }

    /// Persist a prepared candidate with the uniqueness check in the write transaction.
    pub fn insert_document(
        &self,
        collection: &Collection,
        candidate: &PreparedInsert,
    ) -> Result<(), Error> {
        self.insert_fixture(collection, candidate.json())
    }

    /// Read persisted JSON under the format ceiling and verify its primary key.
    pub fn get_document(
        &self,
        collection: &Collection,
        id: &str,
    ) -> Result<Option<Vec<u8>>, Error> {
        self.telemetry
            .measure(crate::telemetry::Operation::DocumentRead, || {
                let Some(json) = self.get_fixture(collection, id)? else {
                    return Ok(None);
                };
                let document = DocumentInput::parse(&json, DEFAULT_MAX_DOCUMENT_BYTES)
                    .map_err(|_| Error::InvalidLayout)?;
                if document.id() != Some(id) {
                    return Err(Error::InvalidLayout);
                }
                Ok(Some(json))
            })
    }

    /// Explicit-ID helper retained for existing experiment fixtures.
    pub fn insert_fixture(&self, collection: &Collection, json: &[u8]) -> Result<(), Error> {
        self.telemetry
            .measure(crate::telemetry::Operation::Insert, || {
                let input = DocumentInput::parse(json, DEFAULT_MAX_DOCUMENT_BYTES)
                    .map_err(|_| Error::InvalidDocument)?;
                let id = input.id().ok_or(Error::InvalidDocument)?;
                let mut write = storage(self.db.begin_write())?;
                storage(write.set_durability(Durability::Immediate))?;
                {
                    let collections = storage(write.open_table(COLLECTIONS))?;
                    check_collection(&collections, collection)?;
                    let mut documents = storage(write.open_table(DOCUMENTS))?;
                    if storage(documents.get((collection.id, id)))?.is_some() {
                        return Err(Error::DuplicateId);
                    }
                    storage(documents.insert((collection.id, id), json))?;
                }
                storage(self.commit_transaction(write))
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
                let snapshot = store.query_documents(&collection, None)?;
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
            store.query_documents(&many, None),
            Err(Error::QueryLimit)
        ));
        assert!(matches!(
            store.query_documents(&large, None),
            Err(Error::QueryLimit)
        ));
        assert_eq!(
            store.query_documents(&many, Some("d10000")).unwrap().len(),
            1
        );
        assert_eq!(store.query_documents(&large, Some("i")).unwrap().len(), 1);
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
