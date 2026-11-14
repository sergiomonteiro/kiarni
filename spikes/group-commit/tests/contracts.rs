use kiarni_engine::writes::Operation;
use kiarni_group_commit_spike::{Checkpoint, Error, Mutation, Outcome, Store};

fn mutation<'a>(id: &'a str, body: &'a [u8]) -> Mutation<'a> {
    Mutation {
        id,
        body,
        operation: Operation::MergePatch,
    }
}
fn seed() -> Vec<Vec<u8>> {
    vec![
        br#"{"_id":"a","v":0}"#.to_vec(),
        br#"{"_id":"b","v":0}"#.to_vec(),
    ]
}

#[test]
fn ordering_rejections_snapshot_and_restart() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("store");
    let store = Store::create(&root, &seed()).unwrap();
    let receipt = store
        .apply_with_hook(
            &[
                mutation("a", br#"{"v":1}"#),
                mutation("b", b"invalid"),
                mutation("a", br#"{"v":2}"#),
                mutation("missing", b"{}"),
            ],
            |point| {
                if point == Checkpoint::Staged {
                    assert_eq!(store.get("a").unwrap().unwrap(), seed()[0]);
                }
                Ok(())
            },
        )
        .unwrap();
    assert!(receipt.committed);
    assert_eq!(receipt.outcomes[0], Outcome::Changed);
    assert!(matches!(receipt.outcomes[1], Outcome::Rejected(_)));
    assert_eq!(receipt.outcomes[2], Outcome::Changed);
    assert!(matches!(receipt.outcomes[3], Outcome::Rejected(_)));
    drop(store);
    let store = Store::open(&root).unwrap();
    let value: serde_json::Value =
        serde_json::from_slice(&store.get("a").unwrap().unwrap()).unwrap();
    assert_eq!(value["v"], 2);
    assert_eq!(store.get("b").unwrap().unwrap(), seed()[1]);
}

#[test]
fn no_op_bounds_and_faults() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::create(&temp.path().join("store"), &seed()).unwrap();
    assert!(
        !store
            .apply_with_hook(&[mutation("a", b"{}")], |_| panic!("no commit expected"))
            .unwrap()
            .committed
    );
    assert!(matches!(store.apply(&[]), Err(Error::InvalidBatch)));
    let too_many: Vec<_> = (0..33).map(|_| mutation("a", b"{}")).collect();
    assert!(matches!(store.apply(&too_many), Err(Error::InvalidBatch)));
    let large = vec![b' '; 8 * 1024 * 1024 + 1];
    assert!(matches!(
        store.apply(&[mutation("a", &large)]),
        Err(Error::InvalidBatch)
    ));
    assert!(matches!(
        store.apply_with_hook(&[mutation("a", br#"{"v":1}"#)], |_| Err(Error::Injected)),
        Err(Error::Injected)
    ));
    assert_eq!(store.get("a").unwrap().unwrap(), seed()[0]);
    assert!(matches!(
        store.apply_with_hook(&[mutation("a", br#"{"v":1}"#)], |point| {
            if point == Checkpoint::Committed {
                Err(Error::Injected)
            } else {
                Ok(())
            }
        }),
        Err(Error::CommitUncertain)
    ));
    let value: serde_json::Value =
        serde_json::from_slice(&store.get("a").unwrap().unwrap()).unwrap();
    assert_eq!(value["v"], 1);
}

#[test]
fn foreign_store_is_untouched() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("data.redb"), b"foreign").unwrap();
    assert!(Store::open(temp.path()).is_err());
    assert_eq!(
        std::fs::read(temp.path().join("data.redb")).unwrap(),
        b"foreign"
    );
}

#[test]
#[ignore = "child process fixture"]
fn crash_worker() {
    use std::io::Write;
    let root = std::env::var_os("KIARNI_GROUP_CRASH_ROOT").unwrap();
    let target = std::env::var("KIARNI_GROUP_CRASH_POINT").unwrap();
    let store = Store::open(std::path::Path::new(&root)).unwrap();
    store
        .apply_with_hook(
            &[mutation("a", br#"{"v":1}"#), mutation("b", br#"{"v":1}"#)],
            |point| {
                if (point == Checkpoint::Staged && target == "staged")
                    || (point == Checkpoint::Committed && target == "committed")
                {
                    println!("CHECKPOINT");
                    std::io::stdout().flush().unwrap();
                    loop {
                        std::thread::park();
                    }
                }
                Ok(())
            },
        )
        .unwrap();
}

#[test]
fn kill_restart_is_atomic_before_and_after_commit() {
    use std::{
        io::{BufRead, BufReader},
        process::{Command, Stdio},
        sync::mpsc,
        time::Duration,
    };
    for point in ["staged", "committed"] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("store");
        drop(Store::create(&root, &seed()).unwrap());
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "crash_worker", "--nocapture"])
            .env("KIARNI_GROUP_CRASH_ROOT", &root)
            .env("KIARNI_GROUP_CRASH_POINT", point)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::sync_channel(1);
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if line.unwrap().contains("CHECKPOINT") {
                    let _ = tx.send(());
                    break;
                }
            }
        });
        let reached = rx.recv_timeout(Duration::from_secs(10));
        child.kill().unwrap();
        child.wait().unwrap();
        reader.join().unwrap();
        reached.unwrap();
        let store = Store::open(&root).unwrap();
        for id in ["a", "b"] {
            let value: serde_json::Value =
                serde_json::from_slice(&store.get(id).unwrap().unwrap()).unwrap();
            assert_eq!(value["v"], if point == "committed" { 1 } else { 0 });
        }
    }
}
