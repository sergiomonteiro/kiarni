//! Bounded collection dispatcher. Callers must confirm storage commits before publication.
mod metrics;
pub mod sse;
use kiarni_core::{DEFAULT_MAX_DOCUMENT_BYTES, DocumentInput};
pub use metrics::{Metrics, Snapshot};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, AtomicU64, Ordering},
        mpsc,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Insert,
    Update,
    Delete,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    InvalidLimits,
    InvalidChange,
    TooManySubscribers,
    Closed,
    SequenceExhausted,
    InvalidReservation,
    ReservationExhausted,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Terminal {
    Overflow = 1,
    Shutdown = 2,
    Interrupted = 3,
}

fn name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}

pub struct Change {
    collection: Box<str>,
    id: Box<str>,
    kind: Kind,
    before: Option<Arc<[u8]>>,
    after: Option<Arc<[u8]>>,
}
impl Change {
    pub fn new(
        collection: &str,
        id: &str,
        kind: Kind,
        before: Option<&[u8]>,
        after: Option<&[u8]>,
    ) -> Result<Self, Error> {
        if [before, after]
            .into_iter()
            .flatten()
            .any(|bytes| bytes.len() > DEFAULT_MAX_DOCUMENT_BYTES)
        {
            return Err(Error::InvalidChange);
        }
        Self::from_shared(
            collection,
            id,
            kind,
            before.map(Arc::from),
            after.map(Arc::from),
        )
    }
    pub fn from_shared(
        collection: &str,
        id: &str,
        kind: Kind,
        before: Option<Arc<[u8]>>,
        after: Option<Arc<[u8]>>,
    ) -> Result<Self, Error> {
        if !name(collection)
            || !name(id)
            || !matches!(
                (kind, before.is_some(), after.is_some()),
                (Kind::Insert, false, true)
                    | (Kind::Update, true, true)
                    | (Kind::Delete, true, false)
            )
        {
            return Err(Error::InvalidChange);
        }
        for bytes in [before.as_deref(), after.as_deref()].into_iter().flatten() {
            let input = DocumentInput::parse(bytes, DEFAULT_MAX_DOCUMENT_BYTES)
                .map_err(|_| Error::InvalidChange)?;
            if input.id() != Some(id) {
                return Err(Error::InvalidChange);
            }
        }
        Ok(Self {
            collection: collection.into(),
            id: id.into(),
            kind,
            before,
            after,
        })
    }
}
impl std::fmt::Debug for Change {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Change")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}
pub struct Event {
    sequence: u64,
    change: Change,
}
impl Event {
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
    pub fn kind(&self) -> Kind {
        self.change.kind
    }
    pub fn collection(&self) -> &str {
        &self.change.collection
    }
    pub fn id(&self) -> &str {
        &self.change.id
    }
    pub fn before(&self) -> Option<&[u8]> {
        self.change.before.as_deref()
    }
    pub fn after(&self) -> Option<&[u8]> {
        self.change.after.as_deref()
    }
}
impl std::fmt::Debug for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Event")
            .field("sequence", &self.sequence)
            .field("kind", &self.kind())
            .finish_non_exhaustive()
    }
}
fn terminal(flag: &AtomicU8) -> Option<Terminal> {
    match flag.load(Ordering::Acquire) {
        0 => None,
        1 => Some(Terminal::Overflow),
        2 => Some(Terminal::Shutdown),
        _ => Some(Terminal::Interrupted),
    }
}
fn end(flag: &AtomicU8, reason: Terminal) -> bool {
    flag.compare_exchange(0, reason as u8, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}
struct Sender {
    collection: Box<str>,
    sender: mpsc::SyncSender<Arc<Event>>,
    terminal: Arc<AtomicU8>,
}
struct State {
    metrics: metrics::Counters,
    sequence: u64,
    occupied: [bool; 32],
    senders: BTreeMap<usize, Sender>,
    closed: Option<Terminal>,
}
struct Shared {
    state: Mutex<State>,
    queued: [AtomicU64; 32],
}

/// Single owner; future registration and publication belong to the writer actor.
pub struct Dispatcher {
    shared: Arc<Shared>,
    slots: usize,
    capacity: usize,
}
impl Dispatcher {
    pub fn new(slots: usize, capacity: usize) -> Result<Self, Error> {
        if !(1..=32).contains(&slots) || !(1..=16).contains(&capacity) {
            return Err(Error::InvalidLimits);
        }
        Ok(Self {
            shared: Arc::new(Shared {
                state: Mutex::new(State {
                    metrics: metrics::Counters::default(),
                    sequence: 0,
                    occupied: [false; 32],
                    senders: BTreeMap::new(),
                    closed: None,
                }),
                queued: std::array::from_fn(|_| AtomicU64::new(0)),
            }),
            slots,
            capacity,
        })
    }
    pub fn metrics(&self) -> Metrics {
        Metrics {
            shared: Arc::clone(&self.shared),
            capacity: self.slots * self.capacity,
        }
    }
    /// Future events only: no initial snapshot or replay.
    pub fn subscribe(&mut self, collection: &str) -> Result<Subscription, Error> {
        if !name(collection) {
            return Err(Error::InvalidChange);
        }
        let mut state = self.shared.state.lock().map_err(|_| Error::Closed)?;
        if state.closed.is_some() {
            return Err(Error::Closed);
        }
        let id = state.occupied[..self.slots]
            .iter()
            .position(|busy| !busy)
            .ok_or(Error::TooManySubscribers)?;
        let (sender, receiver) = mpsc::sync_channel(self.capacity);
        let flag = Arc::new(AtomicU8::new(0));
        state.occupied[id] = true;
        state.metrics.subscriptions = state.metrics.subscriptions.saturating_add(1);
        state.senders.insert(
            id,
            Sender {
                collection: collection.into(),
                sender,
                terminal: Arc::clone(&flag),
            },
        );
        Ok(Subscription {
            shared: Arc::clone(&self.shared),
            id,
            receiver,
            terminal: flag,
        })
    }
    /// Check sequence headroom before committing, retaining exclusive publisher ownership.
    /// This reservation does not itself execute or certify a storage transaction.
    pub fn reserve(&mut self, max_changes: usize) -> Result<Publication<'_>, Error> {
        if !(1..=32).contains(&max_changes) {
            return Err(Error::InvalidReservation);
        }
        {
            let state = self.shared.state.lock().map_err(|_| Error::Closed)?;
            if state.closed.is_some() {
                return Err(Error::Closed);
            }
            state
                .sequence
                .checked_add(max_changes as u64)
                .ok_or(Error::SequenceExhausted)?;
        }
        Ok(Publication {
            dispatcher: self,
            remaining: max_changes,
            published: false,
            finished: false,
        })
    }
    pub fn shutdown(&mut self) {
        self.close(Terminal::Shutdown);
    }
    pub fn interrupt(&mut self) {
        self.close(Terminal::Interrupted);
    }
    pub fn observes(&self, collection: &str) -> Result<bool, Error> {
        let state = self.shared.state.lock().map_err(|_| Error::Closed)?;
        if state.closed.is_some() {
            return Err(Error::Closed);
        }
        Ok(state
            .senders
            .values()
            .any(|sender| sender.collection.as_ref() == collection))
    }
    pub fn interrupt_collection(&mut self, collection: &str) {
        let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        let State {
            senders, metrics, ..
        } = &mut *state;
        senders.retain(|_, sender| {
            if sender.collection.as_ref() != collection {
                return true;
            }
            if end(&sender.terminal, Terminal::Interrupted) {
                metrics.end(Terminal::Interrupted);
            }
            false
        });
    }
    fn close(&mut self, reason: Terminal) {
        let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        state.closed.get_or_insert(reason);
        let mut ended = 0;
        for sender in state.senders.values() {
            if end(&sender.terminal, reason) {
                ended += 1;
            }
        }
        for _ in 0..ended {
            state.metrics.end(reason);
        }
        state.senders.clear();
    }
}
impl Drop for Dispatcher {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub struct Publication<'a> {
    dispatcher: &'a mut Dispatcher,
    remaining: usize,
    published: bool,
    finished: bool,
}
impl Publication<'_> {
    /// For a committed change in a collection without active observers only.
    pub fn advance_unobserved_confirmed(&mut self, collection: &str) -> Result<u64, Error> {
        if self.remaining == 0 {
            return Err(Error::ReservationExhausted);
        }
        let mut state = self
            .dispatcher
            .shared
            .state
            .lock()
            .map_err(|_| Error::Closed)?;
        if state.closed.is_some() {
            return Err(Error::Closed);
        }
        if state
            .senders
            .values()
            .any(|sender| sender.collection.as_ref() == collection)
        {
            return Err(Error::InvalidChange);
        }
        state.sequence = state
            .sequence
            .checked_add(1)
            .ok_or(Error::SequenceExhausted)?;
        self.remaining -= 1;
        self.published = true;
        state.metrics.published = state.metrics.published.saturating_add(1);
        state.metrics.unobserved = state.metrics.unobserved.saturating_add(1);
        Ok(state.sequence)
    }
    /// Precondition: the corresponding mutation's durable commit has succeeded.
    /// No-op and rejected mutations must not be published.
    pub fn publish_confirmed(&mut self, change: Change) -> Result<u64, Error> {
        self.publish_confirmed_at(change, std::time::Instant::now())
    }
    pub fn publish_confirmed_at(
        &mut self,
        change: Change,
        confirmed_at: std::time::Instant,
    ) -> Result<u64, Error> {
        if self.remaining == 0 {
            return Err(Error::ReservationExhausted);
        }
        let mut state = self
            .dispatcher
            .shared
            .state
            .lock()
            .map_err(|_| Error::Closed)?;
        if state.closed.is_some() {
            return Err(Error::Closed);
        }
        let sequence = state
            .sequence
            .checked_add(1)
            .ok_or(Error::SequenceExhausted)?;
        let event = Arc::new(Event { sequence, change });
        state.sequence = sequence;
        self.remaining -= 1;
        self.published = true;
        state.metrics.published = state.metrics.published.saturating_add(1);
        let State {
            senders, metrics, ..
        } = &mut *state;
        let mut observed = false;
        senders.retain(|id, sender| {
            if sender.collection.as_ref() != event.collection() {
                return true;
            }
            observed = true;
            self.dispatcher.shared.queued[*id].fetch_add(1, Ordering::Relaxed);
            match sender.sender.try_send(Arc::clone(&event)) {
                Ok(()) => {
                    metrics.enqueued = metrics.enqueued.saturating_add(1);
                    true
                }
                Err(mpsc::TrySendError::Full(_)) => {
                    self.dispatcher.shared.queued[*id].fetch_sub(1, Ordering::Relaxed);
                    if end(&sender.terminal, Terminal::Overflow) {
                        metrics.end(Terminal::Overflow);
                    }
                    false
                }
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    self.dispatcher.shared.queued[*id].fetch_sub(1, Ordering::Relaxed);
                    if end(&sender.terminal, Terminal::Interrupted) {
                        metrics.end(Terminal::Interrupted);
                    }
                    false
                }
            }
        });
        if observed {
            state.metrics.observe(confirmed_at.elapsed());
        } else {
            state.metrics.unobserved = state.metrics.unobserved.saturating_add(1);
        }
        Ok(sequence)
    }
    /// Unused reserved numbers are not consumed (rejections/no-ops).
    pub fn finish(mut self) {
        self.finished = true;
    }
    pub fn interrupt(mut self) {
        self.dispatcher.close(Terminal::Interrupted);
        self.finished = true;
    }
}
impl Drop for Publication<'_> {
    fn drop(&mut self) {
        if self.published && !self.finished {
            self.dispatcher.close(Terminal::Interrupted);
        }
    }
}

pub struct Subscription {
    shared: Arc<Shared>,
    id: usize,
    receiver: mpsc::Receiver<Arc<Event>>,
    terminal: Arc<AtomicU8>,
}
impl Subscription {
    pub fn terminal_reason(&self) -> Option<Terminal> {
        terminal(&self.terminal)
    }
    pub fn terminate(&self, reason: Terminal) {
        let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        if end(&self.terminal, reason) {
            state.metrics.end(reason);
        }
        state.senders.remove(&self.id);
    }
    fn discard(&self) {
        while self.receiver.try_recv().is_ok() {
            self.shared.queued[self.id].fetch_sub(1, Ordering::Relaxed);
        }
    }
    fn received(&self, event: Option<Arc<Event>>) -> Result<Option<Arc<Event>>, Terminal> {
        if event.is_some() {
            self.shared.queued[self.id].fetch_sub(1, Ordering::Relaxed);
        }
        if let Some(reason) = terminal(&self.terminal) {
            self.discard();
            return Err(reason);
        }
        Ok(event)
    }
    pub fn recv_timeout(
        &self,
        timeout: std::time::Duration,
    ) -> Result<Option<Arc<Event>>, Terminal> {
        if let Some(reason) = terminal(&self.terminal) {
            self.discard();
            return Err(reason);
        }
        let event = match self.receiver.recv_timeout(timeout) {
            Ok(event) => Some(event),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                self.terminate(Terminal::Interrupted);
                None
            }
        };
        self.received(event)
    }
    pub fn try_next(&self) -> Result<Option<Arc<Event>>, Terminal> {
        if let Some(reason) = terminal(&self.terminal) {
            self.discard();
            return Err(reason);
        }
        let event = match self.receiver.try_recv() {
            Ok(event) => Some(event),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.terminate(Terminal::Interrupted);
                None
            }
        };
        self.received(event)
    }
}
impl Drop for Subscription {
    fn drop(&mut self) {
        {
            let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
            state.senders.remove(&self.id);
            if terminal(&self.terminal).is_none() {
                state.metrics.ended[3] = state.metrics.ended[3].saturating_add(1);
            }
        }
        self.discard();
        self.shared
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .occupied[self.id] = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn change(collection: &str) -> Change {
        Change::new(
            collection,
            "secret_id",
            Kind::Insert,
            None,
            Some(br#"{"_id":"secret_id","value":"hidden"}"#),
        )
        .unwrap()
    }
    fn publish(dispatcher: &mut Dispatcher, collection: &str) -> u64 {
        let mut publication = dispatcher.reserve(1).unwrap();
        let sequence = publication.publish_confirmed(change(collection)).unwrap();
        publication.finish();
        sequence
    }
    #[test]
    fn independent_consumers_share_event_and_keep_filtered_sequence_gaps() {
        let mut dispatcher = Dispatcher::new(3, 4).unwrap();
        let one = dispatcher.subscribe("orders").unwrap();
        let two = dispatcher.subscribe("orders").unwrap();
        assert!(one.try_next().unwrap().is_none());
        assert_eq!(publish(&mut dispatcher, "other"), 1);
        assert_eq!(publish(&mut dispatcher, "orders"), 2);
        let first = one.try_next().unwrap().unwrap();
        let second = two.try_next().unwrap().unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(first.sequence(), 2);
        assert_eq!(first.id(), "secret_id");
        assert!(first.before().is_none());
        assert!(first.after().is_some());
        let debug = format!("{first:?}");
        assert!(!debug.contains("secret"));
        assert!(!debug.contains("hidden"));
        assert!(!debug.contains("orders"));
    }
    #[test]
    fn overflow_is_terminal_discards_backlog_and_retains_slot_until_drop() {
        let mut dispatcher = Dispatcher::new(2, 1).unwrap();
        let slow = dispatcher.subscribe("orders").unwrap();
        let fast = dispatcher.subscribe("orders").unwrap();
        publish(&mut dispatcher, "orders");
        fast.try_next().unwrap().unwrap();
        publish(&mut dispatcher, "orders");
        assert_eq!(fast.try_next().unwrap().unwrap().sequence(), 2);
        assert!(matches!(slow.try_next(), Err(Terminal::Overflow)));
        assert!(matches!(
            dispatcher.subscribe("orders"),
            Err(Error::TooManySubscribers)
        ));
        drop(slow);
        let replacement = dispatcher.subscribe("orders").unwrap();
        assert!(replacement.try_next().unwrap().is_none());
        publish(&mut dispatcher, "orders");
        assert_eq!(replacement.try_next().unwrap().unwrap().sequence(), 3);
        dispatcher.shutdown();
        assert!(matches!(fast.try_next(), Err(Terminal::Shutdown)));
    }
    #[test]
    fn unused_reservations_do_not_publish_and_partial_publication_fails_closed() {
        let mut dispatcher = Dispatcher::new(1, 4).unwrap();
        let sub = dispatcher.subscribe("orders").unwrap();
        drop(dispatcher.reserve(32).unwrap());
        assert!(sub.try_next().unwrap().is_none());
        assert_eq!(publish(&mut dispatcher, "orders"), 1);
        sub.try_next().unwrap();
        {
            let mut publication = dispatcher.reserve(2).unwrap();
            publication.publish_confirmed(change("orders")).unwrap();
        }
        assert!(matches!(sub.try_next(), Err(Terminal::Interrupted)));
        assert!(matches!(dispatcher.reserve(1), Err(Error::Closed)));
        assert!(matches!(dispatcher.subscribe("orders"), Err(Error::Closed)));
    }
    #[test]
    fn sequence_exhaustion_is_detected_before_work_and_terminal_reason_is_preserved() {
        let mut dispatcher = Dispatcher::new(1, 1).unwrap();
        let sub = dispatcher.subscribe("orders").unwrap();
        dispatcher.shared.state.lock().unwrap().sequence = u64::MAX - 1;
        assert!(matches!(
            dispatcher.reserve(2),
            Err(Error::SequenceExhausted)
        ));
        assert_eq!(publish(&mut dispatcher, "orders"), u64::MAX);
        assert!(matches!(
            dispatcher.reserve(1),
            Err(Error::SequenceExhausted)
        ));
        dispatcher.shutdown();
        dispatcher.shutdown();
        assert!(matches!(sub.try_next(), Err(Terminal::Shutdown)));
    }
    #[test]
    fn invalid_shapes_identifiers_and_document_bounds_are_rejected() {
        assert!(matches!(Dispatcher::new(33, 1), Err(Error::InvalidLimits)));
        assert!(matches!(Dispatcher::new(1, 0), Err(Error::InvalidLimits)));
        let bytes = br#"{"_id":"doc","n":9007199254740993}"#;
        assert!(Change::new("orders", "doc", Kind::Insert, None, Some(bytes)).is_ok());
        assert!(Change::new("orders", "doc", Kind::Update, Some(bytes), Some(bytes)).is_ok());
        assert!(Change::new("orders", "doc", Kind::Delete, Some(bytes), None).is_ok());
        assert!(matches!(
            Change::new("orders", "doc", Kind::Insert, Some(bytes), Some(bytes)),
            Err(Error::InvalidChange)
        ));
        assert!(matches!(
            Change::new("bad/name", "doc", Kind::Insert, None, Some(bytes)),
            Err(Error::InvalidChange)
        ));
        assert!(matches!(
            Change::new("orders", "other", Kind::Insert, None, Some(bytes)),
            Err(Error::InvalidChange)
        ));
        assert!(matches!(
            Change::new(
                "orders",
                "doc",
                Kind::Insert,
                None,
                Some(&vec![b' '; DEFAULT_MAX_DOCUMENT_BYTES + 1])
            ),
            Err(Error::InvalidChange)
        ));
    }
    #[test]
    fn reconnect_receives_only_future_events_and_first_terminal_wins() {
        let mut dispatcher = Dispatcher::new(1, 1).unwrap();
        let sub = dispatcher.subscribe("orders").unwrap();
        publish(&mut dispatcher, "orders");
        publish(&mut dispatcher, "orders");
        dispatcher.shutdown();
        assert!(matches!(sub.try_next(), Err(Terminal::Overflow)));
        drop(sub);
        let mut fresh = Dispatcher::new(1, 1).unwrap();
        publish(&mut fresh, "orders");
        let sub = fresh.subscribe("orders").unwrap();
        assert!(sub.try_next().unwrap().is_none());
        assert_eq!(publish(&mut fresh, "orders"), 2);
        assert_eq!(sub.try_next().unwrap().unwrap().sequence(), 2);
    }

    #[test]
    fn consumers_on_independent_threads_observe_the_same_order() {
        let mut dispatcher = Dispatcher::new(2, 1).unwrap();
        let one = dispatcher.subscribe("orders").unwrap();
        let two = dispatcher.subscribe("orders").unwrap();
        let (sent, received) = mpsc::sync_channel(2);
        std::thread::scope(|scope| {
            let mut workers = Vec::new();
            for subscription in [one, two] {
                let sent = sent.clone();
                workers.push(scope.spawn(move || {
                    let mut sequences = Vec::new();
                    for expected in 1..=3 {
                        let deadline =
                            std::time::Instant::now() + std::time::Duration::from_secs(2);
                        loop {
                            if let Some(event) = subscription.try_next().unwrap() {
                                assert_eq!(event.sequence(), expected);
                                sequences.push(expected);
                                sent.send(()).unwrap();
                                break;
                            }
                            assert!(std::time::Instant::now() < deadline);
                            std::thread::yield_now();
                        }
                    }
                    sequences
                }));
            }
            for _ in 0..3 {
                publish(&mut dispatcher, "orders");
                for _ in 0..2 {
                    received
                        .recv_timeout(std::time::Duration::from_secs(2))
                        .unwrap();
                }
            }
            for worker in workers {
                assert_eq!(worker.join().unwrap(), vec![1, 2, 3]);
            }
        });
        assert!(dispatcher.subscribe("orders").is_ok());
    }

    #[test]
    fn unobserved_advancement_cannot_skip_an_active_collection() {
        let mut dispatcher = Dispatcher::new(2, 4).unwrap();
        let sub = dispatcher.subscribe("orders").unwrap();
        assert!(dispatcher.observes("orders").unwrap());
        let mut publication = dispatcher.reserve(2).unwrap();
        assert!(matches!(
            publication.advance_unobserved_confirmed("orders"),
            Err(Error::InvalidChange)
        ));
        assert_eq!(
            publication.advance_unobserved_confirmed("other").unwrap(),
            1
        );
        publication.publish_confirmed(change("orders")).unwrap();
        publication.finish();
        assert_eq!(sub.try_next().unwrap().unwrap().sequence(), 2);
        dispatcher.interrupt_collection("orders");
        assert!(matches!(sub.try_next(), Err(Terminal::Interrupted)));
        assert!(!dispatcher.observes("orders").unwrap());
        assert!(dispatcher.subscribe("other").is_ok());
    }

    #[test]
    fn product_limits_allow_thirty_two_handles_and_sixteen_events() {
        let mut dispatcher = Dispatcher::new(32, 16).unwrap();
        let handles: Vec<_> = (0..32)
            .map(|_| dispatcher.subscribe("orders").unwrap())
            .collect();
        assert!(matches!(
            dispatcher.subscribe("orders"),
            Err(Error::TooManySubscribers)
        ));
        for _ in 0..16 {
            publish(&mut dispatcher, "orders");
        }
        publish(&mut dispatcher, "orders");
        for handle in &handles {
            assert!(matches!(handle.try_next(), Err(Terminal::Overflow)));
        }
        assert!(matches!(
            dispatcher.subscribe("orders"),
            Err(Error::TooManySubscribers)
        ));
    }
}
