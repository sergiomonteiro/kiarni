//! Consistent snapshots and restricted index recovery.
use super::*;
use kiarni_engine::query::Query;
/// Holds one consistent snapshot; keep installation locks until snapshots close.
pub struct IndexedSnapshot {
    read: redb::ReadTransaction,
}

/// Restricted recovery handle: no document reads, queries or ordinary writes.
pub struct IndexedMaintenance {
    db: Database,
    telemetry_file: fs::File,
}
impl IndexedMaintenance {
    pub fn open(root: &Path) -> Result<Self, Error> {
        Ok(Self {
            db: open(root)?,
            telemetry_file: io(fs::File::open(root.join("data.redb")))?,
        })
    }
    pub fn collection(&self, name: &str) -> Result<Collection, Error> {
        IndexedSnapshot {
            read: storage(self.db.begin_read())?,
        }
        .collection(name)
    }
    pub fn audit(&self) -> Result<ExportSummary, Error> {
        backup::audit_snapshot(&storage(self.db.begin_read())?)
    }
    /// Definition metadata only; does not make a damaged index queryable.
    pub fn index_field(&self, collection: &Collection) -> Result<Option<String>, Error> {
        IndexedSnapshot::new(&self.db)?.index_field(collection)
    }
    pub fn rebuild_index(&self, collection: &Collection) -> Result<(), IndexedWriteError> {
        lifecycle::maintain(
            None,
            &self.db,
            collection,
            lifecycle::Action::Rebuild,
            |_| Ok(()),
        )
    }
    pub fn drop_index(&self, collection: &Collection) -> Result<(), IndexedWriteError> {
        lifecycle::maintain(None, &self.db, collection, lifecycle::Action::Drop, |_| {
            Ok(())
        })
    }
    /// Become queryable only after a complete successful audit.
    pub fn into_store(self) -> Result<Store, Error> {
        backup::audit_snapshot(&storage(self.db.begin_read())?)?;
        Ok(Store {
            db: self.db,
            telemetry: crate::telemetry::Metrics::default(),
            telemetry_file: self.telemetry_file,
        })
    }
}

impl IndexedSnapshot {
    pub(super) fn new(db: &Database) -> Result<Self, Error> {
        Ok(Self {
            read: storage(db.begin_read())?,
        })
    }
    pub fn collection(&self, name: &str) -> Result<Collection, Error> {
        if !valid_name(name) {
            return Err(Error::InvalidName);
        }
        let id = storage(storage(self.read.open_table(COLLECTIONS))?.get(name))?
            .ok_or(Error::CollectionMissing)?
            .value();
        Ok(Collection {
            name: name.into(),
            id,
        })
    }
    pub fn index_field(&self, collection: &Collection) -> Result<Option<String>, Error> {
        check_collection(&storage(self.read.open_table(COLLECTIONS))?, collection)?;
        storage(storage(self.read.open_table(CATALOG))?.get(collection.id))?
            .map(|value| decode_definition(value.value()).map(str::to_owned))
            .transpose()
    }
    pub fn list_collections(
        &self,
        limit: usize,
        after: Option<&str>,
    ) -> Result<CollectionPage, Error> {
        use std::ops::Bound::{Excluded, Unbounded};
        if !(1..=1000).contains(&limit) {
            return Err(Error::InvalidLimit);
        }
        if after.is_some_and(|v| !valid_name(v)) {
            return Err(Error::InvalidName);
        }
        let table = storage(self.read.open_table(COLLECTIONS))?;
        let mut names = Vec::with_capacity(limit + 1);
        for row in storage(table.range::<&str>((after.map_or(Unbounded, Excluded), Unbounded)))?
            .take(limit + 1)
        {
            names.push(storage(row)?.0.value().to_owned());
        }
        let more = names.len() > limit;
        names.truncate(limit);
        let next_after = if more { names.last().cloned() } else { None };
        Ok(CollectionPage {
            collections: names,
            next_after,
        })
    }
    pub fn get_document(
        &self,
        collection: &Collection,
        id: &str,
    ) -> Result<Option<Vec<u8>>, Error> {
        if !valid_name(id) {
            return Err(Error::InvalidDocument);
        }
        check_collection(&storage(self.read.open_table(COLLECTIONS))?, collection)?;
        storage(storage(self.read.open_table(DOCUMENTS))?.get((collection.id, id)))?
            .map(|v| {
                let parsed = DocumentInput::parse(v.value(), DEFAULT_MAX_DOCUMENT_BYTES)
                    .map_err(|_| Error::InvalidLayout)?;
                if parsed.id() != Some(id) {
                    return Err(Error::InvalidLayout);
                }
                Ok(v.value().to_vec())
            })
            .transpose()
    }
    pub fn count_documents(&self, name: &str) -> Result<DocumentCount, Error> {
        let collection = self.collection(name)?;
        let table = storage(self.read.open_table(DOCUMENTS))?;
        let end = collection.id.checked_add(1).ok_or(Error::InvalidLayout)?;
        let (mut count, mut bytes) = (0, 0usize);
        for row in storage(table.range((collection.id, "")..(end, "")))? {
            let (_, value) = storage(row)?;
            let length = value.value().len();
            if length > DEFAULT_MAX_DOCUMENT_BYTES {
                return Err(Error::InvalidLayout);
            }
            if count == COUNT_MAX_DOCUMENTS || bytes.saturating_add(length) > COUNT_MAX_BYTES {
                return Ok(DocumentCount {
                    count,
                    exact: false,
                });
            }
            count += 1;
            bytes += length;
        }
        Ok(DocumentCount { count, exact: true })
    }
    pub fn query_candidates(
        &self,
        collection: &Collection,
        query: &Query,
    ) -> Result<QueryCandidates, QueryReadError> {
        queries::load(&self.read, collection, query, queries::Mode::Automatic)
    }
    pub fn query(
        &self,
        collection: &Collection,
        query: &Query,
    ) -> Result<IndexedQueryResult, IndexedQueryError> {
        queries::execute(&self.read, collection, query, queries::Mode::Automatic)
    }
    pub fn export(&self, root: &Path) -> Result<ExportSummary, IndexedCopyError> {
        backup::export(&self.read, root, |_| Ok(()))
    }
}

impl std::fmt::Display for IndexedWriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Aborted(error) => error.fmt(f),
            Self::CommitUncertain => f.write_str("commit_uncertain"),
        }
    }
}
impl std::error::Error for IndexedWriteError {}
impl std::fmt::Display for IndexedCopyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Incomplete(error) => error.fmt(f),
            Self::PublicationUncertain => f.write_str("publication_uncertain"),
        }
    }
}
impl std::error::Error for IndexedCopyError {}
impl std::fmt::Display for IndexedQueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Storage(error) => error.fmt(f),
            Self::Query(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for IndexedQueryError {}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queryable_open_requires_audit_and_recovery_handle_cannot_skip_it() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("store");
        let store = Store::create(&path).unwrap();
        let collection = store.create_collection("items").unwrap();
        let input = DocumentInput::parse(
            br#"{"_id":"a","status":"ready"}"#,
            DEFAULT_MAX_DOCUMENT_BYTES,
        )
        .unwrap()
        .prepare_insert()
        .unwrap();
        store.insert_document(&collection, &input).unwrap();
        store.create_index(&collection, "status").unwrap();
        // Corruption injection is confined to this private unit test.
        let write = store.db.begin_write().unwrap();
        write
            .open_table(ENTRIES)
            .unwrap()
            .remove((1, "ready", "a"))
            .unwrap();
        write
            .open_table(COUNTS)
            .unwrap()
            .remove((1, "ready"))
            .unwrap();
        write.commit().unwrap();
        drop(store);
        assert!(matches!(Store::open(&path), Err(Error::InvalidLayout)));
        let recovery = IndexedMaintenance::open(&path).unwrap();
        assert_eq!(recovery.audit(), Err(Error::InvalidLayout));
        assert!(matches!(recovery.into_store(), Err(Error::InvalidLayout)));
        let recovery = IndexedMaintenance::open(&path).unwrap();
        let collection = recovery.collection("items").unwrap();
        recovery.rebuild_index(&collection).unwrap();
        recovery.audit().unwrap();
        let store = recovery.into_store().unwrap();
        assert!(store.get_document(&collection, "a").unwrap().is_some());
        store.audit().unwrap();
        drop(store);
        let recovery = IndexedMaintenance::open(&path).unwrap();
        recovery
            .drop_index(&recovery.collection("items").unwrap())
            .unwrap();
        recovery.into_store().unwrap().audit().unwrap();
    }
}
