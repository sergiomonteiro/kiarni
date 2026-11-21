//! The single production Store: transactional indexed storage and telemetry.
use super::*;
use kiarni_engine::writes::{self, Operation, PreparedWrite};

fn write_error(error: IndexedWriteError) -> Error {
    match error {
        IndexedWriteError::Aborted(error) => error,
        IndexedWriteError::CommitUncertain => Error::Storage,
    }
}
fn group_error(error: IndexedWriteError) -> crate::updates::GroupError {
    match error {
        IndexedWriteError::Aborted(error) => crate::updates::GroupError::Aborted(error),
        IndexedWriteError::CommitUncertain => crate::updates::GroupError::CommitUncertain,
    }
}

impl Store {
    pub fn create(root: &Path) -> Result<Self, Error> {
        let db = create(root)?;
        Self::from_database(db, root)
    }
    pub fn open(root: &Path) -> Result<Self, Error> {
        let db = open(root)?;
        backup::audit_snapshot(&storage(db.begin_read())?)?;
        Self::from_database(db, root)
    }
    fn from_database(db: Database, root: &Path) -> Result<Self, Error> {
        Ok(Self {
            db,
            telemetry: crate::telemetry::Metrics::default(),
            telemetry_file: io(fs::File::open(root.join("data.redb")))?,
        })
    }
    pub fn check_readable(&self) -> Result<(), Error> {
        self.telemetry
            .measure(crate::telemetry::Operation::Health, || validate(&self.db))
    }
    pub fn audit(&self) -> Result<ExportSummary, Error> {
        backup::audit_snapshot(&storage(self.db.begin_read())?)
    }
    pub fn snapshot(&self) -> Result<IndexedSnapshot, Error> {
        IndexedSnapshot::new(&self.db)
    }
    pub fn collection(&self, name: &str) -> Result<Collection, Error> {
        self.telemetry
            .measure(crate::telemetry::Operation::CatalogLookup, || {
                self.snapshot()?.collection(name)
            })
    }
    pub fn list_collections(
        &self,
        limit: usize,
        after: Option<&str>,
    ) -> Result<CollectionPage, Error> {
        self.telemetry
            .measure(crate::telemetry::Operation::CatalogList, || {
                self.snapshot()?.list_collections(limit, after)
            })
    }
    pub fn get_document(
        &self,
        collection: &Collection,
        id: &str,
    ) -> Result<Option<Vec<u8>>, Error> {
        self.telemetry
            .measure(crate::telemetry::Operation::DocumentRead, || {
                self.snapshot()?.get_document(collection, id)
            })
    }
    pub fn create_collection(&self, name: &str) -> Result<Collection, Error> {
        self.telemetry
            .measure(crate::telemetry::Operation::CollectionCreate, || {
                collections::create_collection(Some(&self.telemetry), &self.db, name, |_| Ok(()))
                    .map_err(write_error)
            })
    }
    pub fn drop_collection(&self, collection: &Collection) -> Result<(), Error> {
        self.telemetry
            .measure(crate::telemetry::Operation::CollectionDrop, || {
                collections::drop_collection(
                    Some(&self.telemetry),
                    &self.db,
                    collection,
                    |_| Ok(()),
                )
                .map_err(write_error)
            })
    }
    pub fn create_index(
        &self,
        collection: &Collection,
        field: &str,
    ) -> Result<(), IndexedWriteError> {
        lifecycle::maintain(
            Some(&self.telemetry),
            &self.db,
            collection,
            lifecycle::Action::Build(field),
            |_| Ok(()),
        )
    }
    pub fn rebuild_index(&self, collection: &Collection) -> Result<(), IndexedWriteError> {
        lifecycle::maintain(
            Some(&self.telemetry),
            &self.db,
            collection,
            lifecycle::Action::Rebuild,
            |_| Ok(()),
        )
    }
    pub fn drop_index(&self, collection: &Collection) -> Result<(), IndexedWriteError> {
        lifecycle::maintain(
            Some(&self.telemetry),
            &self.db,
            collection,
            lifecycle::Action::Drop,
            |_| Ok(()),
        )
    }
    pub fn insert_document(
        &self,
        collection: &Collection,
        input: &PreparedInsert,
    ) -> Result<(), Error> {
        self.telemetry
            .measure(crate::telemetry::Operation::Insert, || {
                mutations::apply(
                    Some(&self.telemetry),
                    &self.db,
                    collection,
                    input.id(),
                    mutations::Mutation::Insert(input),
                    |_| Ok(()),
                )
                .map(|_| ())
                .map_err(write_error)
            })
    }
    /// Explicit-ID fixture helper; follows the same indexed mutation path.
    pub fn insert_fixture(&self, collection: &Collection, json: &[u8]) -> Result<(), Error> {
        let input = DocumentInput::parse(json, DEFAULT_MAX_DOCUMENT_BYTES)
            .map_err(|_| Error::InvalidDocument)?;
        if input.id().is_none() {
            return Err(Error::InvalidDocument);
        }
        self.telemetry
            .measure(crate::telemetry::Operation::Insert, || {
                mutations::apply(
                    Some(&self.telemetry),
                    &self.db,
                    collection,
                    input.id().ok_or(Error::InvalidDocument)?,
                    mutations::Mutation::RawInsert(json),
                    |_| Ok(()),
                )
                .map(|_| ())
                .map_err(write_error)
            })
    }
    pub fn delete_document(&self, collection: &Collection, id: &str) -> Result<(), Error> {
        self.delete_inner(collection, id, false)
            .map(|_| ())
            .map_err(write_error)
    }
    pub fn delete_document_captured(
        &self,
        collection: &Collection,
        id: &str,
    ) -> Result<std::sync::Arc<[u8]>, crate::updates::GroupError> {
        self.delete_inner(collection, id, true)
            .map_err(group_error)?
            .ok_or(crate::updates::GroupError::CommitUncertain)
    }
    fn delete_inner(
        &self,
        collection: &Collection,
        id: &str,
        capture: bool,
    ) -> Result<Option<std::sync::Arc<[u8]>>, IndexedWriteError> {
        self.telemetry
            .measure(crate::telemetry::Operation::Delete, || {
                let stage = || -> Result<_, Error> {
                    if !valid_name(id) {
                        return Err(Error::InvalidDocument);
                    }
                    let write = mutations::begin_write(&self.db, collection)?;
                    let before = if capture {
                        let documents = storage(write.open_table(DOCUMENTS))?;
                        let current = storage(documents.get((collection.id, id)))?
                            .ok_or(Error::MissingDocument)?;
                        let parsed =
                            DocumentInput::parse(current.value(), DEFAULT_MAX_DOCUMENT_BYTES)
                                .map_err(|_| Error::InvalidLayout)?;
                        if parsed.id() != Some(id) {
                            return Err(Error::InvalidLayout);
                        }
                        Some(std::sync::Arc::from(current.value()))
                    } else {
                        None
                    };
                    mutations::stage_in(
                        &write,
                        collection,
                        id,
                        mutations::Mutation::Delete,
                        |_| Ok(()),
                    )?;
                    Ok((write, before))
                };
                let (write, before) = stage().map_err(IndexedWriteError::Aborted)?;
                self.commit_transaction(write)
                    .map_err(|_| IndexedWriteError::CommitUncertain)?;
                Ok(before)
            })
    }
    pub fn update_document(
        &self,
        collection: &Collection,
        id: &str,
        operation: Operation,
        body: &[u8],
        max_bytes: usize,
    ) -> Result<PreparedWrite, Error> {
        self.update_before_commit(collection, id, operation, body, max_bytes, || Ok(()))
    }
    pub(crate) fn update_before_commit(
        &self,
        collection: &Collection,
        id: &str,
        operation: Operation,
        body: &[u8],
        max_bytes: usize,
        before_commit: impl FnOnce() -> Result<(), Error>,
    ) -> Result<PreparedWrite, Error> {
        self.telemetry
            .measure(crate::telemetry::Operation::Update, || {
                if !(1..=DEFAULT_MAX_DOCUMENT_BYTES).contains(&max_bytes) {
                    return Err(Error::InvalidLimit);
                }
                if !valid_name(id) {
                    return Err(Error::Write(writes::Error::InvalidRouteId));
                }
                #[cfg(all(unix, feature = "profiling"))]
                let begin = crate::profiling::BEGIN.start();
                let write = mutations::begin_write(&self.db, collection)?;
                #[cfg(all(unix, feature = "profiling"))]
                drop(begin);
                #[cfg(all(unix, feature = "profiling"))]
                let lookup = crate::profiling::LOOKUP.start();
                let current =
                    storage(storage(write.open_table(DOCUMENTS))?.get((collection.id, id)))?
                        .ok_or(Error::MissingDocument)?
                        .value()
                        .to_vec();
                #[cfg(all(unix, feature = "profiling"))]
                drop(lookup);
                #[cfg(all(unix, feature = "profiling"))]
                let transform = crate::profiling::TRANSFORM.start();
                let prepared =
                    writes::prepare(operation, id, &current, body, max_bytes).map_err(|error| {
                        if matches!(error, writes::Error::InvalidCurrentDocument) {
                            Error::InvalidLayout
                        } else {
                            Error::Write(error)
                        }
                    })?;
                #[cfg(all(unix, feature = "profiling"))]
                drop(transform);
                #[cfg(all(unix, feature = "profiling"))]
                let stage = crate::profiling::STAGE.start();
                let changed = mutations::stage_in(
                    &write,
                    collection,
                    id,
                    mutations::Mutation::Prepared(&prepared),
                    |_| Ok(()),
                )?;
                #[cfg(all(unix, feature = "profiling"))]
                drop(stage);
                if changed {
                    before_commit()?;
                    #[cfg(all(unix, feature = "profiling"))]
                    let _commit = crate::profiling::COMMIT.start();
                    storage(self.commit_transaction(write))?;
                } else {
                    storage(write.abort())?;
                }
                Ok(prepared)
            })
    }
    pub fn query(
        &self,
        collection: &Collection,
        query: &kiarni_engine::query::Query,
    ) -> Result<IndexedQueryResult, IndexedQueryError> {
        self.snapshot()?.query(collection, query)
    }
    pub fn restore_store(source: &Path, target: &Path) -> Result<ExportSummary, IndexedCopyError> {
        backup::restore(source, target, |_| Ok(()))
    }
}
