//! Bounded, single-worker collection experiment; not a product write coordinator.
use crate::{Checkpoint, Error, MAX_BODY_BYTES, MAX_OPERATIONS, Mutation, Outcome, Store};
use kiarni_core::DEFAULT_MAX_DOCUMENT_BYTES;
use kiarni_engine::writes::Operation;
use std::{
    collections::VecDeque,
    sync::{Arc, Condvar, Mutex, mpsc},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
pub struct Policy {
    /// Includes the executing group, not just requests waiting in the queue.
    pub max_outstanding: usize,
    pub max_owned_bytes: usize,
    pub max_group_operations: usize,
    pub max_group_bytes: usize,
    pub collection_window: Duration,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            max_outstanding: 64,
            max_owned_bytes: 8 * 1024 * 1024,
            max_group_operations: 32,
            max_group_bytes: MAX_BODY_BYTES,
            collection_window: Duration::from_micros(200),
        }
    }
}

impl Policy {
    fn valid(self) -> bool {
        (1..=256).contains(&self.max_outstanding)
            && (1..=16 * 1024 * 1024).contains(&self.max_owned_bytes)
            && (1..=MAX_OPERATIONS).contains(&self.max_group_operations)
            && (1..=MAX_BODY_BYTES).contains(&self.max_group_bytes)
            && self.collection_window <= Duration::from_millis(10)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionError {
    InvalidPolicy,
    InvalidId,
    TooLarge,
    Full,
    Closed,
    SequenceExhausted,
    WorkerStart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    Transaction(Error),
    NotExecuted,
    WorkerLost,
}

#[derive(Debug)]
pub struct Confirmation {
    /// Order of successful admission, not a durable replay cursor.
    pub sequence: u64,
    pub outcome: Outcome,
    pub group_size: usize,
    pub committed: bool,
    pub group_sequence: u64,
    /// Admission through the beginning of transaction processing, including collection.
    pub queue_ns: u64,
    pub service_ns: u64,
}

pub struct Ticket {
    sequence: u64,
    receiver: mpsc::Receiver<Result<Confirmation, Failure>>,
}
impl Ticket {
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
    /// Dropping a ticket does not cancel an admitted mutation.
    pub fn wait(self) -> Result<Confirmation, Failure> {
        self.receiver.recv().unwrap_or(Err(Failure::WorkerLost))
    }
    /// Nonblocking observation for bounded load generators. A result is consumed once.
    pub fn try_receive(&self) -> Result<Option<Confirmation>, Failure> {
        match self.receiver.try_recv() {
            Ok(result) => result.map(Some),
            Err(mpsc::TryRecvError::Empty) => Ok(None),
            Err(mpsc::TryRecvError::Disconnected) => Err(Failure::WorkerLost),
        }
    }
}

struct Pending {
    sequence: u64,
    id: Box<str>,
    body: Box<[u8]>,
    operation: Operation,
    admitted: Instant,
    reply: mpsc::SyncSender<Result<Confirmation, Failure>>,
}
impl Pending {
    fn bytes(&self) -> usize {
        self.id.len() + self.body.len()
    }
}

#[derive(Default)]
struct State {
    queue: VecDeque<Pending>,
    count: usize,
    bytes: usize,
    next: u64,
    closed: bool,
}
struct Shared {
    state: Mutex<State>,
    ready: Condvar,
    policy: Policy,
}

#[derive(Clone)]
pub struct Client {
    shared: Arc<Shared>,
}
impl Client {
    /// Admission never waits for space. Payload is copied only after checking both budgets.
    pub fn try_submit(
        &self,
        id: &str,
        body: &[u8],
        operation: Operation,
    ) -> Result<Ticket, AdmissionError> {
        if !crate::identifier(id) {
            return Err(AdmissionError::InvalidId);
        }
        let bytes = id
            .len()
            .checked_add(body.len())
            .ok_or(AdmissionError::TooLarge)?;
        let policy = self.shared.policy;
        if body.len() > DEFAULT_MAX_DOCUMENT_BYTES
            || body.len() > policy.max_group_bytes
            || bytes > policy.max_owned_bytes
        {
            return Err(AdmissionError::TooLarge);
        }
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.closed {
            return Err(AdmissionError::Closed);
        }
        if state.count >= policy.max_outstanding || bytes > policy.max_owned_bytes - state.bytes {
            return Err(AdmissionError::Full);
        }
        let sequence = state
            .next
            .checked_add(1)
            .ok_or(AdmissionError::SequenceExhausted)?;
        let (reply, receiver) = mpsc::sync_channel(1);
        state.queue.push_back(Pending {
            sequence,
            id: id.into(),
            body: body.into(),
            operation,
            admitted: Instant::now(),
            reply,
        });
        state.next = sequence;
        state.count += 1;
        state.bytes += bytes;
        self.shared.ready.notify_one();
        Ok(Ticket { sequence, receiver })
    }
}

pub struct Collector {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}
impl Collector {
    pub fn start(store: Store, policy: Policy) -> Result<Self, AdmissionError> {
        Self::start_with_hook(store, policy, |_| Ok(()))
    }
    fn start_with_hook(
        store: Store,
        policy: Policy,
        hook: impl FnMut(Checkpoint) -> Result<(), Error> + Send + 'static,
    ) -> Result<Self, AdmissionError> {
        if !policy.valid() {
            return Err(AdmissionError::InvalidPolicy);
        }
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            ready: Condvar::new(),
            policy,
        });
        let worker_shared = Arc::clone(&shared);
        let worker = thread::Builder::new()
            .name("group-collector-spike".into())
            .spawn(move || run(store, worker_shared, hook))
            .map_err(|_| AdmissionError::WorkerStart)?;
        Ok(Self {
            shared,
            worker: Some(worker),
        })
    }
    pub fn client(&self) -> Client {
        Client {
            shared: Arc::clone(&self.shared),
        }
    }
    /// Closes admission and drains accepted work, bypassing collection delay.
    /// Waiting for an in-progress storage commit has no hard time bound.
    pub fn shutdown(mut self) -> Result<(), Failure> {
        self.finish()
    }
    fn finish(&mut self) -> Result<(), Failure> {
        {
            let mut state = self
                .shared
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            state.closed = true;
            self.shared.ready.notify_one();
        }
        if let Some(worker) = self.worker.take() {
            worker.join().map_err(|_| Failure::WorkerLost)?;
        }
        Ok(())
    }
}
impl Drop for Collector {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

// Also closes admission and releases queued replies if the worker unwinds.
struct WorkerGuard(Arc<Shared>);
impl Drop for WorkerGuard {
    fn drop(&mut self) {
        let mut state = self
            .0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.closed = true;
        state.queue.clear();
        state.count = 0;
        state.bytes = 0;
    }
}

fn run(store: Store, shared: Arc<Shared>, mut hook: impl FnMut(Checkpoint) -> Result<(), Error>) {
    let _guard = WorkerGuard(Arc::clone(&shared));
    loop {
        let group = {
            let mut state = shared
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            while state.queue.is_empty() && !state.closed {
                state = shared
                    .ready
                    .wait(state)
                    .unwrap_or_else(|error| error.into_inner());
            }
            let Some(first) = state.queue.pop_front() else {
                return;
            };
            let deadline = first.admitted + shared.policy.collection_window;
            let mut bytes = first.body.len();
            let mut group = Vec::with_capacity(shared.policy.max_group_operations);
            group.push(first);
            while group.len() < shared.policy.max_group_operations {
                if let Some(next) = state.queue.front() {
                    if bytes + next.body.len() > shared.policy.max_group_bytes {
                        break;
                    }
                    if let Some(next) = state.queue.pop_front() {
                        bytes += next.body.len();
                        group.push(next);
                    }
                } else {
                    let now = Instant::now();
                    if state.closed || now >= deadline {
                        break;
                    }
                    let (next, _) = shared
                        .ready
                        .wait_timeout(state, deadline - now)
                        .unwrap_or_else(|error| error.into_inner());
                    state = next;
                }
            }
            group
        };
        let service_start = Instant::now();
        let queue_ns: Vec<_> = group
            .iter()
            .map(|pending| service_start.duration_since(pending.admitted).as_nanos() as u64)
            .collect();
        let group_sequence = group[0].sequence;
        let mutations: Vec<_> = group
            .iter()
            .map(|pending| Mutation {
                id: &pending.id,
                body: &pending.body,
                operation: pending.operation,
            })
            .collect();
        let receipt = store.apply_with_hook(&mutations, &mut hook);
        let service_ns = service_start.elapsed().as_nanos() as u64;
        let failed = receipt.is_err();
        let group_size = group.len();
        // Release owned payloads before returning budget or delivering confirmations.
        drop(mutations);
        let bytes = group.iter().map(Pending::bytes).sum::<usize>();
        let replies: Vec<_> = group
            .into_iter()
            .map(|pending| (pending.sequence, pending.reply))
            .collect();
        let mut state = shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.count -= group_size;
        state.bytes -= bytes;
        if failed {
            state.closed = true;
        }
        drop(state);
        match receipt {
            Ok(receipt) => {
                for (((sequence, reply), outcome), queue_ns) in
                    replies.into_iter().zip(receipt.outcomes).zip(queue_ns)
                {
                    let _ = reply.try_send(Ok(Confirmation {
                        sequence,
                        outcome,
                        group_size,
                        committed: receipt.committed,
                        group_sequence,
                        queue_ns,
                        service_ns,
                    }));
                }
            }
            Err(error) => {
                for (_, reply) in replies {
                    let _ = reply.try_send(Err(Failure::Transaction(error)));
                }
                let mut state = shared
                    .state
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                for pending in state.queue.drain(..) {
                    let _ = pending.reply.try_send(Err(Failure::NotExecuted));
                }
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;

    fn store() -> (tempfile::TempDir, Store) {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::create(
            &temp.path().join("store"),
            &[br#"{"_id":"a","v":0}"#.to_vec()],
        )
        .unwrap();
        (temp, store)
    }
    fn submit(client: &Client, body: &[u8]) -> Result<Ticket, AdmissionError> {
        client.try_submit("a", body, Operation::MergePatch)
    }
    fn gate() -> (mpsc::SyncSender<()>, mpsc::Receiver<()>) {
        mpsc::sync_channel(1)
    }

    #[test]
    fn budgets_include_active_group_and_receipts_wait_for_commit() {
        let (_temp, store) = store();
        let (entered_tx, entered_rx) = gate();
        let (release_tx, release_rx) = gate();
        let collector = Collector::start_with_hook(
            store,
            Policy {
                max_outstanding: 2,
                max_group_operations: 1,
                ..Policy::default()
            },
            move |point| {
                if point == Checkpoint::Staged {
                    entered_tx.send(()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                }
                Ok(())
            },
        )
        .unwrap();
        let client = collector.client();
        let first = submit(&client, br#"{"v":1}"#).unwrap();
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(
            first.receiver.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        let second = submit(&client, br#"{"v":2}"#).unwrap();
        assert!(matches!(submit(&client, b"{}"), Err(AdmissionError::Full)));
        release_tx.send(()).unwrap();
        assert!(first.wait().unwrap().committed);
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        release_tx.send(()).unwrap();
        second.wait().unwrap();
        collector.shutdown().unwrap();
        assert!(matches!(
            submit(&client, b"{}"),
            Err(AdmissionError::Closed)
        ));
    }

    #[test]
    fn bytes_bound_admission_and_groups_without_skipping_fifo() {
        let (_temp, store) = store();
        let (entered_tx, entered_rx) = gate();
        let (release_tx, release_rx) = gate();
        let body = br#"{"v":1}"#;
        let collector = Collector::start_with_hook(
            store,
            Policy {
                max_owned_bytes: (body.len() + 1) * 3,
                max_group_bytes: body.len(),
                max_group_operations: 32,
                collection_window: Duration::ZERO,
                ..Policy::default()
            },
            move |point| {
                if point == Checkpoint::Staged {
                    entered_tx.send(()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                }
                Ok(())
            },
        )
        .unwrap();
        let client = collector.client();
        let first = submit(&client, body).unwrap();
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let second = submit(&client, br#"{"v":2}"#).unwrap();
        let third = submit(&client, br#"{"v":3}"#).unwrap();
        assert!(matches!(submit(&client, body), Err(AdmissionError::Full)));
        assert_eq!(
            client.shared.state.lock().unwrap().bytes,
            (body.len() + 1) * 3
        );
        for ticket in [first, second, third] {
            release_tx.send(()).unwrap();
            assert_eq!(ticket.wait().unwrap().group_size, 1);
            // The third group has no successor to wait for.
            if client.shared.state.lock().unwrap().count > 0 {
                entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            }
        }
        collector.shutdown().unwrap();
    }

    #[test]
    fn concurrent_clients_follow_admission_order_and_shutdown_drains() {
        let (temp, store) = store();
        let collector = Collector::start(
            store,
            Policy {
                max_group_operations: 8,
                collection_window: Duration::from_millis(10),
                ..Policy::default()
            },
        )
        .unwrap();
        let client = collector.client();
        let barrier = Arc::new(Barrier::new(16));
        let mut accepted = thread::scope(|scope| {
            let threads: Vec<_> = (1..=16)
                .map(|value| {
                    let client = client.clone();
                    let barrier = Arc::clone(&barrier);
                    scope.spawn(move || {
                        barrier.wait();
                        let ticket =
                            submit(&client, format!(r#"{{"v":{value}}}"#).as_bytes()).unwrap();
                        (value, ticket)
                    })
                })
                .collect();
            threads
                .into_iter()
                .map(|worker| worker.join().unwrap())
                .collect::<Vec<_>>()
        });
        accepted.sort_by_key(|(_, ticket)| ticket.sequence());
        let expected = accepted.last().unwrap().0;
        collector.shutdown().unwrap();
        for (index, (_, ticket)) in accepted.into_iter().enumerate() {
            let confirmation = ticket.wait().unwrap();
            assert_eq!(confirmation.sequence, index as u64 + 1);
            assert_eq!(confirmation.outcome, Outcome::Changed);
            assert!((1..=8).contains(&confirmation.group_size));
        }
        let reopened = Store::open(&temp.path().join("store")).unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&reopened.get("a").unwrap().unwrap()).unwrap();
        assert_eq!(value["v"], expected);
    }

    #[test]
    fn transaction_failure_closes_admission_and_does_not_execute_queue() {
        let (temp, store) = store();
        let (entered_tx, entered_rx) = gate();
        let (release_tx, release_rx) = gate();
        let collector = Collector::start_with_hook(
            store,
            Policy {
                max_group_operations: 1,
                ..Policy::default()
            },
            move |point| {
                if point == Checkpoint::Staged {
                    entered_tx.send(()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                    return Err(Error::Injected);
                }
                Ok(())
            },
        )
        .unwrap();
        let client = collector.client();
        let first = submit(&client, br#"{"v":1}"#).unwrap();
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let queued = submit(&client, br#"{"v":2}"#).unwrap();
        release_tx.send(()).unwrap();
        assert!(matches!(
            first.wait(),
            Err(Failure::Transaction(Error::Injected))
        ));
        assert!(matches!(queued.wait(), Err(Failure::NotExecuted)));
        assert!(matches!(
            submit(&client, b"{}"),
            Err(AdmissionError::Closed)
        ));
        collector.shutdown().unwrap();
        let reopened = Store::open(&temp.path().join("store")).unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&reopened.get("a").unwrap().unwrap()).unwrap();
        assert_eq!(value["v"], 0);
    }

    #[test]
    fn invalid_policy_payload_noop_rejection_and_dropped_ticket() {
        let (_temp, store) = store();
        assert!(matches!(
            Collector::start(
                store,
                Policy {
                    max_outstanding: 0,
                    ..Policy::default()
                }
            ),
            Err(AdmissionError::InvalidPolicy)
        ));
        let (temp, store) = self::store();
        let collector = Collector::start(store, Policy::default()).unwrap();
        let client = collector.client();
        assert!(matches!(
            client.try_submit("bad/id", b"{}", Operation::Replace),
            Err(AdmissionError::InvalidId)
        ));
        assert!(matches!(
            submit(&client, &vec![b' '; DEFAULT_MAX_DOCUMENT_BYTES + 1]),
            Err(AdmissionError::TooLarge)
        ));
        assert_eq!(
            submit(&client, b"{}").unwrap().wait().unwrap().outcome,
            Outcome::NoOp
        );
        assert!(matches!(
            submit(&client, b"invalid").unwrap().wait().unwrap().outcome,
            Outcome::Rejected(_)
        ));
        drop(submit(&client, br#"{"v":7}"#).unwrap());
        collector.shutdown().unwrap();
        let reopened = Store::open(&temp.path().join("store")).unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&reopened.get("a").unwrap().unwrap()).unwrap();
        assert_eq!(value["v"], 7);
    }

    #[test]
    fn queued_work_forms_bounded_groups_and_shutdown_releases_idle_worker() {
        let (_temp, store) = store();
        let (entered_tx, entered_rx) = gate();
        let (release_tx, release_rx) = gate();
        let mut first = true;
        let collector = Collector::start_with_hook(
            store,
            Policy {
                max_group_operations: 8,
                collection_window: Duration::from_millis(10),
                ..Policy::default()
            },
            move |point| {
                if point == Checkpoint::Staged && first {
                    first = false;
                    entered_tx.send(()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                }
                Ok(())
            },
        )
        .unwrap();
        let client = collector.client();
        let first = submit(&client, br#"{"v":1}"#).unwrap();
        // A lone operation must finish collection without requiring another arrival.
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let queued: Vec<_> = (2..=19)
            .map(|value| submit(&client, format!(r#"{{"v":{value}}}"#).as_bytes()).unwrap())
            .collect();
        release_tx.send(()).unwrap();
        assert_eq!(first.wait().unwrap().group_size, 1);
        for (index, ticket) in queued.into_iter().enumerate() {
            assert_eq!(
                ticket.wait().unwrap().group_size,
                if index < 16 { 8 } else { 2 }
            );
        }
        collector.shutdown().unwrap();
    }

    #[test]
    fn uncertain_commit_is_never_a_successful_confirmation() {
        let (temp, store) = store();
        let collector = Collector::start_with_hook(store, Policy::default(), |point| {
            if point == Checkpoint::Committed {
                Err(Error::Injected)
            } else {
                Ok(())
            }
        })
        .unwrap();
        let client = collector.client();
        let ticket = submit(&client, br#"{"v":9}"#).unwrap();
        assert!(matches!(
            ticket.wait(),
            Err(Failure::Transaction(Error::CommitUncertain))
        ));
        assert!(matches!(
            submit(&client, b"{}"),
            Err(AdmissionError::Closed)
        ));
        collector.shutdown().unwrap();
        let reopened = Store::open(&temp.path().join("store")).unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&reopened.get("a").unwrap().unwrap()).unwrap();
        assert_eq!(value["v"], 9);
    }

    #[test]
    fn worker_panic_closes_admission_and_releases_all_tickets() {
        let (_temp, store) = store();
        let (entered_tx, entered_rx) = gate();
        let (release_tx, release_rx) = gate();
        let collector = Collector::start_with_hook(
            store,
            Policy {
                max_group_operations: 1,
                ..Policy::default()
            },
            move |point| {
                if point == Checkpoint::Staged {
                    entered_tx.send(()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                    panic!("injected worker failure");
                }
                Ok(())
            },
        )
        .unwrap();
        let client = collector.client();
        let active = submit(&client, br#"{"v":1}"#).unwrap();
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let queued = submit(&client, br#"{"v":2}"#).unwrap();
        release_tx.send(()).unwrap();
        assert!(matches!(active.wait(), Err(Failure::WorkerLost)));
        assert!(matches!(queued.wait(), Err(Failure::WorkerLost)));
        assert!(matches!(
            submit(&client, b"{}"),
            Err(AdmissionError::Closed)
        ));
        assert_eq!(collector.shutdown(), Err(Failure::WorkerLost));
    }
}
