//! Snapshot export contract and regression tests.
use super::*;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ExportSummary {
    pub collections: u64,
    pub documents: u64,
    pub document_bytes: u64,
}
#[cfg(test)]
fn export(read: &redb::ReadTransaction, root: &Path) -> Result<ExportSummary, Error> {
    format_v2::backup::export(read, root, |_| Ok(())).map_err(copy_error)
}
fn copy_error(error: IndexedCopyError) -> Error {
    match error {
        IndexedCopyError::Incomplete(error) => error,
        IndexedCopyError::PublicationUncertain => Error::Storage,
    }
}
impl Store {
    pub fn export_snapshot(&self, root: &Path) -> Result<ExportSummary, Error> {
        format_v2::backup::export(&storage(self.db.begin_read())?, root, |_| Ok(()))
            .map_err(copy_error)
    }
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
        {
            let docs = write.open_table(DOCUMENTS).unwrap();
            let mut count = 0u64;
            let mut bytes = 0u64;
            for row in docs.range((orders.id, "")..(orders.id + 1, "")).unwrap() {
                count += 1;
                bytes += row.unwrap().1.value().len() as u64;
            }
            write
                .open_table(crate::format_v2::STATS)
                .unwrap()
                .insert(orders.id, ("orders", count, bytes))
                .unwrap();
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
