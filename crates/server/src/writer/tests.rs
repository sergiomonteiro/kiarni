use super::*;
mod feeds;

fn fixture() -> (tempfile::TempDir, Arc<Store>) {
    let temp = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::create(&temp.path().join("store")).unwrap());
    let collection = store.create_collection("orders").unwrap();
    store
        .insert_fixture(&collection, br#"{"_id":"doc","v":0}"#)
        .unwrap();
    (temp, store)
}
fn policy() -> Policy {
    Policy {
        max_requests: 64,
        max_document_bytes: 1024 * 1024,
        max_subscribers: 8,
        feed_capacity: 4,
    }
}
fn patch(value: usize) -> Command {
    Command::Update {
        collection: "orders".into(),
        id: "doc".into(),
        operation: Operation::MergePatch,
        body: format!(r#"{{"v":{value}}}"#).into_bytes(),
    }
}
fn json(reply: &Reply) -> serde_json::Value {
    match reply.outcome().unwrap() {
        Outcome::Updated(candidate) => serde_json::from_slice(candidate.json()).unwrap(),
        Outcome::Inserted(candidate) => serde_json::from_slice(candidate.json()).unwrap(),
        _ => panic!("expected document result"),
    }
}
fn stored(store: &Store) -> serde_json::Value {
    let collection = store.collection("orders").unwrap();
    serde_json::from_slice(&store.get_document(&collection, "doc").unwrap().unwrap()).unwrap()
}

struct Held {
    _temp: tempfile::TempDir,
    store: Arc<Store>,
    coordinator: Coordinator,
    client: Client,
    release: mpsc::SyncSender<()>,
    first: Ticket,
}
fn held(policy: Policy) -> Held {
    let (temp, store) = fixture();
    let (entered_tx, entered_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::sync_channel(1);
    let mut first = true;
    let coordinator = Coordinator::start_with_hook(
        Arc::clone(&store),
        policy,
        Arc::new(AtomicBool::new(false)),
        move || {
            if first {
                first = false;
                entered_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            }
        },
    )
    .unwrap();
    let client = coordinator.client();
    let ticket = client
        .submit(Command::CreateCollection {
            name: "initial_barrier".into(),
        })
        .unwrap();
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    Held {
        _temp: temp,
        store,
        coordinator,
        client,
        release: release_tx,
        first: ticket,
    }
}

#[test]
fn updates_share_group_but_all_other_mutations_are_fifo_barriers() {
    let h = held(policy());
    let one = h.client.submit(patch(1)).unwrap();
    let two = h.client.submit(patch(2)).unwrap();
    let delete = h
        .client
        .submit(Command::Delete {
            collection: "orders".into(),
            id: "doc".into(),
        })
        .unwrap();
    let missing = h.client.submit(patch(3)).unwrap();
    let insert = h
        .client
        .submit(Command::Insert {
            collection: "orders".into(),
            body: br#"{"_id":"doc","v":4}"#.to_vec(),
        })
        .unwrap();
    let update = h.client.submit(patch(5)).unwrap();
    let not_empty = h
        .client
        .submit(Command::DropCollection {
            name: "orders".into(),
        })
        .unwrap();
    let delete_again = h
        .client
        .submit(Command::Delete {
            collection: "orders".into(),
            id: "doc".into(),
        })
        .unwrap();
    let drop_collection = h
        .client
        .submit(Command::DropCollection {
            name: "orders".into(),
        })
        .unwrap();
    let recreate = h
        .client
        .submit(Command::CreateCollection {
            name: "orders".into(),
        })
        .unwrap();
    let insert_again = h
        .client
        .submit(Command::Insert {
            collection: "orders".into(),
            body: br#"{"_id":"doc","v":6}"#.to_vec(),
        })
        .unwrap();
    let last = h.client.submit(patch(7)).unwrap();
    assert_eq!(stored(&h.store)["v"], 0);
    h.release.send(()).unwrap();
    h.first.wait().unwrap().outcome().unwrap();
    let first = one.wait().unwrap();
    let second = two.wait().unwrap();
    assert_eq!(first.group_size(), 2);
    assert_eq!(second.group_size(), 2);
    assert_eq!(json(&first)["v"], 1);
    assert_eq!(json(&second)["v"], 2);
    assert!(matches!(
        delete.wait().unwrap().outcome(),
        Ok(Outcome::Deleted)
    ));
    assert!(matches!(
        missing.wait().unwrap().outcome(),
        Err(Code::DocumentNotFound)
    ));
    let inserted = insert.wait().unwrap();
    assert_eq!(inserted.group_size(), 1);
    assert_eq!(json(&inserted)["v"], 4);
    assert_eq!(json(&update.wait().unwrap())["v"], 5);
    assert!(matches!(
        not_empty.wait().unwrap().outcome(),
        Err(Code::CollectionNotEmpty)
    ));
    delete_again.wait().unwrap().outcome().unwrap();
    assert!(matches!(
        drop_collection.wait().unwrap().outcome(),
        Ok(Outcome::Dropped)
    ));
    assert!(matches!(
        recreate.wait().unwrap().outcome(),
        Ok(Outcome::Created)
    ));
    insert_again.wait().unwrap().outcome().unwrap();
    assert_eq!(json(&last.wait().unwrap())["v"], 7);
    assert_eq!(stored(&h.store)["v"], 7);
    h.coordinator.shutdown().unwrap();
}

#[test]
fn timeout_or_dropped_ticket_cancels_queued_work_but_not_running_work() {
    let h = held(policy());
    let expired = h.client.submit(patch(1)).unwrap();
    let cancelled = h.client.submit(patch(2)).unwrap();
    drop(cancelled);
    assert!(matches!(expired.wait(), Err(Code::WriteBusy)));
    h.release.send(()).unwrap();
    // The already running command exceeded the admission deadline and still succeeds.
    h.first.wait().unwrap().outcome().unwrap();
    h.coordinator.shutdown().unwrap();
    assert_eq!(stored(&h.store)["v"], 0);
    let state = h.client.shared.state.lock().unwrap();
    assert_eq!(state.requests, 0);
    assert_eq!(state.bytes, 0);
    assert!(state.waiting.is_empty());
}

#[test]
fn shutdown_rejects_queue_and_preserves_started_transaction() {
    let h = held(policy());
    let queued = h.client.submit(patch(1)).unwrap();
    h.coordinator.close();
    assert!(matches!(
        queued.wait().unwrap().outcome(),
        Err(Code::ShuttingDown)
    ));
    assert!(matches!(h.client.submit(patch(2)), Err(Code::ShuttingDown)));
    h.release.send(()).unwrap();
    assert!(matches!(
        h.first.wait().unwrap().outcome(),
        Ok(Outcome::Created)
    ));
    h.coordinator.shutdown().unwrap();
    assert_eq!(stored(&h.store)["v"], 0);
}

#[test]
fn retained_replies_keep_request_and_owned_capacity_reservations() {
    let (_temp, store) = fixture();
    let coordinator =
        Coordinator::start(store, policy(), Arc::new(AtomicBool::new(false))).unwrap();
    let client = coordinator.client();
    let large_command = || {
        let mut body = Vec::with_capacity(1024 * 1024);
        body.extend_from_slice(br#"{"v":1}"#);
        Command::Update {
            collection: "orders".into(),
            id: "doc".into(),
            operation: Operation::MergePatch,
            body,
        }
    };
    let mut replies: Vec<_> = (0..7)
        .map(|_| client.submit(large_command()).unwrap().wait().unwrap())
        .collect();
    assert_eq!(json(&replies[0])["v"], 1);
    assert!(client.shared.state.lock().unwrap().bytes >= 7 * 1024 * 1024);
    assert!(matches!(
        client.submit(large_command()),
        Err(Code::WriteBusy)
    ));
    drop(replies.pop());
    client
        .submit(large_command())
        .unwrap()
        .wait()
        .unwrap()
        .outcome()
        .unwrap();
    assert_eq!(client.shared.state.lock().unwrap().requests, 6);
    drop(replies);
    assert_eq!(client.shared.state.lock().unwrap().requests, 0);
    coordinator.shutdown().unwrap();
}

#[test]
fn disconnected_running_request_retains_its_slot_until_worker_finishes() {
    let h = held(Policy {
        max_requests: 1,
        ..policy()
    });
    drop(h.first);
    assert!(matches!(h.client.submit(patch(1)), Err(Code::WriteBusy)));
    h.release.send(()).unwrap();
    h.coordinator.shutdown().unwrap();
    assert_eq!(h.client.shared.state.lock().unwrap().requests, 0);
}

#[test]
fn concurrent_producers_keep_individual_results_and_admission_order() {
    let h = held(policy());
    let barrier = Arc::new(std::sync::Barrier::new(32));
    let mut tickets = thread::scope(|scope| {
        let workers: Vec<_> = (1..=32)
            .map(|value| {
                let client = h.client.clone();
                let barrier = Arc::clone(&barrier);
                scope.spawn(move || {
                    barrier.wait();
                    (value, client.submit(patch(value)).unwrap())
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>()
    });
    tickets.sort_by_key(|(_, ticket)| ticket.sequence());
    let expected = tickets.last().unwrap().0;
    // This scenario checks grouping/order, not wall-clock admission expiry.
    // Creating 32 producer threads while the first writer is deliberately held
    // can exhaust the real 100 ms lease under CI/parallel compiler load.
    // Give this fixture a bounded logical lease; expiry keeps its own tests.
    let deadline = Instant::now() + Duration::from_secs(5);
    {
        let mut state = h.client.shared.state.lock().unwrap();
        assert_eq!(state.ready.len(), 32);
        for job in &mut state.ready {
            job.deadline = deadline;
        }
    }
    for (_, ticket) in &mut tickets {
        ticket.deadline = deadline;
    }
    h.release.send(()).unwrap();
    h.first.wait().unwrap().outcome().unwrap();
    for (value, ticket) in tickets {
        let reply = ticket.wait().unwrap();
        assert_eq!(reply.group_size(), 32);
        assert_eq!(json(&reply)["v"], value);
    }
    assert_eq!(stored(&h.store)["v"], expected);
    h.coordinator.shutdown().unwrap();
}

#[test]
fn worker_panic_closes_admission_and_releases_queued_tickets() {
    let (_temp, store) = fixture();
    let (entered_tx, entered_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::sync_channel(1);
    let coordinator = Coordinator::start_with_hook(
        store,
        policy(),
        Arc::new(AtomicBool::new(false)),
        move || {
            entered_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            panic!("injected writer failure");
        },
    )
    .unwrap();
    let client = coordinator.client();
    let active = client.submit(patch(1)).unwrap();
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let queued = client.submit(patch(2)).unwrap();
    release_tx.send(()).unwrap();
    assert!(matches!(active.wait(), Err(Code::StorageUnavailable)));
    assert!(matches!(
        queued.wait().unwrap().outcome(),
        Err(Code::StorageUnavailable)
    ));
    assert!(matches!(
        client.submit(patch(3)),
        Err(Code::StorageUnavailable)
    ));
    assert_eq!(coordinator.shutdown(), Err(Code::StorageUnavailable));
    assert_eq!(client.shared.state.lock().unwrap().requests, 0);
}

#[test]
fn insertion_preserves_generated_ids_and_individual_errors() {
    let (_temp, store) = fixture();
    let coordinator =
        Coordinator::start(store, policy(), Arc::new(AtomicBool::new(false))).unwrap();
    let client = coordinator.client();
    let duplicate = client
        .submit(Command::Insert {
            collection: "orders".into(),
            body: br#"{"_id":"doc"}"#.to_vec(),
        })
        .unwrap()
        .wait()
        .unwrap();
    assert!(matches!(duplicate.outcome(), Err(Code::DuplicateId)));
    let generate = || {
        client
            .submit(Command::Insert {
                collection: "orders".into(),
                body: br#"{"v":9}"#.to_vec(),
            })
            .unwrap()
            .wait()
            .unwrap()
    };
    let one = generate();
    let two = generate();
    assert_ne!(json(&one)["_id"], json(&two)["_id"]);
    assert_eq!(
        json(&client.submit(patch(1)).unwrap().wait().unwrap())["v"],
        1
    );
    coordinator.shutdown().unwrap();
}

#[test]
fn fifo_budget_waiters_advance_when_retained_receipts_are_released() {
    let (_temp, store) = fixture();
    let coordinator = Coordinator::start(
        store,
        Policy {
            max_requests: 128,
            ..policy()
        },
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    let client = coordinator.client();
    let mut replies: Vec<_> = (0..64)
        .map(|value| client.submit(patch(value + 1)).unwrap().wait().unwrap())
        .collect();
    let await_waiters = |count| {
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut state = client.shared.state.lock().unwrap();
        while state.waiting.len() != count {
            assert!(Instant::now() < deadline, "waiters did not arrive");
            state = client
                .shared
                .changed
                .wait_timeout(state, Duration::from_millis(1))
                .unwrap()
                .0;
        }
    };
    let first_client = client.clone();
    let first = thread::spawn(move || first_client.submit(patch(100)).unwrap());
    await_waiters(1);
    let second_client = client.clone();
    let second = thread::spawn(move || second_client.submit(patch(200)).unwrap());
    await_waiters(2);
    drop(replies.pop());
    let first = first.join().unwrap();
    let first_id = first.sequence();
    let first_reply = first.wait().unwrap();
    assert_eq!(json(&first_reply)["v"], 100);
    // The first result still holds its slot; release another for the second waiter.
    drop(replies.pop());
    let second = second.join().unwrap();
    assert!(first_id < second.sequence());
    assert_eq!(json(&second.wait().unwrap())["v"], 200);
    drop(first_reply);
    drop(replies);
    assert_eq!(client.shared.state.lock().unwrap().requests, 0);
    coordinator.shutdown().unwrap();
}

#[test]
fn external_stop_rejects_pending_commands_without_executing_them() {
    let h = held(policy());
    let pending = h.client.submit(patch(1)).unwrap();
    h.client.shared.stop.store(true, Ordering::Relaxed);
    assert!(matches!(h.client.submit(patch(2)), Err(Code::ShuttingDown)));
    h.release.send(()).unwrap();
    h.first.wait().unwrap().outcome().unwrap();
    assert!(matches!(
        pending.wait().unwrap().outcome(),
        Err(Code::ShuttingDown)
    ));
    h.coordinator.shutdown().unwrap();
    assert_eq!(stored(&h.store)["v"], 0);
}

#[test]
fn invalid_configuration_and_unbounded_input_capacity_are_rejected() {
    let (_temp, store) = fixture();
    assert!(matches!(
        Coordinator::start(
            Arc::clone(&store),
            Policy {
                max_requests: 0,
                ..policy()
            },
            Arc::new(AtomicBool::new(false))
        ),
        Err(Code::InvalidLimit)
    ));
    let coordinator =
        Coordinator::start(store, policy(), Arc::new(AtomicBool::new(false))).unwrap();
    let client = coordinator.client();
    assert!(matches!(
        client.submit(Command::CreateCollection {
            name: "bad/name".into()
        }),
        Err(Code::InvalidCollection)
    ));
    let mut body = Vec::with_capacity(DEFAULT_MAX_DOCUMENT_BYTES + 1);
    body.extend_from_slice(b"{}");
    assert!(matches!(
        client.submit(Command::Insert {
            collection: "orders".into(),
            body
        }),
        Err(Code::WriteBusy)
    ));
    assert_eq!(client.shared.state.lock().unwrap().requests, 0);
    coordinator.shutdown().unwrap();
}

#[test]
fn configured_worker_stack_handles_maximum_json_depth_and_large_documents() {
    let (_temp, store) = fixture();
    let coordinator =
        Coordinator::start(store, policy(), Arc::new(AtomicBool::new(false))).unwrap();
    let client = coordinator.client();
    let _subscription = client
        .submit(Command::Subscribe {
            collection: "orders".into(),
        })
        .unwrap()
        .wait()
        .unwrap();
    let mut body = Vec::with_capacity(DEFAULT_MAX_DOCUMENT_BYTES);
    body.extend_from_slice(br#"{"nested":"#);
    body.extend_from_slice(&b"[".repeat(63));
    body.push(b'"');
    body.extend_from_slice(&vec![b'x'; 1024 * 1024 - 512]);
    body.push(b'"');
    body.extend_from_slice(&b"]".repeat(63));
    body.push(b'}');
    let reply = client
        .submit(Command::Update {
            collection: "orders".into(),
            id: "doc".into(),
            operation: Operation::Replace,
            body,
        })
        .unwrap()
        .wait()
        .unwrap();
    assert!(matches!(reply.outcome(), Ok(Outcome::Updated(_))));
    let patch = client
        .submit(Command::Update {
            collection: "orders".into(),
            id: "doc".into(),
            operation: Operation::MergePatch,
            body: br#"{"flag":true}"#.to_vec(),
        })
        .unwrap()
        .wait()
        .unwrap();
    assert!(matches!(patch.outcome(),Ok(Outcome::Updated(candidate)) if candidate.changed()));
    coordinator.shutdown().unwrap();
}

#[test]
fn transport_permit_survives_disconnected_handler_while_command_is_running() {
    let (_temp, store) = fixture();
    let config = kiarni_config::resolve(
        &[
            ("--max-requests", "1"),
            ("--max-streams", "1"),
            ("--feed-queue", "1"),
        ],
        &[],
        None,
    )
    .unwrap();
    let admission = kiarni_config::Admission::new(&config);
    let permit = Arc::new(admission.acquire(false).unwrap());
    let (entered_tx, entered_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::sync_channel(1);
    let coordinator = Coordinator::start_with_hook(
        store,
        Policy {
            max_requests: 1,
            ..policy()
        },
        Arc::new(AtomicBool::new(false)),
        move || {
            entered_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        },
    )
    .unwrap();
    let ticket = coordinator
        .client()
        .submit_with_permit(patch(1), Arc::clone(&permit))
        .unwrap();
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    drop(permit);
    drop(ticket);
    assert!(matches!(
        admission.acquire(false),
        Err(kiarni_config::AdmissionError::Busy)
    ));
    release_tx.send(()).unwrap();
    coordinator.shutdown().unwrap();
    assert!(admission.acquire(false).is_ok());
}
