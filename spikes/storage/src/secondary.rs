//! RF-04 evaluation only. Never open/migrate a product store with this layout.
use kiarni_core::{DEFAULT_MAX_DOCUMENT_BYTES, DocumentInput};
use kiarni_engine::{query::Query, writes};
use redb::{
    Database, Durability, ReadTransaction, ReadableDatabase, ReadableTable, TableDefinition,
};
use std::{collections::BTreeSet, fmt, fs::OpenOptions, path::Path};

pub const DOCUMENTS: TableDefinition<(&str, &str), &[u8]> =
    TableDefinition::new("evaluation_documents");
pub const INDEX: TableDefinition<(&str, &str, &str), ()> =
    TableDefinition::new("evaluation_status");
const META: TableDefinition<&str, u64> = TableDefinition::new("evaluation_meta");
pub const MAX_VALUE_BYTES: usize = 128;
const MAX_DOCUMENTS: usize = 10000;
const MAX_BYTES: usize = 8 * 1024 * 1024;
const END_ID: &str = "\u{7f}"; // Greater than every permitted ASCII identifier byte.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Storage,
    Input,
    Duplicate,
    Missing,
    Layout,
    Limit,
    Injected,
    Query(kiarni_engine::query::Error),
    Write(writes::Error),
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "secondary_evaluation_{self:?}")
    }
}
impl std::error::Error for Error {}
fn db<T, E>(value: Result<T, E>) -> Result<T, Error> {
    value.map_err(|_| Error::Storage)
}
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}
fn status(json: &[u8]) -> Result<Option<String>, Error> {
    let input = DocumentInput::parse(json, DEFAULT_MAX_DOCUMENT_BYTES).map_err(|_| Error::Input)?;
    Ok(input
        .fields()
        .get("status")
        .and_then(|raw| serde_json::from_str::<String>(raw.get()).ok())
        .filter(|value| value.len() <= MAX_VALUE_BYTES))
}

#[derive(Clone, Copy)]
pub enum Mutation<'a> {
    Insert(&'a [u8]),
    Replace(&'a [u8]),
    Patch(&'a [u8]),
    Delete,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Checkpoint {
    Document,
    OldIndexRemoved,
    NewIndexWritten,
    BeforeCommit,
    Committed,
}

pub struct Store {
    pub db: Database,
    indexed: bool,
}
pub struct Snapshot {
    read: ReadTransaction,
    indexed: bool,
}
pub struct Output {
    pub documents: Vec<Vec<u8>>,
    pub plan: &'static str,
    pub examined: usize,
    pub document_bytes: usize,
}

impl Store {
    pub fn create(path: &Path, indexed: bool) -> Result<Self, Error> {
        // Refuse existing destinations, including actual product files.
        let file = db(OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path))?;
        let database = db(Database::builder()
            .set_cache_size(MAX_BYTES)
            .create_with_backend(db(redb::backends::FileBackend::new(file))?))?;
        let mut write = db(database.begin_write())?;
        db(write.set_durability(Durability::Immediate))?;
        db(write.open_table(DOCUMENTS))?;
        if indexed {
            db(write.open_table(INDEX))?;
        }
        {
            let mut meta = db(write.open_table(META))?;
            db(meta.insert("evaluation_version", 1))?;
            db(meta.insert("indexed", u64::from(indexed)))?;
        }
        db(write.commit())?;
        Ok(Self {
            db: database,
            indexed,
        })
    }
    pub fn open(path: &Path) -> Result<Self, Error> {
        let database = db(Database::builder().set_cache_size(MAX_BYTES).open(path))?;
        let indexed = {
            let read = db(database.begin_read())?;
            let meta = db(read.open_table(META))?;
            if db(meta.get("evaluation_version"))?.map(|v| v.value()) != Some(1) {
                return Err(Error::Layout);
            }
            let indexed = match db(meta.get("indexed"))?.map(|v| v.value()) {
                Some(0) => false,
                Some(1) => true,
                _ => return Err(Error::Layout),
            };
            db(read.open_table(DOCUMENTS))?;
            if indexed {
                db(read.open_table(INDEX))?;
            }
            indexed
        };
        Ok(Self {
            db: database,
            indexed,
        })
    }
    pub fn snapshot(&self) -> Result<Snapshot, Error> {
        Ok(Snapshot {
            read: db(self.db.begin_read())?,
            indexed: self.indexed,
        })
    }
    pub fn mutate(&self, collection: &str, id: &str, change: Mutation<'_>) -> Result<bool, Error> {
        self.mutate_with_hook(collection, id, change, &mut |_| Ok(()))
    }
    pub fn mutate_with_hook(
        &self,
        collection: &str,
        id: &str,
        change: Mutation<'_>,
        hook: &mut impl FnMut(Checkpoint) -> Result<(), Error>,
    ) -> Result<bool, Error> {
        if !identifier(collection) || !identifier(id) {
            return Err(Error::Input);
        }
        let mut write = db(self.db.begin_write())?;
        db(write.set_durability(Durability::Immediate))?;
        {
            let mut documents = db(write.open_table(DOCUMENTS))?;
            let old = db(documents.get((collection, id)))?.map(|v| v.value().to_vec());
            let new = match change {
                Mutation::Insert(json) => {
                    let input = DocumentInput::parse(json, DEFAULT_MAX_DOCUMENT_BYTES)
                        .map_err(|_| Error::Input)?;
                    if input.id() != Some(id) {
                        return Err(Error::Input);
                    }
                    if old.is_some() {
                        return Err(Error::Duplicate);
                    }
                    Some(json.to_vec())
                }
                Mutation::Replace(json) | Mutation::Patch(json) => {
                    let old = old.as_deref().ok_or(Error::Missing)?;
                    let operation = if matches!(change, Mutation::Replace(_)) {
                        writes::Operation::Replace
                    } else {
                        writes::Operation::MergePatch
                    };
                    let candidate =
                        writes::prepare(operation, id, old, json, DEFAULT_MAX_DOCUMENT_BYTES)
                            .map_err(Error::Write)?;
                    if !candidate.changed() {
                        return Ok(false);
                    }
                    Some(candidate.json().to_vec())
                }
                Mutation::Delete => {
                    if old.is_none() {
                        return Err(Error::Missing);
                    }
                    None
                }
            };
            if let Some(json) = &new {
                db(documents.insert((collection, id), json.as_slice()))?;
            } else {
                db(documents.remove((collection, id)))?;
            }
            hook(Checkpoint::Document)?;
            if self.indexed {
                let mut index = db(write.open_table(INDEX))?;
                let old_value = old.as_deref().map(status).transpose()?.flatten();
                let new_value = new.as_deref().map(status).transpose()?.flatten();
                if old_value != new_value {
                    if let Some(value) = &old_value
                        && db(index.remove((collection, value.as_str(), id)))?.is_none()
                    {
                        return Err(Error::Layout);
                    }
                } else if let Some(value) = &old_value
                    && db(index.get((collection, value.as_str(), id)))?.is_none()
                {
                    return Err(Error::Layout);
                }
                hook(Checkpoint::OldIndexRemoved)?;
                if old_value != new_value
                    && let Some(value) = &new_value
                    && db(index.insert((collection, value.as_str(), id), ()))?.is_some()
                {
                    return Err(Error::Layout);
                }
                hook(Checkpoint::NewIndexWritten)?;
            }
        }
        hook(Checkpoint::BeforeCommit)?;
        db(write.commit())?;
        hook(Checkpoint::Committed)?;
        Ok(true)
    }
    pub fn query(&self, query: &Query, force_scan: bool) -> Result<Output, Error> {
        self.snapshot()?.query(query, force_scan)
    }
}

fn literal(query: &Query) -> Result<Option<String>, Error> {
    // Only inspect a validated AST; do not infer conjunction/range semantics.
    let value: serde_json::Value = serde_json::from_slice(&query.to_json().map_err(Error::Query)?)
        .map_err(|_| Error::Input)?;
    let filter = &value["filter"];
    Ok((filter["op"] == "eq" && filter["field"] == "status")
        .then(|| filter["value"].as_str())
        .flatten()
        .filter(|value| value.len() <= MAX_VALUE_BYTES)
        .map(str::to_owned))
}
impl Snapshot {
    pub fn query(&self, query: &Query, force_scan: bool) -> Result<Output, Error> {
        let documents = db(self.read.open_table(DOCUMENTS))?;
        let collection = query.collection();
        let value = literal(query)?;
        let mut inputs = Vec::new();
        let mut bytes = 0;
        let mut add = |id: &str, json: &[u8]| -> Result<(), Error> {
            if inputs.len() >= MAX_DOCUMENTS || bytes + json.len() > MAX_BYTES {
                return Err(Error::Limit);
            }
            let input = DocumentInput::parse(json, DEFAULT_MAX_DOCUMENT_BYTES)
                .map_err(|_| Error::Layout)?;
            if input.id() != Some(id) {
                return Err(Error::Layout);
            }
            bytes += json.len();
            inputs.push(input);
            Ok(())
        };
        let plan = if !force_scan && query.id().is_some() {
            let id = query.id().ok_or(Error::Input)?;
            if let Some(json) = db(documents.get((collection, id)))? {
                add(id, json.value())?;
            }
            "id"
        } else if self.indexed && !force_scan && value.is_some() {
            let value = value.as_deref().ok_or(Error::Input)?;
            let index = db(self.read.open_table(INDEX))?;
            for entry in db(index.range((collection, value, "")..=(collection, value, END_ID)))? {
                let (key, _) = db(entry)?;
                let (_, _, id) = key.value();
                let json = db(documents.get((collection, id)))?.ok_or(Error::Layout)?;
                if status(json.value())?.as_deref() != Some(value) {
                    return Err(Error::Layout);
                }
                add(id, json.value())?;
            }
            "status_eq"
        } else {
            for entry in db(documents.range((collection, "")..=(collection, END_ID)))? {
                let (key, json) = db(entry)?;
                add(key.value().1, json.value())?;
            }
            "scan"
        };
        let examined = inputs.len();
        Ok(Output {
            documents: query.evaluate_documents(&inputs).map_err(Error::Query)?,
            plan,
            examined,
            document_bytes: bytes,
        })
    }
    /// Full bounded consistency audit, not an O(1) query guarantee.
    pub fn audit(&self) -> Result<usize, Error> {
        let documents = db(self.read.open_table(DOCUMENTS))?;
        let mut expected = BTreeSet::new();
        let mut count = 0;
        let mut bytes = 0;
        for entry in db(documents.iter())? {
            let (key, json) = db(entry)?;
            count += 1;
            bytes += json.value().len();
            if count > MAX_DOCUMENTS || bytes > MAX_BYTES {
                return Err(Error::Limit);
            }
            let (collection, id) = key.value();
            let input = DocumentInput::parse(json.value(), DEFAULT_MAX_DOCUMENT_BYTES)
                .map_err(|_| Error::Layout)?;
            if !identifier(collection) || input.id() != Some(id) {
                return Err(Error::Layout);
            }
            if let Some(value) = status(json.value())? {
                expected.insert((collection.to_owned(), value, id.to_owned()));
            }
        }
        if self.indexed {
            let index = db(self.read.open_table(INDEX))?;
            let mut actual = BTreeSet::new();
            for (i, entry) in db(index.iter())?.enumerate() {
                if i >= MAX_DOCUMENTS {
                    return Err(Error::Limit);
                }
                let (key, _) = db(entry)?;
                let (collection, value, id) = key.value();
                actual.insert((collection.to_owned(), value.to_owned(), id.to_owned()));
            }
            if actual != expected {
                return Err(Error::Layout);
            }
        }
        Ok(count)
    }
}
