//! Disposable M0 experiment. No public storage contract or stable file format.

pub mod secondary;

use std::{fmt, fs::OpenOptions, path::Path};

use kiarni_core::{DEFAULT_MAX_DOCUMENT_BYTES, DocumentInput, PreparedInsert};
use redb::{
    Database, Durability, ReadTransaction, ReadableDatabase, ReadableTable, TableDefinition,
};

pub const DOCUMENTS: TableDefinition<(&str, &str), &[u8]> = TableDefinition::new("documents");
// Synthetic secondary index for a string field only; not a query contract.
pub const STATUS: TableDefinition<(&str, &str, &str), ()> = TableDefinition::new("status");
pub const MAX_ID_ATTEMPTS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Storage,
    InvalidDocument,
    DuplicateId,
    MissingDocument,
    IdGenerationExhausted,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Storage => "storage_error",
            Self::InvalidDocument => "invalid_document",
            Self::DuplicateId => "duplicate_id",
            Self::MissingDocument => "missing_document",
            Self::IdGenerationExhausted => "id_generation_exhausted",
        })
    }
}

impl std::error::Error for Error {}

fn storage<T, E>(result: Result<T, E>) -> Result<T, Error> {
    result.map_err(|_| Error::Storage)
}

pub struct Store {
    // Exposed solely for experiment assertions/fault injection.
    pub db: Database,
}

fn status(json: &[u8]) -> Result<String, Error> {
    let input = DocumentInput::parse(json, DEFAULT_MAX_DOCUMENT_BYTES)
        .map_err(|_| Error::InvalidDocument)?;
    let raw = input.fields().get("status").ok_or(Error::InvalidDocument)?;
    serde_json::from_str(raw.get()).map_err(|_| Error::InvalidDocument)
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, Error> {
        let db = storage(
            Database::builder()
                .set_cache_size(8 * 1024 * 1024)
                .create(path),
        )?;
        let mut write = storage(db.begin_write())?;
        storage(write.set_durability(Durability::Immediate))?;
        storage(write.open_table(DOCUMENTS))?;
        storage(write.open_table(STATUS))?;
        storage(write.commit())?;
        Ok(Self { db })
    }

    pub fn insert_json(&self, collection: &str, json: &[u8]) -> Result<String, Error> {
        let input = DocumentInput::parse(json, DEFAULT_MAX_DOCUMENT_BYTES)
            .map_err(|_| Error::InvalidDocument)?;
        if input.id().is_some() {
            let candidate = input.prepare_insert().map_err(|_| Error::InvalidDocument)?;
            self.insert(collection, &candidate)?;
            return Ok(candidate.id().into());
        }
        self.retry_generated(collection, || {
            let candidate = DocumentInput::parse(json, DEFAULT_MAX_DOCUMENT_BYTES)
                .and_then(DocumentInput::prepare_insert)
                .map_err(|_| Error::InvalidDocument)?;
            Ok((candidate.id().to_owned(), candidate.json().to_vec()))
        })
    }

    fn retry_generated(
        &self,
        collection: &str,
        mut generate: impl FnMut() -> Result<(String, Vec<u8>), Error>,
    ) -> Result<String, Error> {
        for _ in 0..MAX_ID_ATTEMPTS {
            let (id, json) = generate()?;
            match self.put(collection, &id, &json, false) {
                Ok(()) => return Ok(id),
                Err(Error::DuplicateId) => continue,
                Err(error) => return Err(error),
            }
        }
        Err(Error::IdGenerationExhausted)
    }

    pub fn insert(&self, collection: &str, candidate: &PreparedInsert) -> Result<(), Error> {
        self.put(collection, candidate.id(), candidate.json(), false)
    }

    pub fn replace(&self, collection: &str, candidate: &PreparedInsert) -> Result<(), Error> {
        self.put(collection, candidate.id(), candidate.json(), true)
    }

    fn put(&self, collection: &str, id: &str, json: &[u8], replace: bool) -> Result<(), Error> {
        let new_status = status(json)?;
        let mut write = storage(self.db.begin_write())?;
        storage(write.set_durability(Durability::Immediate))?;
        {
            let mut documents = storage(write.open_table(DOCUMENTS))?;
            let mut index = storage(write.open_table(STATUS))?;
            let old = storage(documents.get((collection, id)))?.map(|v| v.value().to_vec());
            match (old.as_deref(), replace) {
                (Some(_), false) => return Err(Error::DuplicateId),
                (None, true) => return Err(Error::MissingDocument),
                (Some(old), true) => {
                    let old_status = status(old)?;
                    storage(index.remove((collection, old_status.as_str(), id)))?;
                }
                (None, false) => {}
            }
            storage(documents.insert((collection, id), json))?;
            storage(index.insert((collection, new_status.as_str(), id), ()))?;
        }
        storage(write.commit())
    }

    pub fn delete(&self, collection: &str, id: &str) -> Result<(), Error> {
        let mut write = storage(self.db.begin_write())?;
        storage(write.set_durability(Durability::Immediate))?;
        {
            let mut documents = storage(write.open_table(DOCUMENTS))?;
            let mut index = storage(write.open_table(STATUS))?;
            let old = storage(documents.get((collection, id)))?
                .ok_or(Error::MissingDocument)?
                .value()
                .to_vec();
            let old_status = status(&old)?;
            storage(index.remove((collection, old_status.as_str(), id)))?;
            storage(documents.remove((collection, id)))?;
        }
        storage(write.commit())
    }

    pub fn get(&self, collection: &str, id: &str) -> Result<Option<Vec<u8>>, Error> {
        let read = storage(self.db.begin_read())?;
        let table = storage(read.open_table(DOCUMENTS))?;
        Ok(storage(table.get((collection, id)))?.map(|v| v.value().to_vec()))
    }
}

/// Logical backup of both experimental tables from one read snapshot.
/// Destination must be new. A failure may leave a partial file to discard.
pub fn export_snapshot(snapshot: &ReadTransaction, destination: &Path) -> Result<(), Error> {
    let file = storage(
        OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(destination),
    )?;
    let backend = storage(redb::backends::FileBackend::new(file))?;
    let target = storage(
        Database::builder()
            .set_cache_size(8 * 1024 * 1024)
            .create_with_backend(backend),
    )?;
    let mut write = storage(target.begin_write())?;
    storage(write.set_durability(Durability::Immediate))?;
    {
        let source_docs = storage(snapshot.open_table(DOCUMENTS))?;
        let source_index = storage(snapshot.open_table(STATUS))?;
        let mut target_docs = storage(write.open_table(DOCUMENTS))?;
        let mut target_index = storage(write.open_table(STATUS))?;
        for entry in storage(source_docs.iter())? {
            let (key, value) = storage(entry)?;
            storage(target_docs.insert(key.value(), value.value()))?;
        }
        for entry in storage(source_index.iter())? {
            let (key, _) = storage(entry)?;
            storage(target_index.insert(key.value(), ()))?;
        }
    }
    storage(write.commit())
}

impl From<Error> for kiarni_http::Code {
    fn from(error: Error) -> Self {
        use Error as E;
        match error {
            E::Storage => Self::StorageUnavailable,
            E::InvalidDocument => Self::InvalidDocument,
            E::DuplicateId => Self::DuplicateId,
            E::MissingDocument => Self::DocumentNotFound,
            E::IdGenerationExhausted => Self::IdGenerationExhausted,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_collisions_retry_but_explicit_conflicts_do_not_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("test.redb")).unwrap();
        let first = br#"{"_id":"collision","status":"old"}"#;
        store.insert_json("orders", first).unwrap();
        let mut attempts = 0;
        let id = store
            .retry_generated("orders", || {
                attempts += 1;
                let id = if attempts == 1 { "collision" } else { "free" };
                Ok((
                    id.into(),
                    format!(r#"{{"_id":"{id}","status":"new"}}"#).into_bytes(),
                ))
            })
            .unwrap();
        assert_eq!(id, "free");
        assert_eq!(attempts, 2);
        assert_eq!(store.insert_json("orders", first), Err(Error::DuplicateId));
        assert_eq!(store.get("orders", "collision").unwrap().unwrap(), first);
    }

    #[test]
    fn generated_collision_attempts_are_bounded_and_errors_stop_immediately() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("test.redb")).unwrap();
        let json = br#"{"_id":"collision","status":"old"}"#;
        store.insert_json("orders", json).unwrap();
        let mut attempts = 0;
        let result = store.retry_generated("orders", || {
            attempts += 1;
            Ok(("collision".into(), json.to_vec()))
        });
        assert_eq!(result, Err(Error::IdGenerationExhausted));
        assert_eq!(attempts, MAX_ID_ATTEMPTS);
        let mut calls = 0;
        let result = store.retry_generated("orders", || {
            calls += 1;
            Err(Error::InvalidDocument)
        });
        assert_eq!(result, Err(Error::InvalidDocument));
        assert_eq!(calls, 1);
    }
}
