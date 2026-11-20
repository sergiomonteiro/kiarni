//! ADR 0034 candidate. Not compiled into the server or exposed through Store.
use super::*;
mod lifecycle;
mod mutations;

const CATALOG: TableDefinition<u64, &[u8]> = TableDefinition::new("index_catalog");
const ENTRIES: TableDefinition<(u64, &str, &str), ()> = TableDefinition::new("secondary_entries");
const STATS: TableDefinition<u64, (&str, u64, u64)> = TableDefinition::new("collection_stats");
const COUNTS: TableDefinition<(u64, &str), u64> = TableDefinition::new("secondary_counts");
const HEADER: [u8; 4] = [1, 1, 1, 128]; // revision, string kind, ready, UTF-8 byte limit

fn encode_definition(field: &str) -> Result<Vec<u8>, Error> {
    if !valid_name(field) || field == "_id" {
        return Err(Error::InvalidLayout);
    }
    let mut bytes = HEADER.to_vec();
    bytes.push(field.len() as u8);
    bytes.extend_from_slice(field.as_bytes());
    Ok(bytes)
}

fn decode_definition(bytes: &[u8]) -> Result<&str, Error> {
    if bytes.len() < 6 || bytes[..4] != HEADER || bytes.len() != 5 + bytes[4] as usize {
        return Err(Error::InvalidLayout);
    }
    let field = std::str::from_utf8(&bytes[5..]).map_err(|_| Error::InvalidLayout)?;
    if !valid_name(field) || field == "_id" {
        return Err(Error::InvalidLayout);
    }
    Ok(field)
}

/// Preflight always precedes the redb open, including recovery.
fn preflight(root: &Path) -> Result<(), Error> {
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
    if manifest.magic != "kiarni" || manifest.version != 2 || manifest.engine != "redb-4.5.0" {
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

fn open(root: &Path) -> Result<Database, Error> {
    preflight(root)?;
    let db = storage(
        Database::builder()
            .set_cache_size(8 * 1024 * 1024)
            .open(root.join("data.redb")),
    )?;
    validate(&db)?;
    Ok(db)
}

fn create(root: &Path) -> Result<Database, Error> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    io(builder.create(root))?;
    let path = root.join("data.redb");
    let backend = storage(redb::backends::FileBackend::new(create_file(&path)?))?;
    let db = storage(
        Database::builder()
            .set_cache_size(8 * 1024 * 1024)
            .create_with_backend(backend),
    )?;
    let mut write = storage(db.begin_write())?;
    storage(write.set_durability(Durability::Immediate))?;
    {
        let mut metadata = storage(write.open_table(METADATA))?;
        storage(metadata.insert("format_version", 2))?;
        storage(metadata.insert("next_collection_id", 1))?;
        storage(write.open_table(COLLECTIONS))?;
        storage(write.open_table(DOCUMENTS))?;
        storage(write.open_table(CATALOG))?;
        storage(write.open_table(ENTRIES))?;
        storage(write.open_table(STATS))?;
        storage(write.open_table(COUNTS))?;
    }
    storage(write.commit())?;
    storage(db.close())?;
    io(io(fs::File::open(&path))?.sync_all())?;
    sync_dir(root)?;
    let mut manifest = supported_manifest();
    manifest.version = 2;
    let mut file = create_file(&root.join("format.json"))?;
    io(file.write_all(&serde_json::to_vec(&manifest).map_err(|_| Error::InvalidManifest)?))?;
    io(file.sync_all())?;
    sync_dir(root)?;
    sync_dir(
        root.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )?;
    open(root)
}

/// Structural validation only: does not prove document/index equivalence.
fn validate(db: &Database) -> Result<(), Error> {
    let read = storage(db.begin_read())?;
    let mut names = storage(read.list_tables())?
        .take(8)
        .map(|t| t.name().to_owned())
        .collect::<Vec<_>>();
    names.sort();
    if names
        != [
            "collection_stats",
            "collections",
            "documents",
            "index_catalog",
            "metadata",
            "secondary_counts",
            "secondary_entries",
        ]
        || storage(read.list_multimap_tables())?.next().is_some()
    {
        return Err(Error::InvalidLayout);
    }
    let metadata = read
        .open_table(METADATA)
        .map_err(|_| Error::InvalidLayout)?;
    let next = storage(metadata.get("next_collection_id"))?.map(|v| v.value());
    if storage(metadata.len())? != 2
        || storage(metadata.get("format_version"))?.map(|v| v.value()) != Some(2)
        || next.is_none_or(|n| n == 0)
    {
        return Err(Error::InvalidLayout);
    }
    let next = next.ok_or(Error::InvalidLayout)?;
    let collections = read
        .open_table(COLLECTIONS)
        .map_err(|_| Error::InvalidLayout)?;
    read.open_table(DOCUMENTS)
        .map_err(|_| Error::InvalidLayout)?;
    let catalog = read.open_table(CATALOG).map_err(|_| Error::InvalidLayout)?;
    let stats = read.open_table(STATS).map_err(|_| Error::InvalidLayout)?;
    read.open_table(ENTRIES).map_err(|_| Error::InvalidLayout)?;
    read.open_table(COUNTS).map_err(|_| Error::InvalidLayout)?;
    // Reverse ownership avoids a growing in-memory ID set or quadratic scans.
    if storage(stats.len())? != storage(collections.len())? {
        return Err(Error::InvalidLayout);
    }
    for row in storage(collections.iter())? {
        let (name, id) = storage(row)?;
        if !valid_name(name.value()) || id.value() == 0 || id.value() >= next {
            return Err(Error::InvalidLayout);
        }
        let owner = storage(stats.get(id.value()))?.ok_or(Error::InvalidLayout)?;
        if owner.value().0 != name.value() {
            return Err(Error::InvalidLayout);
        }
    }
    for row in storage(stats.iter())? {
        let (id, stats) = storage(row)?;
        let owner = storage(collections.get(stats.value().0))?.ok_or(Error::InvalidLayout)?;
        if owner.value() != id.value() {
            return Err(Error::InvalidLayout);
        }
    }
    for row in storage(catalog.iter())? {
        let (id, definition) = storage(row)?;
        if storage(stats.get(id.value()))?.is_none() {
            return Err(Error::InvalidLayout);
        }
        decode_definition(definition.value())?;
    }
    Ok(())
}

#[test]
fn definition_has_fixed_bytes_and_strict_validation() {
    assert_eq!(
        encode_definition("status").unwrap(),
        include_bytes!("../tests/fixtures/index-status-v1.bin")
    );
    assert_eq!(
        decode_definition(include_bytes!("../tests/fixtures/index-status-v1.bin")).unwrap(),
        "status"
    );
    for field in ["", "_id", "nested.path", "é", &"x".repeat(129)] {
        assert!(encode_definition(field).is_err());
    }
    let largest = encode_definition(&"x".repeat(128)).unwrap();
    assert_eq!(decode_definition(&largest).unwrap().len(), 128);
    let fixture = encode_definition("status").unwrap();
    for end in 0..fixture.len() {
        assert!(decode_definition(&fixture[..end]).is_err());
    }
    for offset in 0..5 {
        let mut bytes = fixture.clone();
        bytes[offset] = 0;
        assert!(decode_definition(&bytes).is_err());
    }
    let mut extra = fixture;
    extra.push(0);
    assert!(decode_definition(&extra).is_err());
}

#[test]
fn new_destination_roundtrip_and_old_store_refusal() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("candidate");
    let db = create(&root).unwrap();
    let write = db.begin_write().unwrap();
    write
        .open_table(COLLECTIONS)
        .unwrap()
        .insert("tasks", 1)
        .unwrap();
    write
        .open_table(METADATA)
        .unwrap()
        .insert("next_collection_id", 2)
        .unwrap();
    write
        .open_table(STATS)
        .unwrap()
        .insert(1, ("tasks", 0, 0))
        .unwrap();
    let definition = encode_definition("status").unwrap();
    write
        .open_table(CATALOG)
        .unwrap()
        .insert(1, definition.as_slice())
        .unwrap();
    write.commit().unwrap();
    drop(db);
    drop(open(&root).unwrap());
    let before = fs::read(root.join("data.redb")).unwrap();
    assert!(matches!(Store::open(&root), Err(Error::UnsupportedFormat)));
    assert!(create(&root).is_err());
    assert_eq!(fs::read(root.join("data.redb")).unwrap(), before);
    let old = temp.path().join("old");
    drop(Store::create(&old).unwrap());
    let before = fs::read(old.join("data.redb")).unwrap();
    assert!(matches!(open(&old), Err(Error::UnsupportedFormat)));
    assert_eq!(fs::read(old.join("data.redb")).unwrap(), before);
    drop(Store::open(&old).unwrap());
}

#[test]
fn missing_tables_are_not_repaired_and_unknown_layout_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("candidate");
    let db = create(&root).unwrap();
    let write = db.begin_write().unwrap();
    write.delete_table(ENTRIES).unwrap();
    write.commit().unwrap();
    drop(db);
    assert!(matches!(open(&root), Err(Error::InvalidLayout)));
    let db = Database::open(root.join("data.redb")).unwrap();
    assert!(
        !db.begin_read()
            .unwrap()
            .list_tables()
            .unwrap()
            .any(|t| t.name() == "secondary_entries")
    );
}

#[test]
fn corrupt_catalog_and_metadata_are_rejected() {
    for case in 0..12 {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("candidate");
        let db = create(&root).unwrap();
        let write = db.begin_write().unwrap();
        match case {
            0 => {
                write
                    .open_table(METADATA)
                    .unwrap()
                    .insert("format_version", 1)
                    .unwrap();
            }
            1 => {
                write
                    .open_table(METADATA)
                    .unwrap()
                    .insert("next_collection_id", 0)
                    .unwrap();
            }
            2 => {
                write
                    .open_table(CATALOG)
                    .unwrap()
                    .insert(1, encode_definition("status").unwrap().as_slice())
                    .unwrap();
            }
            3 => {
                write
                    .open_table(STATS)
                    .unwrap()
                    .insert(1, ("tasks", 0, 0))
                    .unwrap();
            }
            4 => {
                write
                    .open_table(COLLECTIONS)
                    .unwrap()
                    .insert("tasks", 1)
                    .unwrap();
            }
            5 => {
                write
                    .open_table(TableDefinition::<u64, u64>::new("unexpected"))
                    .unwrap();
            }
            6 => {
                write
                    .open_table(METADATA)
                    .unwrap()
                    .insert("next_collection_id", 2)
                    .unwrap();
                write
                    .open_table(COLLECTIONS)
                    .unwrap()
                    .insert("tasks", 1)
                    .unwrap();
                write
                    .open_table(STATS)
                    .unwrap()
                    .insert(1, ("tasks", 0, 0))
                    .unwrap();
                write
                    .open_table(CATALOG)
                    .unwrap()
                    .insert(1, &[9u8; 6][..])
                    .unwrap();
            }
            7 => {
                write.delete_table(COUNTS).unwrap();
                write
                    .open_table(TableDefinition::<u64, u64>::new("secondary_counts"))
                    .unwrap();
            }
            8 => {
                write
                    .open_table(METADATA)
                    .unwrap()
                    .insert("unexpected", 1)
                    .unwrap();
            }
            9..=11 => {
                write
                    .open_table(METADATA)
                    .unwrap()
                    .insert("next_collection_id", 2)
                    .unwrap();
                write
                    .open_table(COLLECTIONS)
                    .unwrap()
                    .insert("tasks", 1)
                    .unwrap();
                write
                    .open_table(STATS)
                    .unwrap()
                    .insert(1, ("other", 0, 0))
                    .unwrap();
                if case == 10 {
                    write
                        .open_table(COLLECTIONS)
                        .unwrap()
                        .insert("other", 1)
                        .unwrap();
                    write
                        .open_table(STATS)
                        .unwrap()
                        .insert(2, ("tasks", 0, 0))
                        .unwrap();
                }
                if case == 11 {
                    write
                        .open_table(STATS)
                        .unwrap()
                        .insert(1, ("tasks", 0, 0))
                        .unwrap();
                    write
                        .open_table(COLLECTIONS)
                        .unwrap()
                        .insert("other", 1)
                        .unwrap();
                    write
                        .open_table(STATS)
                        .unwrap()
                        .insert(2, ("other", 0, 0))
                        .unwrap();
                }
            }
            _ => unreachable!(),
        }
        write.commit().unwrap();
        drop(db);
        assert!(
            matches!(open(&root), Err(Error::InvalidLayout)),
            "case {case}"
        );
    }
}

#[test]
fn partial_initialization_is_never_reused() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("candidate");
    drop(create(&root).unwrap());
    fs::remove_file(root.join("format.json")).unwrap();
    let before = fs::read(root.join("data.redb")).unwrap();
    assert!(matches!(open(&root), Err(Error::Io)));
    assert!(create(&root).is_err());
    assert!(!root.join("format.json").exists());
    assert_eq!(fs::read(root.join("data.redb")).unwrap(), before);
}

#[test]
fn invalid_manifest_is_rejected_before_touching_database() {
    for bytes in [
        b"{\"magic\":\"kiarni\",\"version\":3,\"engine\":\"redb-4.5.0\"}".as_slice(),
        b"{\"magic\":\"kiarni\",\"version\":2,\"version\":2,\"engine\":\"redb-4.5.0\"}",
        b"{\"magic\":\"kiarni\",\"version\":2,\"engine\":\"redb-4.5.0\",\"extra\":true}",
        b"{",
        &[b' '; 1025],
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("candidate");
        drop(create(&root).unwrap());
        // Deliberately not a redb file: preflight must report the manifest error.
        fs::write(root.join("data.redb"), b"untouched").unwrap();
        fs::write(root.join("format.json"), bytes).unwrap();
        assert!(matches!(
            open(&root),
            Err(Error::InvalidManifest | Error::UnsupportedFormat)
        ));
        assert_eq!(fs::read(root.join("data.redb")).unwrap(), b"untouched");
    }
}
