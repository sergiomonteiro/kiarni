//! Snapshot export, independent of CLI, authentication and artifact publication.
use super::*;
use redb::{ReadTransaction, WriteTransaction};
const IDS: TableDefinition<u64, &str> = TableDefinition::new("backup_collection_ids");
const BATCH_ITEMS: usize = 256;
const BATCH_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ExportSummary {
    pub collections: u64,
    pub documents: u64,
    pub document_bytes: u64,
}
#[derive(Default)]
struct Batch {
    items: usize,
    bytes: usize,
}
fn begin(db: &Database) -> Result<WriteTransaction, Error> {
    let mut write = storage(db.begin_write())?;
    storage(write.set_durability(Durability::Immediate))?;
    Ok(write)
}
impl Batch {
    fn advance(
        &mut self,
        db: &Database,
        write: WriteTransaction,
        bytes: usize,
    ) -> Result<WriteTransaction, Error> {
        self.items += 1;
        self.bytes += bytes;
        if self.items >= BATCH_ITEMS || self.bytes >= BATCH_BYTES {
            storage(write.commit())?;
            self.items = 0;
            self.bytes = 0;
            begin(db)
        } else {
            Ok(write)
        }
    }
}
impl Store {
    /// Export exactly one snapshot into a new private store directory.
    /// A failed export can leave a partial destination; callers must stage it.
    pub fn export_snapshot(&self, root: &Path) -> Result<ExportSummary, Error> {
        let read = storage(self.db.begin_read())?;
        export(&read, root)
    }
}
fn export(read: &ReadTransaction, root: &Path) -> Result<ExportSummary, Error> {
    let metadata = storage(read.open_table(METADATA))?;
    let next = storage(metadata.get("next_collection_id"))?
        .ok_or(Error::InvalidLayout)?
        .value();
    if storage(metadata.len())? != 2
        || next == 0
        || storage(metadata.get("format_version"))?.map(|v| v.value()) != Some(1)
    {
        return Err(Error::InvalidLayout);
    }
    let collections = storage(read.open_table(COLLECTIONS))?;
    let documents = storage(read.open_table(DOCUMENTS))?;
    let target = Store::create(root)?;
    let mut write = begin(&target.db)?;
    {
        let mut out = storage(write.open_table(METADATA))?;
        storage(out.insert("next_collection_id", next))?;
        storage(write.open_table(IDS))?;
    }
    let mut batch = Batch::default();
    let mut summary = ExportSummary {
        collections: 0,
        documents: 0,
        document_bytes: 0,
    };
    for entry in storage(collections.iter())? {
        let (name, id) = storage(entry)?;
        let (name, id) = (name.value(), id.value());
        if !valid_name(name) || id == 0 || id >= next {
            return Err(Error::InvalidLayout);
        }
        {
            let mut ids = storage(write.open_table(IDS))?;
            if storage(ids.insert(id, name))?.is_some() {
                return Err(Error::InvalidLayout);
            }
            storage(storage(write.open_table(COLLECTIONS))?.insert(name, id))?;
        }
        summary.collections = summary
            .collections
            .checked_add(1)
            .ok_or(Error::InvalidLayout)?;
        write = batch.advance(&target.db, write, name.len() + 8)?;
    }
    for entry in storage(documents.iter())? {
        let (key, bytes) = storage(entry)?;
        let (cid, id) = key.value();
        let bytes = bytes.value();
        if !valid_name(id) || storage(storage(write.open_table(IDS))?.get(cid))?.is_none() {
            return Err(Error::InvalidLayout);
        }
        let document = DocumentInput::parse(bytes, DEFAULT_MAX_DOCUMENT_BYTES)
            .map_err(|_| Error::InvalidLayout)?;
        if document.id() != Some(id) {
            return Err(Error::InvalidLayout);
        }
        storage(storage(write.open_table(DOCUMENTS))?.insert((cid, id), bytes))?;
        summary.documents = summary
            .documents
            .checked_add(1)
            .ok_or(Error::InvalidLayout)?;
        summary.document_bytes = summary
            .document_bytes
            .checked_add(bytes.len() as u64)
            .ok_or(Error::InvalidLayout)?;
        write = batch.advance(&target.db, write, bytes.len() + id.len() + 8)?;
    }
    storage(write.delete_table(IDS))?;
    storage(write.commit())?;
    storage(target.db.close())?;
    drop(target.telemetry_file);
    io(io(fs::File::open(root.join("data.redb")))?.sync_all())?;
    io(io(fs::File::open(root.join("format.json")))?.sync_all())?;
    sync_dir(root)?;
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn multiple_batches_preserve_one_snapshot_raw_json_catalog_and_next_id() {
        let temp = tempfile::tempdir().unwrap();
        let source = Store::create(&temp.path().join("source")).unwrap();
        let orders = source.create_collection("orders").unwrap();
        let removed = source.create_collection("removed").unwrap();
        source.drop_collection(&removed).unwrap();
        source.create_collection("empty").unwrap();
        let original = br#"{"_id":"d000","n":9007199254740993,"price":1.234567890123456789}"#;
        let write = source.db.begin_write().unwrap();
        {
            let mut docs = write.open_table(DOCUMENTS).unwrap();
            docs.insert((orders.id, "d000"), original.as_slice())
                .unwrap();
            for n in 1..600 {
                let id = format!("d{n:03}");
                let bytes = format!(r#"{{"_id":"{id}","v":{n}}}"#);
                docs.insert((orders.id, id.as_str()), bytes.as_bytes())
                    .unwrap();
            }
            for n in 0..6 {
                let id = format!("large{n}");
                let bytes = format!(r#"{{"_id":"{id}","v":"{}"}}"#, "x".repeat(900_000));
                docs.insert((orders.id, id.as_str()), bytes.as_bytes())
                    .unwrap();
            }
        }
        write.commit().unwrap();
        let snapshot = source.db.begin_read().unwrap();
        source.create_collection("later").unwrap();
        source
            .update_document(
                &orders,
                "d000",
                kiarni_engine::writes::Operation::MergePatch,
                br#"{"v":true}"#,
                DEFAULT_MAX_DOCUMENT_BYTES,
            )
            .unwrap();
        let out = temp.path().join("export");
        let summary = export(&snapshot, &out).unwrap();
        assert_eq!(summary.collections, 2);
        assert_eq!(summary.documents, 606);
        assert!(summary.document_bytes > 5_000_000);
        let target = Store::open(&out).unwrap();
        assert_eq!(
            target.list_collections(1000, None).unwrap().collections,
            ["empty", "orders"]
        );
        assert_eq!(
            target
                .get_document(&target.collection("orders").unwrap(), "d000")
                .unwrap()
                .unwrap(),
            original
        );
        assert_eq!(target.create_collection("next").unwrap().id, 4);
        assert!(target.collection("later").is_err());
        assert_eq!(source.collection("later").unwrap().id, 4);
    }
    #[test]
    fn invalid_references_duplicate_ids_and_documents_are_not_exported() {
        for case in 0..6 {
            let temp = tempfile::tempdir().unwrap();
            let source = Store::create(&temp.path().join("source")).unwrap();
            let collection = source.create_collection("orders").unwrap();
            let write = source.db.begin_write().unwrap();
            match case {
                0 => {
                    write
                        .open_table(COLLECTIONS)
                        .unwrap()
                        .insert("duplicate", collection.id)
                        .unwrap();
                }
                1 => {
                    write
                        .open_table(COLLECTIONS)
                        .unwrap()
                        .insert("bad/name", collection.id)
                        .unwrap();
                }
                2 => {
                    write
                        .open_table(METADATA)
                        .unwrap()
                        .insert("next_collection_id", 1)
                        .unwrap();
                }
                3 => {
                    write
                        .open_table(DOCUMENTS)
                        .unwrap()
                        .insert((99, "x"), br#"{"_id":"x"}"#.as_slice())
                        .unwrap();
                }
                4 => {
                    write
                        .open_table(DOCUMENTS)
                        .unwrap()
                        .insert((collection.id, "x"), br#"{"_id":"y"}"#.as_slice())
                        .unwrap();
                }
                _ => {
                    write
                        .open_table(DOCUMENTS)
                        .unwrap()
                        .insert((collection.id, "x"), b"not JSON".as_slice())
                        .unwrap();
                }
            }
            write.commit().unwrap();
            assert_eq!(
                source.export_snapshot(&temp.path().join("out")),
                Err(Error::InvalidLayout)
            );
        }
    }
    #[test]
    fn empty_store_exports_and_existing_target_is_never_reused() {
        let temp = tempfile::tempdir().unwrap();
        let source = Store::create(&temp.path().join("source")).unwrap();
        let out = temp.path().join("out");
        assert_eq!(source.export_snapshot(&out).unwrap().documents, 0);
        assert!(source.export_snapshot(&out).is_err());
        Store::open(&out).unwrap().check_readable().unwrap();
    }
}
