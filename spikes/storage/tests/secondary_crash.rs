use kiarni_engine::query::Query;
use kiarni_storage_spike::secondary::{Mutation, Store};
use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};
struct Worker(Child);
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn count(store: &Store, value: &str, scan: bool) -> usize {
    let query=Query::parse(serde_json::to_string(&serde_json::json!({"from":"orders","filter":{"op":"eq","field":"status","value":value},"limit":10})).unwrap().as_bytes()).unwrap();
    store.query(&query, scan).unwrap().documents.len()
}
#[test]
fn killed_update_and_delete_never_publish_partial_index_maintenance() {
    for indexed in [false, true] {
        for operation in ["patch", "delete"] {
            let points = if indexed {
                vec![
                    "document",
                    "old_index",
                    "new_index",
                    "before_commit",
                    "committed",
                ]
            } else {
                vec!["document", "before_commit", "committed"]
            };
            for point in points {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join("db");
                {
                    let store = Store::create(&path, indexed).unwrap();
                    store
                        .mutate(
                            "orders",
                            "target",
                            Mutation::Insert(br#"{"_id":"target","status":"open"}"#),
                        )
                        .unwrap();
                    store
                        .mutate(
                            "other",
                            "target",
                            Mutation::Insert(br#"{"_id":"target","status":"untouched"}"#),
                        )
                        .unwrap();
                }
                let mut worker = Worker(
                    Command::new(env!("CARGO_BIN_EXE_secondary-fault-worker"))
                        .arg(&path)
                        .args([operation, point])
                        .stdin(Stdio::piped())
                        .stdout(Stdio::piped())
                        .spawn()
                        .unwrap(),
                );
                let stdout = worker.0.stdout.take().unwrap();
                let (tx, rx) = mpsc::sync_channel(1);
                let reader = thread::spawn(move || {
                    let mut line = String::new();
                    BufReader::new(stdout).read_line(&mut line).unwrap();
                    let _ = tx.send(line);
                });
                assert_eq!(
                    rx.recv_timeout(Duration::from_secs(15))
                        .expect("worker missed checkpoint"),
                    "READY\n"
                );
                reader.join().unwrap();
                worker.0.kill().unwrap();
                assert!(!worker.0.wait().unwrap().success());
                let store = Store::open(&path).unwrap();
                let committed = point == "committed";
                assert_eq!(
                    store.snapshot().unwrap().audit().unwrap(),
                    if committed && operation == "delete" {
                        1
                    } else {
                        2
                    }
                );
                for scan in [false, true] {
                    assert_eq!(count(&store, "open", scan), usize::from(!committed));
                    assert_eq!(
                        count(&store, "paid", scan),
                        usize::from(committed && operation == "patch")
                    );
                }
            }
        }
    }
}
