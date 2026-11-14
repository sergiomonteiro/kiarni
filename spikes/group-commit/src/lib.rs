//! Isolated batching experiment. Receipts are available only after transaction completion.

pub mod collector;

use kiarni_core::{DEFAULT_MAX_DOCUMENT_BYTES, DocumentInput};
use kiarni_engine::writes::{self, Operation};
use redb::{Database, Durability, ReadableDatabase, ReadableTable, TableDefinition};
use serde::Serialize;
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::Path,
    time::Instant,
};

const DOCUMENTS: TableDefinition<(u64, &str), &[u8]> = TableDefinition::new("documents");
const COLLECTIONS: TableDefinition<&str, u64> = TableDefinition::new("collections");
const METADATA: TableDefinition<&str, u64> = TableDefinition::new("metadata");
const MARKER: &[u8] = b"kiarni-group-commit-spike-v1\n";
pub const MAX_OPERATIONS: usize = 32;
pub const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Io,
    Storage,
    InvalidFormat,
    InvalidSeed,
    InvalidBatch,
    Injected,
    CommitUncertain,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "group_commit_{self:?}")
    }
}
impl std::error::Error for Error {}

fn storage<T, E>(value: Result<T, E>) -> Result<T, Error> {
    value.map_err(|_| Error::Storage)
}
fn io<T>(value: std::io::Result<T>) -> Result<T, Error> {
    value.map_err(|_| Error::Io)
}

pub struct Store {
    db: Database,
}

pub struct Mutation<'a> {
    pub id: &'a str,
    pub body: &'a [u8],
    pub operation: Operation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    MissingDocument,
    InvalidId,
    Document(writes::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Changed,
    NoOp,
    Rejected(Rejection),
}

#[derive(Default, Serialize)]
pub struct Timings {
    pub begin_ns: u64,
    pub transform_ns: u64,
    pub stage_ns: u64,
    pub commit_ns: u64,
    pub total_ns: u64,
}

pub struct Receipt {
    pub outcomes: Vec<Outcome>,
    pub committed: bool,
    pub timings: Timings,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Checkpoint {
    Staged,
    Committed,
}

fn private_file(path: &Path) -> Result<File, Error> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    io(options.open(path))
}

fn identifier(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

impl Store {
    pub fn create(root: &Path, seed: &[Vec<u8>]) -> Result<Self, Error> {
        let mut directory = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            directory.mode(0o700);
        }
        io(directory.create(root))?;
        if seed.len() > 10000 || seed.iter().map(Vec::len).sum::<usize>() > MAX_BODY_BYTES {
            return Err(Error::InvalidSeed);
        }
        let backend = storage(redb::backends::FileBackend::new(private_file(
            &root.join("data.redb"),
        )?))?;
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
            storage(metadata.insert("next_collection_id", 2))?;
            let mut collections = storage(write.open_table(COLLECTIONS))?;
            storage(collections.insert("bench", 1))?;
            let mut documents = storage(write.open_table(DOCUMENTS))?;
            for json in seed {
                let input = DocumentInput::parse(json, DEFAULT_MAX_DOCUMENT_BYTES)
                    .map_err(|_| Error::InvalidSeed)?;
                let id = input.id().ok_or(Error::InvalidSeed)?;
                if storage(documents.get((1, id)))?.is_some() {
                    return Err(Error::InvalidSeed);
                }
                storage(documents.insert((1, id), json.as_slice()))?;
            }
        }
        storage(write.commit())?;
        let mut marker = private_file(&root.join("experiment"))?;
        io(marker.write_all(MARKER))?;
        io(marker.sync_all())?;
        io(io(File::open(root))?.sync_all())?;
        Ok(Self { db })
    }

    pub fn open(root: &Path) -> Result<Self, Error> {
        use std::io::Read;
        let file = io(File::open(root.join("experiment")))?;
        let mut marker = Vec::new();
        io(file.take(128).read_to_end(&mut marker))?;
        if marker != MARKER {
            return Err(Error::InvalidFormat);
        }
        let db = storage(
            Database::builder()
                .set_cache_size(8 * 1024 * 1024)
                .open(root.join("data.redb")),
        )?;
        let read = storage(db.begin_read())?;
        storage(read.open_table(DOCUMENTS))?;
        storage(read.open_table(COLLECTIONS))?;
        storage(read.open_table(METADATA))?;
        drop(read);
        Ok(Self { db })
    }

    pub fn get(&self, id: &str) -> Result<Option<Vec<u8>>, Error> {
        let read = storage(self.db.begin_read())?;
        let table = storage(read.open_table(DOCUMENTS))?;
        storage(table.get((1, id)))?
            .map(|value| {
                if value.value().len() > DEFAULT_MAX_DOCUMENT_BYTES {
                    return Err(Error::InvalidFormat);
                }
                Ok(value.value().to_vec())
            })
            .transpose()
    }

    pub fn apply(&self, mutations: &[Mutation<'_>]) -> Result<Receipt, Error> {
        self.apply_with_hook(mutations, |_| Ok(()))
    }

    pub fn apply_with_hook(
        &self,
        mutations: &[Mutation<'_>],
        mut hook: impl FnMut(Checkpoint) -> Result<(), Error>,
    ) -> Result<Receipt, Error> {
        let total = Instant::now();
        if mutations.is_empty()
            || mutations.len() > MAX_OPERATIONS
            || mutations
                .iter()
                .map(|mutation| mutation.body.len())
                .sum::<usize>()
                > MAX_BODY_BYTES
        {
            return Err(Error::InvalidBatch);
        }
        let begin = Instant::now();
        let mut write = storage(self.db.begin_write())?;
        storage(write.set_durability(Durability::Immediate))?;
        let mut timings = Timings {
            begin_ns: begin.elapsed().as_nanos() as u64,
            ..Timings::default()
        };
        let mut outcomes = Vec::with_capacity(mutations.len());
        let mut changed = false;
        {
            let mut documents = storage(write.open_table(DOCUMENTS))?;
            for mutation in mutations {
                if !identifier(mutation.id) {
                    outcomes.push(Outcome::Rejected(Rejection::InvalidId));
                    continue;
                }
                let Some(current) = storage(documents.get((1, mutation.id)))? else {
                    outcomes.push(Outcome::Rejected(Rejection::MissingDocument));
                    continue;
                };
                if current.value().len() > DEFAULT_MAX_DOCUMENT_BYTES {
                    return Err(Error::InvalidFormat);
                }
                let transform = Instant::now();
                let candidate = writes::prepare(
                    mutation.operation,
                    mutation.id,
                    current.value(),
                    mutation.body,
                    DEFAULT_MAX_DOCUMENT_BYTES,
                );
                timings.transform_ns += transform.elapsed().as_nanos() as u64;
                drop(current);
                match candidate {
                    Err(error) => outcomes.push(Outcome::Rejected(Rejection::Document(error))),
                    Ok(candidate) if !candidate.changed() => outcomes.push(Outcome::NoOp),
                    Ok(candidate) => {
                        let stage = Instant::now();
                        storage(documents.insert((1, mutation.id), candidate.json()))?;
                        timings.stage_ns += stage.elapsed().as_nanos() as u64;
                        outcomes.push(Outcome::Changed);
                        changed = true;
                    }
                }
            }
        }
        if changed {
            hook(Checkpoint::Staged)?;
            let commit = Instant::now();
            write.commit().map_err(|_| Error::CommitUncertain)?;
            timings.commit_ns = commit.elapsed().as_nanos() as u64;
            hook(Checkpoint::Committed).map_err(|_| Error::CommitUncertain)?;
        } else {
            storage(write.abort())?;
        }
        timings.total_ns = total.elapsed().as_nanos() as u64;
        Ok(Receipt {
            outcomes,
            committed: changed,
            timings,
        })
    }
}
