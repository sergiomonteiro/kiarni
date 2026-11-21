//! Process-local telemetry at Store primitive boundaries, never document labels.
use std::{
    fmt::Write,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
const BOUNDS: [u64; 7] = [1000, 10000, 100000, 1000000, 5000000, 30000000, u64::MAX];
const LABELS: [&str; 7] = ["0.001", "0.01", "0.1", "1", "5", "30", "+Inf"];
const OPERATIONS: [&str; 12] = [
    "health",
    "catalog_list",
    "catalog_lookup",
    "count",
    "query_read",
    "document_read",
    "insert",
    "update",
    "delete",
    "collection_create",
    "collection_drop",
    "update_group",
];
#[derive(Clone, Copy)]
pub(crate) enum Operation {
    Health,
    CatalogList,
    CatalogLookup,
    Count,
    QueryRead,
    DocumentRead,
    Insert,
    Update,
    Delete,
    CollectionCreate,
    CollectionDrop,
    UpdateGroup,
}
fn add(counter: &AtomicU64, amount: u64) {
    let _ = counter.try_update(Ordering::Release, Ordering::Relaxed, |value| {
        Some(value.saturating_add(amount))
    });
}
fn value(counter: &AtomicU64) -> u64 {
    counter.load(Ordering::Acquire)
}
#[derive(Default)]
struct Histogram {
    buckets: [AtomicU64; 7],
    sum_us: AtomicU64,
}
impl Histogram {
    fn observe(&self, elapsed: Duration) {
        for index in (0..7).rev() {
            if index == 6 || elapsed.as_nanos() <= u128::from(BOUNDS[index]) * 1000 {
                add(&self.buckets[index], 1);
            }
        }
        add(
            &self.sum_us,
            u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX),
        );
    }
    fn write(&self, text: &mut String, family: &str, operation: Option<&str>) {
        let buckets = self.buckets.each_ref().map(value);
        for (bound, count) in LABELS.iter().zip(buckets) {
            if let Some(op) = operation {
                let _ = writeln!(
                    text,
                    "{family}_bucket{{operation=\"{op}\",le=\"{bound}\"}} {count}"
                );
            } else {
                let _ = writeln!(text, "{family}_bucket{{le=\"{bound}\"}} {count}");
            }
        }
        if let Some(op) = operation {
            let _ = writeln!(
                text,
                "{family}_sum{{operation=\"{op}\"}} {}\n{family}_count{{operation=\"{op}\"}} {}",
                value(&self.sum_us) as f64 / 1e6,
                buckets[6]
            );
        } else {
            let _ = writeln!(
                text,
                "{family}_sum {}\n{family}_count {}",
                value(&self.sum_us) as f64 / 1e6,
                buckets[6]
            );
        }
    }
}
#[derive(Default)]
pub(crate) struct Metrics {
    operations: [[AtomicU64; 2]; 12],
    duration: [Histogram; 12],
    commits: [AtomicU64; 2],
    commit_duration: Histogram,
    receipts: [AtomicU64; 2],
    items: [AtomicU64; 3],
}
impl Metrics {
    pub fn measure<T, E>(&self, op: Operation, run: impl FnOnce() -> Result<T, E>) -> Result<T, E> {
        let started = Instant::now();
        let result = run();
        let index = op as usize;
        add(&self.operations[index][usize::from(result.is_err())], 1);
        self.duration[index].observe(started.elapsed());
        result
    }
    pub(crate) fn commit(&self, confirmed: bool, elapsed: Duration) {
        add(&self.commits[usize::from(!confirmed)], 1);
        self.commit_duration.observe(elapsed);
    }
    pub fn group(&self, receipt: &crate::updates::Receipt) {
        add(&self.receipts[usize::from(!receipt.committed)], 1);
        for outcome in &receipt.outcomes {
            let index = match outcome {
                Ok(write) if write.changed() => 0,
                Ok(_) => 1,
                Err(_) => 2,
            };
            add(&self.items[index], 1);
        }
    }
    fn prometheus(&self, file_bytes: Option<u64>) -> String {
        let mut text = String::with_capacity(16384);
        text.push_str("# HELP kiarni_storage_operations_total Completed Store primitive calls, not physical IO or HTTP requests.\n# TYPE kiarni_storage_operations_total counter\n");
        for (operation, counters) in OPERATIONS.iter().zip(&self.operations) {
            for (outcome, counter) in ["success", "error"].iter().zip(counters) {
                let _ = writeln!(
                    text,
                    "kiarni_storage_operations_total{{operation=\"{operation}\",outcome=\"{outcome}\"}} {}",
                    value(counter)
                );
            }
        }
        text.push_str("# HELP kiarni_storage_operation_duration_seconds Store primitive elapsed time including failures.\n# TYPE kiarni_storage_operation_duration_seconds histogram\n");
        for (operation, histogram) in OPERATIONS.iter().zip(&self.duration) {
            histogram.write(
                &mut text,
                "kiarni_storage_operation_duration_seconds",
                Some(operation),
            );
        }
        text.push_str("# HELP kiarni_storage_commits_total Engine commit return outcomes; an error is not a known rollback.\n# TYPE kiarni_storage_commits_total counter\n");
        for (outcome, counter) in ["confirmed", "uncertain"].iter().zip(&self.commits) {
            let _ = writeln!(
                text,
                "kiarni_storage_commits_total{{outcome=\"{outcome}\"}} {}",
                value(counter)
            );
        }
        text.push_str("# HELP kiarni_storage_commit_duration_seconds Engine commit call elapsed time, not isolated fsync.\n# TYPE kiarni_storage_commit_duration_seconds histogram\n");
        self.commit_duration
            .write(&mut text, "kiarni_storage_commit_duration_seconds", None);
        text.push_str("# HELP kiarni_storage_group_receipts_total Returned group receipts, not client acknowledgements.\n# TYPE kiarni_storage_group_receipts_total counter\n");
        for (commit, counter) in ["confirmed", "not_required"].iter().zip(&self.receipts) {
            let _ = writeln!(
                text,
                "kiarni_storage_group_receipts_total{{commit=\"{commit}\"}} {}",
                value(counter)
            );
        }
        text.push_str("# HELP kiarni_storage_group_items_total Outcomes in returned group receipts.\n# TYPE kiarni_storage_group_items_total counter\n");
        for (outcome, counter) in ["changed", "noop", "rejected"].iter().zip(&self.items) {
            let _ = writeln!(
                text,
                "kiarni_storage_group_items_total{{outcome=\"{outcome}\"}} {}",
                value(counter)
            );
        }
        text.push_str("# HELP kiarni_storage_file_logical_bytes Logical database file length, not physical blocks or document bytes.\n# TYPE kiarni_storage_file_logical_bytes gauge\n");
        let _ = writeln!(
            text,
            "kiarni_storage_file_logical_bytes {}",
            file_bytes
                .map(|bytes| bytes.to_string())
                .unwrap_or_else(|| "NaN".into())
        );
        text.push_str("# HELP kiarni_storage_file_bytes_available Whether file length metadata was available.\n# TYPE kiarni_storage_file_bytes_available gauge\n");
        let _ = writeln!(
            text,
            "kiarni_storage_file_bytes_available {}",
            u8::from(file_bytes.is_some())
        );
        text
    }
}
impl crate::Store {
    pub fn prometheus_metrics(&self) -> String {
        self.telemetry
            .prometheus(self.telemetry_file.metadata().ok().map(|meta| meta.len()))
    }
    pub(crate) fn commit_transaction(
        &self,
        transaction: redb::WriteTransaction,
    ) -> Result<(), redb::CommitError> {
        let started = Instant::now();
        let result = transaction.commit();
        self.telemetry.commit(result.is_ok(), started.elapsed());
        result
    }
}
pub const STATE_BYTES: usize = std::mem::size_of::<Metrics>();
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saturated_commit_outcomes_and_missing_file_metadata_are_explicit() {
        let metrics = Metrics::default();
        metrics.commits[0].store(u64::MAX, Ordering::Relaxed);
        metrics.commit(true, Duration::from_millis(1));
        metrics.commit(false, Duration::from_millis(2));
        assert_eq!(value(&metrics.commits[0]), u64::MAX);
        assert_eq!(value(&metrics.commits[1]), 1);
        let text = metrics.prometheus(None);
        assert!(text.contains("file_logical_bytes NaN\n"));
        assert!(text.contains("file_bytes_available 0\n"));
        assert!(text.contains("commit_duration_seconds_count 2\n"));
        for counters in &metrics.operations {
            for counter in counters {
                counter.store(u64::MAX, Ordering::Relaxed);
            }
        }
        for histogram in &metrics.duration {
            for bucket in &histogram.buckets {
                bucket.store(u64::MAX, Ordering::Relaxed);
            }
            histogram.sum_us.store(u64::MAX, Ordering::Relaxed);
        }
        assert!(metrics.prometheus(Some(u64::MAX)).len() < 32768);
    }
}

#[cfg(test)]
mod store_tests {
    use crate::{Error, Store, updates::Update};
    use kiarni_engine::writes::Operation as WriteOperation;
    fn sample(text: &str, name: &str) -> u64 {
        text.lines()
            .find_map(|line| line.strip_prefix(&format!("{name} ")))
            .unwrap()
            .parse()
            .unwrap()
    }
    #[test]
    fn store_primitives_noops_group_items_and_restart_have_explicit_counts() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("data");
        let store = Store::create(&root).unwrap();
        let collection = store.create_collection("private_collection").unwrap();
        assert_eq!(
            store.create_collection("private_collection"),
            Err(Error::CollectionExists)
        );
        store
            .insert_fixture(
                &collection,
                br#"{"_id":"private_doc","v":0,"s":"PRIVATE_PAYLOAD"}"#,
            )
            .unwrap();
        assert!(
            store
                .get_document(&collection, "private_doc")
                .unwrap()
                .is_some()
        );
        assert!(store.get_document(&collection, "absent").unwrap().is_none());
        store.list_collections(1, None).unwrap();
        store.collection("private_collection").unwrap();
        store.count_documents("private_collection").unwrap();
        store.query_documents(&collection, None).unwrap();
        store.check_readable().unwrap();
        store
            .update_document(
                &collection,
                "private_doc",
                WriteOperation::MergePatch,
                br#"{"v":0}"#,
                1024,
            )
            .unwrap();
        let updates = [
            Update {
                collection: "private_collection",
                id: "private_doc",
                operation: WriteOperation::MergePatch,
                body: br#"{"v":1}"#,
            },
            Update {
                collection: "private_collection",
                id: "private_doc",
                operation: WriteOperation::MergePatch,
                body: br#"{"v":1}"#,
            },
            Update {
                collection: "private_collection",
                id: "absent",
                operation: WriteOperation::MergePatch,
                body: br#"{"v":1}"#,
            },
        ];
        assert!(store.update_group(&updates, 1024).unwrap().committed);
        assert!(!store.update_group(&updates[..1], 1024).unwrap().committed);
        let text = store.prometheus_metrics();
        assert_eq!(
            sample(&text, "kiarni_storage_commits_total{outcome=\"confirmed\"}"),
            3
        );
        assert_eq!(
            sample(
                &text,
                "kiarni_storage_group_receipts_total{commit=\"confirmed\"}"
            ),
            1
        );
        assert_eq!(
            sample(
                &text,
                "kiarni_storage_group_receipts_total{commit=\"not_required\"}"
            ),
            1
        );
        for (name, count) in [("changed", 1), ("noop", 2), ("rejected", 1)] {
            assert_eq!(
                sample(
                    &text,
                    &format!("kiarni_storage_group_items_total{{outcome=\"{name}\"}}")
                ),
                count
            );
        }
        assert_eq!(
            sample(
                &text,
                "kiarni_storage_operations_total{operation=\"document_read\",outcome=\"success\"}"
            ),
            2
        );
        assert_eq!(
            sample(
                &text,
                "kiarni_storage_operations_total{operation=\"collection_create\",outcome=\"error\"}"
            ),
            1
        );
        assert_eq!(sample(&text, "kiarni_storage_file_bytes_available"), 1);
        assert!(sample(&text, "kiarni_storage_file_logical_bytes") > 0);
        for secret in ["private_collection", "private_doc", "PRIVATE_PAYLOAD"] {
            assert!(!text.contains(secret));
        }
        // Scrape is not a writer: it can run while another write transaction is held.
        let write = store.db.begin_write().unwrap();
        assert!(!store.prometheus_metrics().is_empty());
        write.abort().unwrap();
        store.delete_document(&collection, "private_doc").unwrap();
        assert_eq!(
            store.delete_document(&collection, "private_doc"),
            Err(Error::MissingDocument)
        );
        store.drop_collection(&collection).unwrap();
        assert_eq!(
            sample(
                &store.prometheus_metrics(),
                "kiarni_storage_commits_total{outcome=\"confirmed\"}"
            ),
            5
        );
        drop(store);
        let reopened = Store::open(&root).unwrap();
        assert_eq!(
            sample(
                &reopened.prometheus_metrics(),
                "kiarni_storage_commits_total{outcome=\"confirmed\"}"
            ),
            0
        );
    }
}
