use super::*;
use kiarni_feeds::{Kind, Subscription, Terminal};

fn subscription(reply: &Reply) -> &Subscription {
    match reply.outcome().unwrap() {
        Outcome::Subscribed(sub) => sub,
        _ => panic!("expected subscription"),
    }
}
fn subscribe(client: &Client) -> Reply {
    client
        .submit(Command::Subscribe {
            collection: "orders".into(),
        })
        .unwrap()
        .wait()
        .unwrap()
}
fn number(bytes: Option<&[u8]>) -> usize {
    serde_json::from_slice::<serde_json::Value>(bytes.unwrap()).unwrap()["v"]
        .as_u64()
        .unwrap() as usize
}

#[test]
fn nothing_is_published_before_commit_or_before_confirmed_publication() {
    let (_temp, store) = fixture();
    let (pre_tx, pre_rx) = mpsc::sync_channel(1);
    let (pre_release, pre_wait) = mpsc::sync_channel(1);
    let (post_tx, post_rx) = mpsc::sync_channel(1);
    let (post_release, post_wait) = mpsc::sync_channel(1);
    let mut calls = 0;
    let coordinator = Coordinator::start_with_hooks(
        Arc::clone(&store),
        policy(),
        Arc::new(AtomicBool::new(false)),
        move || {
            calls += 1;
            if calls == 2 {
                pre_tx.send(()).unwrap();
                pre_wait.recv_timeout(Duration::from_secs(5)).unwrap();
            }
        },
        move || {
            post_tx.send(()).unwrap();
            post_wait.recv_timeout(Duration::from_secs(5)).unwrap();
            Ok(())
        },
    )
    .unwrap();
    let client = coordinator.client();
    let sub = subscribe(&client);
    let ticket = client.submit(patch(1)).unwrap();
    pre_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(stored(&store)["v"], 0);
    assert!(subscription(&sub).try_next().unwrap().is_none());
    pre_release.send(()).unwrap();
    post_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(
        client
            .feed_metrics()
            .prometheus()
            .contains("kiarni_changefeed_commit_to_dispatch_seconds_count 0\n")
    );
    assert_eq!(stored(&store)["v"], 1);
    assert!(subscription(&sub).try_next().unwrap().is_none());
    post_release.send(()).unwrap();
    ticket.wait().unwrap().outcome().unwrap();
    assert!(
        client
            .feed_metrics()
            .prometheus()
            .contains("kiarni_changefeed_commit_to_dispatch_seconds_count 1\n")
    );
    let event = subscription(&sub).try_next().unwrap().unwrap();
    assert_eq!(event.kind(), Kind::Update);
    assert_eq!(number(event.before()), 0);
    assert_eq!(number(event.after()), 1);
    coordinator.shutdown().unwrap();
}

#[test]
fn grouped_transitions_are_ordered_and_noops_or_rejections_do_not_publish() {
    let h = held(policy());
    let first = h
        .client
        .submit(Command::Subscribe {
            collection: "orders".into(),
        })
        .unwrap();
    let second = h
        .client
        .submit(Command::Subscribe {
            collection: "orders".into(),
        })
        .unwrap();
    let one = h.client.submit(patch(1)).unwrap();
    let two = h.client.submit(patch(2)).unwrap();
    let noop = h
        .client
        .submit(Command::Update {
            collection: "orders".into(),
            id: "doc".into(),
            operation: Operation::MergePatch,
            body: b"{}".to_vec(),
        })
        .unwrap();
    let invalid = h
        .client
        .submit(Command::Update {
            collection: "orders".into(),
            id: "doc".into(),
            operation: Operation::MergePatch,
            body: b"bad json".to_vec(),
        })
        .unwrap();
    h.release.send(()).unwrap();
    h.first.wait().unwrap().outcome().unwrap();
    let first = first.wait().unwrap();
    let second = second.wait().unwrap();
    assert_eq!(one.wait().unwrap().group_size(), 4);
    two.wait().unwrap().outcome().unwrap();
    noop.wait().unwrap().outcome().unwrap();
    assert!(invalid.wait().unwrap().outcome().is_err());
    for sequence in 1..=2 {
        let one = subscription(&first).try_next().unwrap().unwrap();
        let two = subscription(&second).try_next().unwrap().unwrap();
        assert!(Arc::ptr_eq(&one, &two));
        assert_eq!(one.sequence(), sequence);
        assert_eq!(number(one.before()), sequence as usize - 1);
        assert_eq!(number(one.after()), sequence as usize);
    }
    assert!(subscription(&first).try_next().unwrap().is_none());
    h.coordinator.shutdown().unwrap();
}

#[test]
fn subscription_barrier_has_no_snapshot_or_replay() {
    let h = held(policy());
    let before = h.client.submit(patch(1)).unwrap();
    let sub = h
        .client
        .submit(Command::Subscribe {
            collection: "orders".into(),
        })
        .unwrap();
    let after = h.client.submit(patch(2)).unwrap();
    h.release.send(()).unwrap();
    h.first.wait().unwrap().outcome().unwrap();
    before.wait().unwrap().outcome().unwrap();
    let sub = sub.wait().unwrap();
    after.wait().unwrap().outcome().unwrap();
    let event = subscription(&sub).try_next().unwrap().unwrap();
    assert_eq!(event.sequence(), 2);
    assert_eq!(number(event.before()), 1);
    assert_eq!(number(event.after()), 2);
    assert!(subscription(&sub).try_next().unwrap().is_none());
    h.coordinator.shutdown().unwrap();
}

#[test]
fn insert_delete_and_collection_generation_have_explicit_lifecycle() {
    let (_temp, store) = fixture();
    let coordinator =
        Coordinator::start(store, policy(), Arc::new(AtomicBool::new(false))).unwrap();
    let client = coordinator.client();
    let old = subscribe(&client);
    client
        .submit(Command::Delete {
            collection: "orders".into(),
            id: "doc".into(),
        })
        .unwrap()
        .wait()
        .unwrap()
        .outcome()
        .unwrap();
    let event = subscription(&old).try_next().unwrap().unwrap();
    assert_eq!(event.kind(), Kind::Delete);
    assert_eq!(number(event.before()), 0);
    assert!(event.after().is_none());
    client
        .submit(Command::DropCollection {
            name: "orders".into(),
        })
        .unwrap()
        .wait()
        .unwrap()
        .outcome()
        .unwrap();
    assert!(matches!(
        subscription(&old).try_next(),
        Err(Terminal::Interrupted)
    ));
    client
        .submit(Command::CreateCollection {
            name: "orders".into(),
        })
        .unwrap()
        .wait()
        .unwrap()
        .outcome()
        .unwrap();
    let new = subscribe(&client);
    let inserted = client
        .submit(Command::Insert {
            collection: "orders".into(),
            body: br#"{"v":9}"#.to_vec(),
        })
        .unwrap()
        .wait()
        .unwrap();
    let event = subscription(&new).try_next().unwrap().unwrap();
    assert_eq!(event.kind(), Kind::Insert);
    assert!(event.before().is_none());
    assert_eq!(number(event.after()), 9);
    assert_eq!(event.id(), json(&inserted)["_id"].as_str().unwrap());
    coordinator.shutdown().unwrap();
    assert!(matches!(
        subscription(&new).try_next(),
        Err(Terminal::Shutdown)
    ));
}

#[test]
fn slow_consumer_overflow_never_stops_other_consumers_or_writes() {
    let (_temp, store) = fixture();
    let coordinator = Coordinator::start(
        store,
        Policy {
            max_subscribers: 2,
            feed_capacity: 1,
            ..policy()
        },
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    let client = coordinator.client();
    let slow = subscribe(&client);
    let fast = subscribe(&client);
    for n in 1..=3 {
        client
            .submit(patch(n))
            .unwrap()
            .wait()
            .unwrap()
            .outcome()
            .unwrap();
        assert_eq!(
            number(subscription(&fast).try_next().unwrap().unwrap().after()),
            n
        );
    }
    assert!(matches!(
        subscription(&slow).try_next(),
        Err(Terminal::Overflow)
    ));
    assert!(matches!(
        subscribe(&client).outcome(),
        Err(Code::SubscriptionLimit)
    ));
    drop(slow);
    let replacement = subscribe(&client);
    assert!(subscription(&replacement).try_next().unwrap().is_none());
    coordinator.shutdown().unwrap();
}

#[test]
fn failure_after_commit_interrupts_even_before_first_event_and_never_acknowledges() {
    let (_temp, store) = fixture();
    let coordinator = Coordinator::start_with_hooks(
        Arc::clone(&store),
        policy(),
        Arc::new(AtomicBool::new(false)),
        || {},
        || Err(Code::StorageUnavailable),
    )
    .unwrap();
    let client = coordinator.client();
    let sub = subscribe(&client);
    let reply = client.submit(patch(1)).unwrap().wait().unwrap();
    assert!(matches!(reply.outcome(), Err(Code::StorageUnavailable)));
    assert_eq!(stored(&store)["v"], 1);
    assert!(matches!(
        subscription(&sub).try_next(),
        Err(Terminal::Interrupted)
    ));
    assert!(matches!(
        client.submit(patch(2)),
        Err(Code::StorageUnavailable)
    ));
    coordinator.shutdown().unwrap();
}

#[test]
fn failure_before_storage_never_emits_or_changes_the_document() {
    let (_temp, store) = fixture();
    let mut calls = 0;
    let coordinator = Coordinator::start_with_hook(
        Arc::clone(&store),
        policy(),
        Arc::new(AtomicBool::new(false)),
        move || {
            calls += 1;
            if calls == 2 {
                panic!("injected failure before storage");
            }
        },
    )
    .unwrap();
    let client = coordinator.client();
    let sub = subscribe(&client);
    assert!(matches!(
        client.submit(patch(1)).unwrap().wait(),
        Err(Code::StorageUnavailable)
    ));
    assert_eq!(stored(&store)["v"], 0);
    // Join ensures the unwind handler has interrupted the dispatcher.
    assert_eq!(coordinator.shutdown(), Err(Code::StorageUnavailable));
    assert!(matches!(
        subscription(&sub).try_next(),
        Err(Terminal::Interrupted)
    ));
}

#[test]
fn metrics_do_not_wait_for_a_blocked_writer() {
    let h = held(policy());
    let snapshot = h.client.feed_metrics().prometheus();
    assert!(snapshot.contains("kiarni_changefeed_events_published_total 0\n"));
    h.release.send(()).unwrap();
    h.first.wait().unwrap().outcome().unwrap();
    h.coordinator.shutdown().unwrap();
}
