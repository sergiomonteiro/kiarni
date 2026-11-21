//! Snapshot copy candidate only. No artifact hashes, CLI or overwrite publication.
use super::*;
use redb::{ReadTransaction, WriteTransaction};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Point {
    Batch,
    Audited,
    BeforeManifest,
    Published,
}

#[derive(Debug, PartialEq, Eq)]
enum CopyFailure {
    Incomplete(Error),
    PublicationUncertain,
}
impl From<Error> for CopyFailure {
    fn from(error: Error) -> Self {
        Self::Incomplete(error)
    }
}

fn audit_snapshot(read: &ReadTransaction) -> Result<ExportSummary, Error> {
    validate_snapshot(read)?;
    let collections = storage(read.open_table(COLLECTIONS))?;
    let documents = storage(read.open_table(DOCUMENTS))?;
    let catalog = storage(read.open_table(CATALOG))?;
    let entries = storage(read.open_table(ENTRIES))?;
    let stats = storage(read.open_table(STATS))?;
    let counts = storage(read.open_table(COUNTS))?;
    let mut summary = ExportSummary {
        collections: 0,
        documents: 0,
        document_bytes: 0,
    };
    for row in storage(collections.iter())? {
        let (name, id) = storage(row)?;
        lifecycle::audit_tables(
            &Collection {
                name: name.value().to_owned(),
                id: id.value(),
            },
            &documents,
            &catalog,
            &entries,
            &stats,
            &counts,
        )?;
        summary.collections = summary
            .collections
            .checked_add(1)
            .ok_or(Error::InvalidLayout)?;
    }
    // Collection-local audit cannot detect records owned by a nonexistent ID.
    for row in storage(documents.iter())? {
        let (key, value) = storage(row)?;
        if storage(stats.get(key.value().0))?.is_none() {
            return Err(Error::InvalidLayout);
        }
        summary.documents = summary
            .documents
            .checked_add(1)
            .ok_or(Error::InvalidLayout)?;
        summary.document_bytes = summary
            .document_bytes
            .checked_add(value.value().len() as u64)
            .ok_or(Error::InvalidLayout)?;
    }
    for row in storage(entries.iter())? {
        let (key, _) = storage(row)?;
        if storage(stats.get(key.value().0))?.is_none() {
            return Err(Error::InvalidLayout);
        }
    }
    for row in storage(counts.iter())? {
        let (key, _) = storage(row)?;
        if storage(stats.get(key.value().0))?.is_none() {
            return Err(Error::InvalidLayout);
        }
    }
    Ok(summary)
}

fn begin(db: &Database) -> Result<WriteTransaction, Error> {
    let mut write = storage(db.begin_write())?;
    storage(write.set_durability(Durability::Immediate))?;
    Ok(write)
}

fn export(
    read: &ReadTransaction,
    root: &Path,
    point: impl FnMut(Point) -> Result<(), Error>,
) -> Result<ExportSummary, CopyFailure> {
    let mut publishing = false;
    export_copy(read, root, point, &mut publishing).map_err(|error| {
        if publishing {
            CopyFailure::PublicationUncertain
        } else {
            CopyFailure::Incomplete(error)
        }
    })
}

fn export_copy(
    read: &ReadTransaction,
    root: &Path,
    mut point: impl FnMut(Point) -> Result<(), Error>,
    publishing: &mut bool,
) -> Result<ExportSummary, Error> {
    let expected = audit_snapshot(read)?;
    let target = create_candidate(root, false)?;
    let mut write = begin(&target)?;
    let (mut items, mut bytes) = (0usize, 0usize);
    macro_rules! copy {
        ($table:expr,$size:expr) => {
            for row in storage(storage(read.open_table($table))?.iter())? {
                let (key, value) = storage(row)?;
                let size = ($size)(key.value(), value.value());
                storage(storage(write.open_table($table))?.insert(key.value(), value.value()))?;
                items += 1;
                bytes = bytes.checked_add(size).ok_or(Error::InvalidLayout)?;
                if items >= 256 || bytes >= 4 * 1024 * 1024 {
                    storage(write.commit())?;
                    point(Point::Batch)?;
                    write = begin(&target)?;
                    items = 0;
                    bytes = 0;
                }
            }
        };
    }
    copy!(METADATA, |key: &str, _: u64| key.len() + 8);
    copy!(COLLECTIONS, |key: &str, _: u64| key.len() + 8);
    copy!(DOCUMENTS, |key: (u64, &str), value: &[u8]| key.1.len()
        + 8
        + value.len());
    copy!(CATALOG, |_: u64, value: &[u8]| 8 + value.len());
    copy!(ENTRIES, |key: (u64, &str, &str), _: ()| 8
        + key.1.len()
        + key.2.len());
    copy!(STATS, |_: u64, value: (&str, u64, u64)| 24 + value.0.len());
    copy!(COUNTS, |key: (u64, &str), _: u64| 16 + key.1.len());
    storage(write.commit())?;
    point(Point::Batch)?;
    let actual = audit_snapshot(&storage(target.begin_read())?)?;
    if actual != expected {
        return Err(Error::InvalidLayout);
    }
    point(Point::Audited)?;
    storage(target.close())?;
    io(io(fs::File::open(root.join("data.redb")))?.sync_all())?;
    sync_dir(root)?;
    point(Point::BeforeManifest)?;
    let mut manifest = supported_manifest();
    manifest.version = 2;
    let bytes = serde_json::to_vec(&manifest).map_err(|_| Error::InvalidManifest)?;
    *publishing = true;
    let mut file = create_file(&root.join("format.json"))?;
    io(file.write_all(&bytes))?;
    io(file.sync_all())?;
    sync_dir(root)?;
    sync_dir(
        root.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )?;
    // Error here means the complete copy may exist; never infer rollback.
    point(Point::Published)?;
    Ok(actual)
}

fn restore(
    source: &Path,
    target: &Path,
    point: impl FnMut(Point) -> Result<(), Error>,
) -> Result<ExportSummary, CopyFailure> {
    preflight(source)?;
    let source_path = io(fs::canonicalize(source))?;
    let parent = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    if io(fs::canonicalize(parent))?.starts_with(source_path) {
        return Err(Error::InvalidLayout.into());
    }
    // Read-only open refuses dirty databases instead of recovering the artifact.
    let db = storage(
        Database::builder()
            .set_cache_size(8 * 1024 * 1024)
            .open_read_only(source.join("data.redb")),
    )?;
    export(&storage(db.begin_read())?, target, point)
}

mod tests;
