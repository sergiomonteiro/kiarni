use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

use kiarni_storage_spike::{DOCUMENTS, STATUS, Store};
use redb::{Database, DatabaseError, ReadableDatabase, ReadableTableMetadata};

struct Worker(Child);

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn abrupt_kill_recovers_committed_data_and_rolls_back_partial_mutations() {
    for mode in ["document", "both", "committed"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("crash.redb");
        {
            let store = Store::open(&path).unwrap();
            store
                .insert_json("orders", br#"{"_id":"confirmed","status":"paid"}"#)
                .unwrap();
        }
        let mut worker = Worker(
            Command::new(env!("CARGO_BIN_EXE_fault-worker"))
                .arg(mode)
                .arg(&path)
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
                .expect("worker never reached fault point"),
            "READY\n"
        );
        reader.join().unwrap();
        assert!(
            matches!(
                Database::open(&path),
                Err(DatabaseError::DatabaseAlreadyOpen)
            ),
            "{mode}"
        );
        worker.0.kill().unwrap();
        let exit = worker.0.wait().unwrap();
        assert!(!exit.success());
        let store = Store::open(&path).unwrap();
        assert!(
            store.get("orders", "confirmed").unwrap().is_some(),
            "{mode}"
        );
        let committed = mode == "committed";
        assert_eq!(
            store.get("orders", "pending").unwrap().is_some(),
            committed,
            "{mode}"
        );
        let read = store.db.begin_read().unwrap();
        let docs = read.open_table(DOCUMENTS).unwrap();
        let index = read.open_table(STATUS).unwrap();
        assert_eq!(docs.len().unwrap(), if committed { 2 } else { 1 });
        assert_eq!(index.len().unwrap(), docs.len().unwrap());
        assert!(
            index
                .get(("orders", "paid", "confirmed"))
                .unwrap()
                .is_some()
        );
        assert_eq!(
            index.get(("orders", "new", "pending")).unwrap().is_some(),
            committed
        );
    }
}
