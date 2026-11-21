use super::*;

pub const COUNT_MAX_DOCUMENTS: u64 = 10_000;
pub const COUNT_MAX_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DocumentCount {
    pub count: u64,
    /// False denotes a lower bound, never an extrapolated estimate.
    pub exact: bool,
}

impl Store {
    /// Bounded primary-entry count from one catalog/document snapshot.
    pub fn count_documents(&self, name: &str) -> Result<DocumentCount, Error> {
        self.telemetry
            .measure(crate::telemetry::Operation::Count, || {
                self.snapshot()?.count_documents(name)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_are_snapshot_reads_of_committed_primary_entries_and_recreated_collections() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("data");
        let store = Store::create(&root).unwrap();
        let collection = store.create_collection("orders").unwrap();
        assert_eq!(
            store.count_documents("orders").unwrap(),
            DocumentCount {
                count: 0,
                exact: true
            }
        );
        store
            .insert_fixture(&collection, br#"{"_id":"one"}"#)
            .unwrap();
        assert_eq!(
            store.insert_fixture(&collection, br#"{"_id":"one"}"#),
            Err(Error::DuplicateId)
        );
        assert_eq!(store.count_documents("orders").unwrap().count, 1);
        let write = store.db.begin_write().unwrap();
        {
            let mut table = write.open_table(DOCUMENTS).unwrap();
            table
                .insert(
                    (collection.id, "pending"),
                    br#"{"_id":"pending"}"#.as_slice(),
                )
                .unwrap();
        }
        assert_eq!(store.count_documents("orders").unwrap().count, 1);
        write.abort().unwrap();
        store.delete_document(&collection, "one").unwrap();
        store.drop_collection(&collection).unwrap();
        assert_eq!(
            store.count_documents("orders"),
            Err(Error::CollectionMissing)
        );
        store.create_collection("orders").unwrap();
        assert_eq!(store.count_documents("orders").unwrap().count, 0);
        drop(store);
        assert_eq!(
            Store::open(&root)
                .unwrap()
                .count_documents("orders")
                .unwrap()
                .count,
            0
        );
    }

    #[test]
    fn document_boundary_is_exact_until_a_lookahead_entry_exists() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::create(&temp.path().join("data")).unwrap();
        let collection = store.create_collection("orders").unwrap();
        // One fixture transaction avoids thousands of durable fsyncs.
        let write = store.db.begin_write().unwrap();
        {
            let mut table = write.open_table(DOCUMENTS).unwrap();
            for i in 0..COUNT_MAX_DOCUMENTS {
                let id = format!("doc_{i:05}");
                let json = format!(r#"{{"_id":"{id}"}}"#);
                table
                    .insert((collection.id, id.as_str()), json.as_bytes())
                    .unwrap();
            }
        }
        write.commit().unwrap();
        assert_eq!(
            store.count_documents("orders").unwrap(),
            DocumentCount {
                count: COUNT_MAX_DOCUMENTS,
                exact: true
            }
        );
        store
            .insert_fixture(&collection, br#"{"_id":"extra"}"#)
            .unwrap();
        assert_eq!(
            store.count_documents("orders").unwrap(),
            DocumentCount {
                count: COUNT_MAX_DOCUMENTS,
                exact: false
            }
        );
        let other = store.create_collection("other").unwrap();
        store.insert_fixture(&other, br#"{"_id":"extra"}"#).unwrap();
        assert_eq!(store.count_documents("other").unwrap().count, 1);
    }

    #[test]
    fn byte_budget_truncates_and_invalid_layout_never_returns_partial_success() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::create(&temp.path().join("data")).unwrap();
        let collection = store.create_collection("orders").unwrap();
        let write = store.db.begin_write().unwrap();
        {
            let mut table = write.open_table(DOCUMENTS).unwrap();
            for i in 0..9 {
                let id = format!("doc_{i}");
                let mut json = format!(r#"{{"_id":"{id}","s":""#).into_bytes();
                json.resize(DEFAULT_MAX_DOCUMENT_BYTES - 2, b'x');
                json.extend_from_slice(b"\"}");
                table
                    .insert((collection.id, id.as_str()), json.as_slice())
                    .unwrap();
            }
        }
        write.commit().unwrap();
        assert_eq!(
            store.count_documents("orders").unwrap(),
            DocumentCount {
                count: 8,
                exact: false
            }
        );
        let write = store.db.begin_write().unwrap();
        write
            .open_table(DOCUMENTS)
            .unwrap()
            .insert(
                (collection.id, "a"),
                vec![b'x'; DEFAULT_MAX_DOCUMENT_BYTES + 1].as_slice(),
            )
            .unwrap();
        write.commit().unwrap();
        assert_eq!(store.count_documents("orders"), Err(Error::InvalidLayout));
        assert_eq!(store.count_documents("bad/name"), Err(Error::InvalidName));
    }
}
