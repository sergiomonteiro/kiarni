//! Explicit format-2 storage API. Owns the engine; no raw writes escape this facade.
use super::*;
use crate::updates::{CapturedReceipt, GroupError, Receipt, Update};
use kiarni_engine::{
    query::Query,
    writes::{self, Operation, PreparedWrite},
};

/// Experimental format-2 store. Callers own installation locking/lifecycle.
/// Not selected by the current server; maintenance calls are synchronous/bounded.
pub struct IndexedStore {
    db: Database,
}
/// Holds one consistent snapshot; keep installation locks until snapshots close.
pub struct IndexedSnapshot {
    read: redb::ReadTransaction,
}

/// Restricted recovery handle: no document reads, queries or ordinary writes.
pub struct IndexedMaintenance {
    db: Database,
}
impl IndexedMaintenance {
    pub fn open(root: &Path) -> Result<Self, Error> {
        Ok(Self { db: open(root)? })
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
    pub fn rebuild_index(&self, collection: &Collection) -> Result<(), IndexedWriteError> {
        lifecycle::maintain(&self.db, collection, lifecycle::Action::Rebuild, |_| Ok(()))
    }
    pub fn drop_index(&self, collection: &Collection) -> Result<(), IndexedWriteError> {
        lifecycle::maintain(&self.db, collection, lifecycle::Action::Drop, |_| Ok(()))
    }
    /// Become queryable only after a complete successful audit.
    pub fn into_store(self) -> Result<IndexedStore, Error> {
        backup::audit_snapshot(&storage(self.db.begin_read())?)?;
        Ok(IndexedStore { db: self.db })
    }
}

impl IndexedStore {
    /// Create only in a new directory. Does not migrate a legacy installation.
    pub fn create(root: &Path) -> Result<Self, Error> {
        Ok(Self { db: create(root)? })
    }
    /// Audit all logical references before admitting queries or writes.
    pub fn open(root: &Path) -> Result<Self, Error> {
        let db = open(root)?;
        backup::audit_snapshot(&storage(db.begin_read())?)?;
        Ok(Self { db })
    }
    pub fn check_readable(&self) -> Result<(), Error> {
        validate(&self.db)
    }
    pub fn audit(&self) -> Result<ExportSummary, Error> {
        backup::audit_snapshot(&storage(self.db.begin_read())?)
    }
    pub fn snapshot(&self) -> Result<IndexedSnapshot, Error> {
        Ok(IndexedSnapshot {
            read: storage(self.db.begin_read())?,
        })
    }
    pub fn collection(&self, name: &str) -> Result<Collection, Error> {
        self.snapshot()?.collection(name)
    }
    pub fn list_collections(
        &self,
        limit: usize,
        after: Option<&str>,
    ) -> Result<CollectionPage, Error> {
        self.snapshot()?.list_collections(limit, after)
    }
    pub fn count_documents(&self, name: &str) -> Result<DocumentCount, Error> {
        self.snapshot()?.count_documents(name)
    }
    pub fn create_collection(&self, name: &str) -> Result<Collection, IndexedWriteError> {
        collections::create_collection(&self.db, name, |_| Ok(()))
    }
    pub fn drop_collection(&self, collection: &Collection) -> Result<(), IndexedWriteError> {
        collections::drop_collection(&self.db, collection, |_| Ok(()))
    }
    pub fn create_index(
        &self,
        collection: &Collection,
        field: &str,
    ) -> Result<(), IndexedWriteError> {
        lifecycle::maintain(
            &self.db,
            collection,
            lifecycle::Action::Build(field),
            |_| Ok(()),
        )
    }
    pub fn rebuild_index(&self, collection: &Collection) -> Result<(), IndexedWriteError> {
        lifecycle::maintain(&self.db, collection, lifecycle::Action::Rebuild, |_| Ok(()))
    }
    pub fn drop_index(&self, collection: &Collection) -> Result<(), IndexedWriteError> {
        lifecycle::maintain(&self.db, collection, lifecycle::Action::Drop, |_| Ok(()))
    }
    pub fn insert_document(
        &self,
        collection: &Collection,
        input: &PreparedInsert,
    ) -> Result<(), IndexedWriteError> {
        mutations::apply(
            &self.db,
            collection,
            input.id(),
            mutations::Mutation::Insert(input),
            |_| Ok(()),
        )
        .map(|_| ())
    }
    pub fn get_document(
        &self,
        collection: &Collection,
        id: &str,
    ) -> Result<Option<Vec<u8>>, Error> {
        self.snapshot()?.get_document(collection, id)
    }
    pub fn delete_document(
        &self,
        collection: &Collection,
        id: &str,
    ) -> Result<(), IndexedWriteError> {
        self.delete_inner(collection, id, false).map(|_| ())
    }
    pub fn delete_document_captured(
        &self,
        collection: &Collection,
        id: &str,
    ) -> Result<std::sync::Arc<[u8]>, IndexedWriteError> {
        self.delete_inner(collection, id, true)?
            .ok_or(IndexedWriteError::CommitUncertain)
    }
    fn delete_inner(
        &self,
        collection: &Collection,
        id: &str,
        capture: bool,
    ) -> Result<Option<std::sync::Arc<[u8]>>, IndexedWriteError> {
        let stage = || -> Result<_, Error> {
            if !valid_name(id) {
                return Err(Error::InvalidDocument);
            }
            let write = mutations::begin_write(&self.db, collection)?;
            let before = if capture {
                let documents = storage(write.open_table(DOCUMENTS))?;
                let current =
                    storage(documents.get((collection.id, id)))?.ok_or(Error::MissingDocument)?;
                let parsed = DocumentInput::parse(current.value(), DEFAULT_MAX_DOCUMENT_BYTES)
                    .map_err(|_| Error::InvalidLayout)?;
                if parsed.id() != Some(id) {
                    return Err(Error::InvalidLayout);
                }
                Some(std::sync::Arc::from(current.value()))
            } else {
                None
            };
            mutations::stage_in(&write, collection, id, mutations::Mutation::Delete, |_| {
                Ok(())
            })?;
            Ok((write, before))
        };
        let (write, before) = stage().map_err(IndexedWriteError::Aborted)?;
        write
            .commit()
            .map_err(|_| IndexedWriteError::CommitUncertain)?;
        Ok(before)
    }
    pub fn update_document(
        &self,
        collection: &Collection,
        id: &str,
        operation: Operation,
        body: &[u8],
        max_bytes: usize,
    ) -> Result<PreparedWrite, IndexedWriteError> {
        let staged = || -> Result<_, Error> {
            if !(1..=DEFAULT_MAX_DOCUMENT_BYTES).contains(&max_bytes) {
                return Err(Error::InvalidLimit);
            }
            if !valid_name(id) {
                return Err(Error::Write(writes::Error::InvalidRouteId));
            }
            let write = mutations::begin_write(&self.db, collection)?;
            let current = storage(storage(write.open_table(DOCUMENTS))?.get((collection.id, id)))?
                .ok_or(Error::MissingDocument)?
                .value()
                .to_vec();
            let prepared =
                writes::prepare(operation, id, &current, body, max_bytes).map_err(|error| {
                    if matches!(error, writes::Error::InvalidCurrentDocument) {
                        Error::InvalidLayout
                    } else {
                        Error::Write(error)
                    }
                })?;
            let changed = mutations::stage_in(
                &write,
                collection,
                id,
                mutations::Mutation::Prepared(&prepared),
                |_| Ok(()),
            )?;
            Ok((write, prepared, changed))
        };
        let (write, prepared, changed) = staged().map_err(IndexedWriteError::Aborted)?;
        if changed {
            write
                .commit()
                .map_err(|_| IndexedWriteError::CommitUncertain)?;
        } else {
            storage(write.abort()).map_err(IndexedWriteError::Aborted)?;
        }
        Ok(prepared)
    }
    pub fn update_group(
        &self,
        updates: &[Update<'_>],
        max_bytes: usize,
    ) -> Result<Receipt, GroupError> {
        groups::apply(&self.db, updates, max_bytes, None, |_| Ok(())).map(|v| v.receipt)
    }
    pub fn update_group_captured(
        &self,
        updates: &[Update<'_>],
        max_bytes: usize,
        capture: &[bool],
    ) -> Result<CapturedReceipt, GroupError> {
        groups::apply(&self.db, updates, max_bytes, Some(capture), |_| Ok(()))
    }
    pub fn query(
        &self,
        collection: &Collection,
        query: &Query,
    ) -> Result<IndexedQueryResult, IndexedQueryError> {
        self.snapshot()?.query(collection, query)
    }
    pub fn export_snapshot(&self, root: &Path) -> Result<ExportSummary, IndexedCopyError> {
        self.snapshot()?.export(root)
    }
    /// Restore a store, not bootstrap/auth or an installation. Source is read-only.
    pub fn restore_store(source: &Path, target: &Path) -> Result<ExportSummary, IndexedCopyError> {
        backup::restore(source, target, |_| Ok(()))
    }
}

impl IndexedSnapshot {
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
        let store = IndexedStore::create(&path).unwrap();
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
        assert!(matches!(
            IndexedStore::open(&path),
            Err(Error::InvalidLayout)
        ));
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
