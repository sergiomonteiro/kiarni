//! Disposable M0 experiment; no network protocol or durable event history.
pub mod sse;

use std::{
    collections::BTreeMap,
    fmt,
    path::Path,
    sync::{
        Arc, Mutex, TryLockError, Weak,
        atomic::{AtomicU8, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError},
    },
    time::Duration,
};

use kiarni_core::{DEFAULT_MAX_DOCUMENT_BYTES, PreparedInsert};
use kiarni_storage_spike::Store;

const ACTIVE: u8 = 0;
const OVERFLOW: u8 = 1;
const SHUTDOWN: u8 = 2;
const INTERRUPTED: u8 = 3;
const MAX_SUBSCRIBERS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Busy,
    Closed,
    InvalidLimits,
    InvalidCollection,
    SubscriberLimit,
    SequenceExhausted,
    Poisoned,
    Aborted,
    Storage(kiarni_storage_spike::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiveError {
    Timeout,
    Overflow,
    Shutdown,
    Interrupted,
    Disconnected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Insert,
    Update,
    Delete,
}

pub struct Event {
    pub sequence: u64,
    pub kind: Kind,
    pub collection: String,
    pub id: String,
    pub before: Option<Arc<[u8]>>,
    pub after: Option<Arc<[u8]>>,
}

impl fmt::Debug for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Event")
            .field("sequence", &self.sequence)
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

pub enum Mutation<'a> {
    Insert(&'a PreparedInsert),
    Replace(&'a PreparedInsert),
    Delete(&'a str),
}

struct Subscriber {
    collection: String,
    sender: Option<SyncSender<Arc<Event>>>,
    terminal: Arc<AtomicU8>,
}

struct State {
    store: Store,
    sequence: u64,
    next_subscriber: u64,
    subscribers: BTreeMap<u64, Subscriber>,
    closed: bool,
    capacity: usize,
}

#[derive(Clone)]
pub struct Coordinator(Arc<Mutex<State>>);

pub struct Subscription {
    id: u64,
    owner: Weak<Mutex<State>>,
    receiver: Receiver<Arc<Event>>,
    terminal: Arc<AtomicU8>,
}

impl Subscription {
    fn terminal_error(&self) -> Option<ReceiveError> {
        match self.terminal.load(Ordering::Acquire) {
            OVERFLOW => Some(ReceiveError::Overflow),
            SHUTDOWN => Some(ReceiveError::Shutdown),
            INTERRUPTED => Some(ReceiveError::Interrupted),
            _ => None,
        }
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Result<Arc<Event>, ReceiveError> {
        if let Some(error) = self.terminal_error() {
            while self.receiver.try_recv().is_ok() {}
            return Err(error);
        }
        let result = self.receiver.recv_timeout(timeout);
        if let Some(error) = self.terminal_error() {
            while self.receiver.try_recv().is_ok() {}
            return Err(error);
        }
        result.map_err(|error| match error {
            RecvTimeoutError::Timeout => ReceiveError::Timeout,
            RecvTimeoutError::Disconnected => ReceiveError::Disconnected,
        })
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(owner) = self.owner.upgrade()
            && let Ok(mut state) = owner.lock()
        {
            state.subscribers.remove(&self.id);
            // Release queued payloads before another handle can occupy the slot.
            while self.receiver.try_recv().is_ok() {}
        }
    }
}

impl Drop for State {
    fn drop(&mut self) {
        for subscriber in self.subscribers.values() {
            let _ = subscriber.terminal.compare_exchange(
                ACTIVE,
                SHUTDOWN,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
        }
    }
}

#[derive(Clone, Copy)]
enum Stage {
    BeforeCommit,
    AfterCommit,
}

impl Coordinator {
    pub fn open(path: &Path, capacity: usize) -> Result<Self, Error> {
        if !(1..=8).contains(&capacity) {
            return Err(Error::InvalidLimits);
        }
        let store = Store::open(path).map_err(Error::Storage)?;
        Ok(Self(Arc::new(Mutex::new(State {
            store,
            sequence: 0,
            next_subscriber: 0,
            subscribers: BTreeMap::new(),
            closed: false,
            capacity,
        }))))
    }

    pub fn subscribe(&self, collection: &str) -> Result<Subscription, Error> {
        validate_collection(collection)?;
        let mut state = self.0.lock().map_err(|_| Error::Poisoned)?;
        if state.closed {
            return Err(Error::Closed);
        }
        if state.subscribers.len() >= MAX_SUBSCRIBERS {
            return Err(Error::SubscriberLimit);
        }
        let id = state
            .next_subscriber
            .checked_add(1)
            .ok_or(Error::SequenceExhausted)?;
        state.next_subscriber = id;
        let (sender, receiver) = mpsc::sync_channel(state.capacity);
        let terminal = Arc::new(AtomicU8::new(ACTIVE));
        state.subscribers.insert(
            id,
            Subscriber {
                collection: collection.into(),
                sender: Some(sender),
                terminal: Arc::clone(&terminal),
            },
        );
        Ok(Subscription {
            id,
            owner: Arc::downgrade(&self.0),
            receiver,
            terminal,
        })
    }

    pub fn apply(&self, collection: &str, mutation: Mutation<'_>) -> Result<u64, Error> {
        self.apply_with_hook(collection, mutation, |_, _| true)
    }

    /// Fixture read through the same coordinator; mutations remain encapsulated.
    pub fn get(&self, collection: &str, id: &str) -> Result<Option<Vec<u8>>, Error> {
        validate_collection(collection)?;
        if id.is_empty()
            || id.len() > 128
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        {
            return Err(Error::Storage(kiarni_storage_spike::Error::InvalidDocument));
        }
        let state = self.0.lock().map_err(|_| Error::Poisoned)?;
        if state.closed {
            return Err(Error::Closed);
        }
        state.store.get(collection, id).map_err(Error::Storage)
    }

    fn apply_with_hook(
        &self,
        collection: &str,
        mutation: Mutation<'_>,
        mut hook: impl FnMut(Stage, &Store) -> bool,
    ) -> Result<u64, Error> {
        validate_collection(collection)?;
        let mut state = match self.0.try_lock() {
            Ok(state) => state,
            Err(TryLockError::WouldBlock) => return Err(Error::Busy),
            Err(TryLockError::Poisoned(_)) => return Err(Error::Poisoned),
        };
        if state.closed {
            return Err(Error::Closed);
        }
        let sequence = state
            .sequence
            .checked_add(1)
            .ok_or(Error::SequenceExhausted)?;
        if let Mutation::Insert(candidate) | Mutation::Replace(candidate) = mutation
            && candidate.json().len() > DEFAULT_MAX_DOCUMENT_BYTES
        {
            return Err(Error::Storage(kiarni_storage_spike::Error::InvalidDocument));
        }
        let (id, kind, after) = match mutation {
            Mutation::Insert(candidate) => (
                candidate.id(),
                Kind::Insert,
                Some(Arc::from(candidate.json())),
            ),
            Mutation::Replace(candidate) => (
                candidate.id(),
                Kind::Update,
                Some(Arc::from(candidate.json())),
            ),
            Mutation::Delete(id) => (id, Kind::Delete, None),
        };
        // Bound metadata on delete too; prepared candidates already validate IDs.
        if id.is_empty()
            || id.len() > 128
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        {
            return Err(Error::Storage(kiarni_storage_spike::Error::InvalidDocument));
        }
        let before = state
            .store
            .get(collection, id)
            .map_err(Error::Storage)?
            .map(Arc::from);
        if !hook(Stage::BeforeCommit, &state.store) {
            return Err(Error::Aborted);
        }
        match mutation {
            Mutation::Insert(candidate) => state.store.insert(collection, candidate),
            Mutation::Replace(candidate) => state.store.replace(collection, candidate),
            Mutation::Delete(id) => state.store.delete(collection, id),
        }
        .map_err(Error::Storage)?;
        state.sequence = sequence;
        if !hook(Stage::AfterCommit, &state.store) {
            terminate(&mut state, INTERRUPTED);
            return Ok(sequence);
        }
        let event = Arc::new(Event {
            sequence,
            kind,
            collection: collection.into(),
            id: id.into(),
            before,
            after,
        });
        for subscriber in state.subscribers.values_mut() {
            if subscriber.collection != collection {
                continue;
            }
            let result = subscriber
                .sender
                .as_ref()
                .map(|sender| sender.try_send(Arc::clone(&event)));
            match result {
                Some(Err(TrySendError::Full(_))) => {
                    subscriber.terminal.store(OVERFLOW, Ordering::Release);
                    subscriber.sender = None;
                }
                Some(Err(TrySendError::Disconnected(_))) => {
                    subscriber.sender = None;
                }
                Some(Ok(())) | None => {}
            }
        }
        Ok(sequence)
    }

    pub fn shutdown(&self) -> Result<(), Error> {
        let mut state = self.0.lock().map_err(|_| Error::Poisoned)?;
        state.closed = true;
        terminate(&mut state, SHUTDOWN);
        Ok(())
    }
}

fn terminate(state: &mut State, reason: u8) {
    for subscriber in state.subscribers.values_mut() {
        let _ = subscriber.terminal.compare_exchange(
            ACTIVE,
            reason,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        subscriber.sender = None;
    }
}

fn validate_collection(collection: &str) -> Result<(), Error> {
    if collection.is_empty()
        || collection.len() > 128
        || !collection
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
    {
        return Err(Error::InvalidCollection);
    }
    Ok(())
}

impl From<Error> for kiarni_http::Code {
    fn from(error: Error) -> Self {
        use Error as E;
        match error {
            E::Busy => Self::WriteBusy,
            E::Closed => Self::ShuttingDown,
            E::InvalidLimits | E::Aborted => Self::InternalError,
            E::InvalidCollection => Self::InvalidCollection,
            E::SubscriberLimit => Self::SubscriptionLimit,
            E::SequenceExhausted => Self::SequenceExhausted,
            E::Poisoned => Self::ServiceUnavailable,
            E::Storage(error) => Self::from(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kiarni_core::DocumentInput;
    use std::{sync::Barrier, thread, time::Instant};

    const WAIT: Duration = Duration::from_secs(3);

    fn candidate(id: &str, status: &str) -> PreparedInsert {
        DocumentInput::parse(
            format!(r#"{{"_id":"{id}","status":"{status}"}}"#).as_bytes(),
            DEFAULT_MAX_DOCUMENT_BYTES,
        )
        .unwrap()
        .prepare_insert()
        .unwrap()
    }

    fn fixture(capacity: usize) -> (tempfile::TempDir, Coordinator) {
        let dir = tempfile::tempdir().unwrap();
        let coordinator = Coordinator::open(&dir.path().join("db.redb"), capacity).unwrap();
        (dir, coordinator)
    }

    fn empty(subscription: &Subscription) {
        assert!(matches!(
            subscription.recv_timeout(Duration::ZERO),
            Err(ReceiveError::Timeout)
        ));
    }

    #[test]
    fn two_subscribers_receive_crud_states_after_commit() {
        let (_dir, coordinator) = fixture(4);
        let first = coordinator.subscribe("orders").unwrap();
        let second = coordinator.subscribe("orders").unwrap();
        let other = coordinator.subscribe("other").unwrap();
        let old = candidate("secret-id", "new");
        let new = candidate("secret-id", "paid");
        assert_eq!(coordinator.apply("orders", Mutation::Insert(&old)), Ok(1));
        for subscriber in [&first, &second] {
            let event = subscriber.recv_timeout(WAIT).unwrap();
            assert_eq!(event.sequence, 1);
            assert_eq!(event.kind, Kind::Insert);
            assert_eq!(event.id, "secret-id");
            assert_eq!(event.collection, "orders");
            assert!(event.before.is_none());
            assert_eq!(event.after.as_deref(), Some(old.json()));
            assert!(
                coordinator
                    .0
                    .lock()
                    .unwrap()
                    .store
                    .get("orders", &event.id)
                    .unwrap()
                    .is_some()
            );
            assert_eq!(
                format!("{event:?}"),
                "Event { sequence: 1, kind: Insert, .. }"
            );
        }
        assert_eq!(coordinator.apply("orders", Mutation::Replace(&new)), Ok(2));
        assert_eq!(
            coordinator.apply("orders", Mutation::Delete("secret-id")),
            Ok(3)
        );
        for subscriber in [&first, &second] {
            let update = subscriber.recv_timeout(WAIT).unwrap();
            assert_eq!(update.kind, Kind::Update);
            assert_eq!(update.sequence, 2);
            assert_eq!(update.before.as_deref(), Some(old.json()));
            assert_eq!(update.after.as_deref(), Some(new.json()));
            let delete = subscriber.recv_timeout(WAIT).unwrap();
            assert_eq!(delete.kind, Kind::Delete);
            assert_eq!(delete.sequence, 3);
            assert_eq!(delete.before.as_deref(), Some(new.json()));
            assert!(delete.after.is_none());
        }
        empty(&other);
    }

    #[test]
    fn pause_after_commit_cannot_be_overtaken_by_another_writer() {
        let (_dir, coordinator) = fixture(4);
        let sub = coordinator.subscribe("orders").unwrap();
        let writer = coordinator.clone();
        let (reached_tx, reached_rx) = mpsc::sync_channel(1);
        let (continue_tx, continue_rx) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            writer.apply_with_hook(
                "orders",
                Mutation::Insert(&candidate("first", "new")),
                |stage, store| {
                    match stage {
                        Stage::BeforeCommit => {
                            assert!(store.get("orders", "first").unwrap().is_none())
                        }
                        Stage::AfterCommit => {
                            assert!(store.get("orders", "first").unwrap().is_some())
                        }
                    }
                    reached_tx.send(()).unwrap();
                    continue_rx.recv_timeout(WAIT).unwrap();
                    true
                },
            )
        });
        let next = candidate("second", "new");
        for _ in 0..2 {
            reached_rx.recv_timeout(WAIT).unwrap();
            empty(&sub);
            assert_eq!(
                coordinator.apply("orders", Mutation::Insert(&next)),
                Err(Error::Busy)
            );
            continue_tx.send(()).unwrap();
        }
        assert_eq!(worker.join().unwrap(), Ok(1));
        assert_eq!(coordinator.apply("orders", Mutation::Insert(&next)), Ok(2));
        assert_eq!(sub.recv_timeout(WAIT).unwrap().id, "first");
        assert_eq!(sub.recv_timeout(WAIT).unwrap().id, "second");
    }

    #[test]
    fn concurrent_writers_produce_the_same_order_for_independent_consumers() {
        let (_dir, coordinator) = fixture(8);
        let first = coordinator.subscribe("orders").unwrap();
        let second = coordinator.subscribe("orders").unwrap();
        let barrier = Arc::new(Barrier::new(4));
        let mut workers = Vec::new();
        for worker in 0..4 {
            let coordinator = coordinator.clone();
            let barrier = Arc::clone(&barrier);
            workers.push(thread::spawn(move || {
                barrier.wait();
                let mut receipts = Vec::new();
                for i in 0..2 {
                    let id = format!("w{worker}_{i}");
                    let candidate = candidate(&id, "new");
                    let deadline = Instant::now() + WAIT;
                    loop {
                        match coordinator.apply("orders", Mutation::Insert(&candidate)) {
                            Ok(sequence) => {
                                receipts.push((sequence, id));
                                break;
                            }
                            Err(Error::Busy) if Instant::now() < deadline => thread::yield_now(),
                            other => panic!("writer failed: {other:?}"),
                        }
                    }
                }
                receipts
            }));
        }
        let mut committed: Vec<_> = workers
            .into_iter()
            .flat_map(|worker| worker.join().unwrap())
            .collect();
        committed.sort();
        assert_eq!(
            committed.iter().map(|(seq, _)| *seq).collect::<Vec<_>>(),
            (1..=8).collect::<Vec<_>>()
        );
        for sub in [&first, &second] {
            let received: Vec<_> = (0..8)
                .map(|_| {
                    let event = sub.recv_timeout(WAIT).unwrap();
                    (event.sequence, event.id.clone())
                })
                .collect();
            assert_eq!(received, committed);
        }
    }

    #[test]
    fn failed_mutations_and_precommit_faults_do_not_emit_or_consume_sequence() {
        let (_dir, coordinator) = fixture(4);
        let sub = coordinator.subscribe("orders").unwrap();
        let document = candidate("a", "new");
        assert_eq!(
            coordinator.apply_with_hook("orders", Mutation::Insert(&document), |_, _| false),
            Err(Error::Aborted)
        );
        assert!(
            coordinator
                .0
                .lock()
                .unwrap()
                .store
                .get("orders", "a")
                .unwrap()
                .is_none()
        );
        empty(&sub);
        assert_eq!(
            coordinator.apply("orders", Mutation::Insert(&document)),
            Ok(1)
        );
        sub.recv_timeout(WAIT).unwrap();
        assert_eq!(
            coordinator.apply("orders", Mutation::Insert(&document)),
            Err(Error::Storage(kiarni_storage_spike::Error::DuplicateId))
        );
        assert_eq!(
            coordinator.apply("orders", Mutation::Delete("missing")),
            Err(Error::Storage(kiarni_storage_spike::Error::MissingDocument))
        );
        empty(&sub);
        assert_eq!(coordinator.apply("orders", Mutation::Delete("a")), Ok(2));
        assert_eq!(sub.recv_timeout(WAIT).unwrap().sequence, 2);
    }

    #[test]
    fn postcommit_gap_keeps_data_and_interrupts_subscribers_without_replay() {
        let (_dir, coordinator) = fixture(4);
        let old = coordinator.subscribe("orders").unwrap();
        let result = coordinator.apply_with_hook(
            "orders",
            Mutation::Insert(&candidate("gap", "new")),
            |stage, _| matches!(stage, Stage::BeforeCommit),
        );
        assert_eq!(result, Ok(1));
        assert!(
            coordinator
                .0
                .lock()
                .unwrap()
                .store
                .get("orders", "gap")
                .unwrap()
                .is_some()
        );
        assert!(matches!(
            old.recv_timeout(WAIT),
            Err(ReceiveError::Interrupted)
        ));
        let fresh = coordinator.subscribe("orders").unwrap();
        empty(&fresh);
        assert_eq!(
            coordinator.apply("orders", Mutation::Insert(&candidate("future", "new"))),
            Ok(2)
        );
        assert_eq!(fresh.recv_timeout(WAIT).unwrap().id, "future");
    }

    #[test]
    fn slow_consumer_overflows_without_blocking_writer_or_fast_consumer() {
        let (_dir, coordinator) = fixture(1);
        let slow = coordinator.subscribe("orders").unwrap();
        let fast = coordinator.subscribe("orders").unwrap();
        for i in 0..4 {
            assert_eq!(
                coordinator.apply(
                    "orders",
                    Mutation::Insert(&candidate(&format!("id{i}"), "new"))
                ),
                Ok(i + 1)
            );
            assert_eq!(fast.recv_timeout(WAIT).unwrap().sequence, i + 1);
        }
        assert!(matches!(
            slow.recv_timeout(WAIT),
            Err(ReceiveError::Overflow)
        ));
        assert!(matches!(
            slow.recv_timeout(WAIT),
            Err(ReceiveError::Overflow)
        ));
        assert_eq!(
            coordinator
                .0
                .lock()
                .unwrap()
                .subscribers
                .values()
                .filter(|subscriber| subscriber.sender.is_some())
                .count(),
            1
        );
        // No queued payloads retained after terminal error is observed.
        assert!(slow.receiver.try_recv().is_err());
    }

    #[test]
    fn overflowed_handles_still_count_toward_the_retention_limit() {
        let (_dir, coordinator) = fixture(1);
        let handles: Vec<_> = (0..8)
            .map(|_| coordinator.subscribe("orders").unwrap())
            .collect();
        coordinator
            .apply("orders", Mutation::Insert(&candidate("a", "new")))
            .unwrap();
        coordinator
            .apply("orders", Mutation::Insert(&candidate("b", "new")))
            .unwrap();
        assert!(
            handles
                .iter()
                .all(|handle| handle.terminal_error() == Some(ReceiveError::Overflow))
        );
        assert!(matches!(
            coordinator.subscribe("orders"),
            Err(Error::SubscriberLimit)
        ));
        // Terminal reasons are sticky even across shutdown.
        coordinator.shutdown().unwrap();
        assert!(
            handles
                .iter()
                .all(|handle| matches!(handle.recv_timeout(WAIT), Err(ReceiveError::Overflow)))
        );
    }

    #[test]
    fn naive_commit_then_publish_can_reverse_confirmed_commit_order() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(&dir.path().join("naive.redb")).unwrap());
        let writer = Arc::clone(&store);
        let (committed_tx, committed_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        let (events_tx, events_rx) = mpsc::sync_channel(2);
        let first_events = events_tx.clone();
        let first = thread::spawn(move || {
            writer.insert("orders", &candidate("first", "new")).unwrap();
            committed_tx.send(()).unwrap();
            release_rx.recv_timeout(WAIT).unwrap();
            first_events.send("first").unwrap();
        });
        committed_rx.recv_timeout(WAIT).unwrap();
        store.insert("orders", &candidate("second", "new")).unwrap();
        events_tx.send("second").unwrap();
        release_tx.send(()).unwrap();
        first.join().unwrap();
        assert_eq!(events_rx.recv_timeout(WAIT).unwrap(), "second");
        assert_eq!(events_rx.recv_timeout(WAIT).unwrap(), "first");
    }

    #[test]
    fn dropping_and_reconnecting_releases_capacity_and_has_no_history() {
        let (_dir, coordinator) = fixture(1);
        let mut subscriptions: Vec<_> = (0..8)
            .map(|_| coordinator.subscribe("orders").unwrap())
            .collect();
        assert!(matches!(
            coordinator.subscribe("orders"),
            Err(Error::SubscriberLimit)
        ));
        subscriptions.pop();
        let replacement = coordinator.subscribe("orders").unwrap();
        drop(subscriptions);
        drop(replacement);
        assert_eq!(
            coordinator.apply("orders", Mutation::Insert(&candidate("past", "new"))),
            Ok(1)
        );
        let fresh = coordinator.subscribe("orders").unwrap();
        empty(&fresh);
        assert_eq!(
            coordinator.apply("orders", Mutation::Insert(&candidate("future", "new"))),
            Ok(2)
        );
        assert_eq!(fresh.recv_timeout(WAIT).unwrap().id, "future");
    }

    #[test]
    fn shutdown_discards_backlog_and_rejects_new_work() {
        let (_dir, coordinator) = fixture(4);
        let sub = coordinator.subscribe("orders").unwrap();
        coordinator
            .apply("orders", Mutation::Insert(&candidate("a", "new")))
            .unwrap();
        coordinator.shutdown().unwrap();
        coordinator.shutdown().unwrap();
        assert!(matches!(
            sub.recv_timeout(WAIT),
            Err(ReceiveError::Shutdown)
        ));
        assert!(matches!(
            coordinator.subscribe("orders"),
            Err(Error::Closed)
        ));
        assert_eq!(
            coordinator.apply("orders", Mutation::Delete("a")),
            Err(Error::Closed)
        );
    }

    #[test]
    fn shutdown_wakes_a_waiting_consumer_and_drop_closes_live_handles() {
        let (_dir, coordinator) = fixture(4);
        let sub = coordinator.subscribe("orders").unwrap();
        let worker = thread::spawn(move || sub.recv_timeout(WAIT));
        coordinator.shutdown().unwrap();
        assert!(matches!(
            worker.join().unwrap(),
            Err(ReceiveError::Shutdown)
        ));
        let (_other_dir, other) = fixture(4);
        let sub = other.subscribe("orders").unwrap();
        drop(other);
        assert!(matches!(
            sub.recv_timeout(WAIT),
            Err(ReceiveError::Shutdown)
        ));
    }

    #[test]
    fn restart_preserves_data_but_sequence_and_history_are_not_durable() {
        let (dir, coordinator) = fixture(4);
        coordinator
            .apply("orders", Mutation::Insert(&candidate("past", "new")))
            .unwrap();
        drop(coordinator);
        let reopened = Coordinator::open(&dir.path().join("db.redb"), 4).unwrap();
        assert!(
            reopened
                .0
                .lock()
                .unwrap()
                .store
                .get("orders", "past")
                .unwrap()
                .is_some()
        );
        let sub = reopened.subscribe("orders").unwrap();
        empty(&sub);
        assert_eq!(
            reopened.apply("orders", Mutation::Insert(&candidate("new", "new"))),
            Ok(1)
        );
        assert_eq!(sub.recv_timeout(WAIT).unwrap().sequence, 1);
    }

    #[test]
    fn limits_and_sequence_exhaustion_are_checked_before_commit() {
        let (dir, coordinator) = fixture(4);
        for capacity in [0, 9] {
            assert!(matches!(
                Coordinator::open(&dir.path().join("other.redb"), capacity),
                Err(Error::InvalidLimits)
            ));
        }
        let document = candidate("a", "new");
        for collection in ["", "../orders"] {
            assert!(matches!(
                coordinator.subscribe(collection),
                Err(Error::InvalidCollection)
            ));
            assert_eq!(
                coordinator.apply(collection, Mutation::Insert(&document)),
                Err(Error::InvalidCollection)
            );
        }
        let sub = coordinator.subscribe("orders").unwrap();
        coordinator.0.lock().unwrap().sequence = u64::MAX;
        assert_eq!(
            coordinator.apply("orders", Mutation::Insert(&document)),
            Err(Error::SequenceExhausted)
        );
        assert!(
            coordinator
                .0
                .lock()
                .unwrap()
                .store
                .get("orders", "a")
                .unwrap()
                .is_none()
        );
        empty(&sub);
    }

    #[test]
    fn oversized_candidates_are_rejected_before_copying_into_events() {
        let (_dir, coordinator) = fixture(4);
        let sub = coordinator.subscribe("orders").unwrap();
        let json = format!(
            r#"{{"_id":"large","status":"new","payload":"{}"}}"#,
            "x".repeat(DEFAULT_MAX_DOCUMENT_BYTES)
        );
        // Core supports configurable limits; this experiment always caps at 1 MiB.
        let document = DocumentInput::parse(json.as_bytes(), 2 * DEFAULT_MAX_DOCUMENT_BYTES)
            .unwrap()
            .prepare_insert()
            .unwrap();
        assert_eq!(
            coordinator.apply("orders", Mutation::Insert(&document)),
            Err(Error::Storage(kiarni_storage_spike::Error::InvalidDocument))
        );
        assert!(
            coordinator
                .0
                .lock()
                .unwrap()
                .store
                .get("orders", "large")
                .unwrap()
                .is_none()
        );
        empty(&sub);
    }

    #[test]
    fn node_sequence_has_expected_gaps_between_collection_scopes() {
        let (_dir, coordinator) = fixture(4);
        let orders = coordinator.subscribe("orders").unwrap();
        let other = coordinator.subscribe("other").unwrap();
        let document = candidate("a", "new");
        assert_eq!(
            coordinator.apply("orders", Mutation::Insert(&document)),
            Ok(1)
        );
        assert_eq!(
            coordinator.apply("other", Mutation::Insert(&document)),
            Ok(2)
        );
        assert_eq!(coordinator.apply("orders", Mutation::Delete("a")), Ok(3));
        assert_eq!(orders.recv_timeout(WAIT).unwrap().sequence, 1);
        assert_eq!(orders.recv_timeout(WAIT).unwrap().sequence, 3);
        assert_eq!(other.recv_timeout(WAIT).unwrap().sequence, 2);
        empty(&other);
    }
}
