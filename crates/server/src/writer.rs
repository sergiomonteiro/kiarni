//! Bounded product coordinator used by all HTTP mutation handlers (ADR 0017).
mod execution;
#[cfg(test)]
mod tests;
use execution::execute;

use kiarni_core::{DEFAULT_MAX_DOCUMENT_BYTES, PreparedInsert};
use kiarni_engine::writes::{Operation, PreparedWrite};
use kiarni_http::Code;
use kiarni_storage::{Store, updates};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub enum Command {
    Update {
        collection: String,
        id: String,
        operation: Operation,
        body: Vec<u8>,
    },
    Insert {
        collection: String,
        body: Vec<u8>,
    },
    Delete {
        collection: String,
        id: String,
    },
    CreateCollection {
        name: String,
    },
    DropCollection {
        name: String,
    },
    Subscribe {
        collection: String,
    },
}
impl Command {
    fn owned_bytes(&self) -> usize {
        match self {
            Self::Update {
                collection,
                id,
                body,
                ..
            } => collection.capacity() + id.capacity() + body.capacity(),
            Self::Insert { collection, body } => collection.capacity() + body.capacity(),
            Self::Delete { collection, id } => collection.capacity() + id.capacity(),
            Self::CreateCollection { name } | Self::DropCollection { name } => name.capacity(),
            Self::Subscribe { collection } => collection.capacity(),
        }
    }
    fn body_bytes(&self) -> usize {
        match self {
            Self::Update { body, .. } | Self::Insert { body, .. } => body.len(),
            _ => 0,
        }
    }
    fn is_update(&self) -> bool {
        matches!(self, Self::Update { .. })
    }
    fn validate(&self, max_document: usize) -> Result<(), Code> {
        let name_ok = |name: &str| {
            !name.is_empty()
                && name.len() <= 128
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        };
        let (collection, id) = match self {
            Self::Update { collection, id, .. } | Self::Delete { collection, id } => {
                (collection, Some(id))
            }
            Self::Insert { collection, .. } => (collection, None),
            Self::Subscribe { collection } => (collection, None),
            Self::CreateCollection { name } | Self::DropCollection { name } => (name, None),
        };
        if !name_ok(collection) {
            return Err(Code::InvalidCollection);
        }
        if id.is_some_and(|id| !name_ok(id)) {
            return Err(Code::InvalidId);
        }
        if self.body_bytes() > max_document {
            return Err(Code::DocumentTooLarge);
        }
        // Waiting requests already own these allocations. Keep their per-request
        // footprint bounded even before admission to the shared 8MiB budget.
        let body_capacity = match self {
            Self::Update { body, .. } | Self::Insert { body, .. } => body.capacity(),
            _ => 0,
        };
        if collection.capacity() > 128
            || id.is_some_and(|id| id.capacity() > 128)
            || body_capacity > DEFAULT_MAX_DOCUMENT_BYTES
        {
            return Err(Code::WriteBusy);
        }
        Ok(())
    }
}

pub enum Outcome {
    Updated(PreparedWrite),
    Inserted(PreparedInsert),
    Created,
    Deleted,
    Dropped,
    Subscribed(kiarni_feeds::Subscription),
}

/// JSON results stay borrowed from the reply so its reservation covers their lifetime.
pub struct Reply {
    outcome: Result<Outcome, Code>,
    sequence: u64,
    group_size: usize,
    _lease: Lease,
}
impl Reply {
    pub fn outcome(&self) -> Result<&Outcome, Code> {
        self.outcome.as_ref().map_err(|code| *code)
    }
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
    pub fn group_size(&self) -> usize {
        self.group_size
    }
}

#[derive(Clone, Copy)]
pub struct Policy {
    pub max_requests: usize,
    pub max_document_bytes: usize,
    pub max_subscribers: usize,
    pub feed_capacity: usize,
}
impl Policy {
    fn valid(self) -> bool {
        (1..=128).contains(&self.max_requests)
            && (1..=DEFAULT_MAX_DOCUMENT_BYTES).contains(&self.max_document_bytes)
            && (1..=32).contains(&self.max_subscribers)
            && (1..=16).contains(&self.feed_capacity)
    }
}
const INPUT_BUDGET: usize = 8 * 1024 * 1024;
pub const WORKER_STACK_BYTES: usize = 768 * 1024;
const ADMISSION_TIMEOUT: Duration = Duration::from_millis(100);
const STOP_POLL: Duration = Duration::from_millis(25);

struct Job {
    id: u64,
    command: Command,
    deadline: Instant,
    sender: mpsc::SyncSender<Reply>,
    lease: Lease,
}
#[derive(Default)]
struct State {
    next: u64,
    requests: usize,
    admitted: usize,
    bytes: usize,
    waiting: VecDeque<u64>,
    ready: VecDeque<Job>,
    closed: Option<Code>,
}
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    stop: Arc<AtomicBool>,
    policy: Policy,
    feed_metrics: kiarni_feeds::Metrics,
}
struct Lease {
    shared: Arc<Shared>,
    id: u64,
    admitted_bytes: Option<usize>,
    _request: Option<Arc<kiarni_config::Permit>>,
}
impl Drop for Lease {
    fn drop(&mut self) {
        let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        state.waiting.retain(|id| *id != self.id);
        state.requests -= 1;
        if let Some(bytes) = self.admitted_bytes {
            state.admitted -= 1;
            state.bytes -= bytes;
        }
        self.shared.changed.notify_all();
    }
}

pub struct Ticket {
    shared: Arc<Shared>,
    id: u64,
    deadline: Instant,
    receiver: mpsc::Receiver<Reply>,
}
impl Ticket {
    pub fn sequence(&self) -> u64 {
        self.id
    }
    pub fn wait(self) -> Result<Reply, Code> {
        match self
            .receiver
            .recv_timeout(self.deadline.saturating_duration_since(Instant::now()))
        {
            Ok(reply) => Ok(reply),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(Code::StorageUnavailable),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if let Some(job) = remove_queued(&self.shared, self.id) {
                    drop(job);
                    return Err(if self.shared.stop.load(Ordering::Relaxed) {
                        Code::ShuttingDown
                    } else {
                        Code::WriteBusy
                    });
                }
                // Already started: the admission deadline cannot cancel its commit.
                self.receiver.recv().map_err(|_| Code::StorageUnavailable)
            }
        }
    }
}
impl Drop for Ticket {
    fn drop(&mut self) {
        drop(remove_queued(&self.shared, self.id));
    }
}
fn remove_queued(shared: &Shared, id: u64) -> Option<Job> {
    let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
    let position = state.ready.iter().position(|job| job.id == id)?;
    state.ready.remove(position)
}

#[derive(Clone)]
pub struct Client {
    shared: Arc<Shared>,
}
impl Client {
    pub fn feed_metrics(&self) -> kiarni_feeds::Snapshot {
        self.shared.feed_metrics.snapshot()
    }

    /// Transfers request buffers; waits at most 100ms for FIFO budget admission.
    pub fn submit(&self, command: Command) -> Result<Ticket, Code> {
        self.submit_reserved(command, None)
    }
    /// Share the connection's admission slot until command and reply buffers are gone.
    pub fn submit_with_permit(
        &self,
        command: Command,
        permit: Arc<kiarni_config::Permit>,
    ) -> Result<Ticket, Code> {
        self.submit_reserved(command, Some(permit))
    }
    fn submit_reserved(
        &self,
        command: Command,
        permit: Option<Arc<kiarni_config::Permit>>,
    ) -> Result<Ticket, Code> {
        let deadline = Instant::now() + ADMISSION_TIMEOUT;
        command.validate(self.shared.policy.max_document_bytes)?;
        let bytes = command.owned_bytes();
        if bytes > INPUT_BUDGET {
            return Err(Code::WriteBusy);
        }
        let id = {
            let mut state = self
                .shared
                .state
                .lock()
                .map_err(|_| Code::ServiceUnavailable)?;
            if self.shared.stop.load(Ordering::Relaxed) {
                return Err(Code::ShuttingDown);
            }
            if let Some(code) = state.closed {
                return Err(code);
            }
            if state.requests >= self.shared.policy.max_requests {
                return Err(Code::WriteBusy);
            }
            let id = state.next.checked_add(1).ok_or(Code::ServiceUnavailable)?;
            state.next = id;
            state.requests += 1;
            state.waiting.push_back(id);
            id
        };
        let mut lease = Lease {
            shared: Arc::clone(&self.shared),
            id,
            admitted_bytes: None,
            _request: permit,
        };
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| Code::ServiceUnavailable)?;
        loop {
            if self.shared.stop.load(Ordering::Relaxed) {
                return Err(Code::ShuttingDown);
            }
            if let Some(code) = state.closed {
                return Err(code);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(Code::WriteBusy);
            }
            if state.waiting.front() == Some(&id)
                && state.admitted < self.shared.policy.max_requests.min(64)
                && bytes <= INPUT_BUDGET - state.bytes
            {
                state.waiting.pop_front();
                state.admitted += 1;
                state.bytes += bytes;
                lease.admitted_bytes = Some(bytes);
                let (sender, receiver) = mpsc::sync_channel(1);
                state.ready.push_back(Job {
                    id,
                    command,
                    deadline,
                    sender,
                    lease,
                });
                self.shared.changed.notify_all();
                return Ok(Ticket {
                    shared: Arc::clone(&self.shared),
                    id,
                    deadline,
                    receiver,
                });
            }
            let (next, _) = self
                .shared
                .changed
                .wait_timeout(state, remaining.min(STOP_POLL))
                .map_err(|_| Code::ServiceUnavailable)?;
            state = next;
        }
    }
}

pub struct Coordinator {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}
impl Coordinator {
    pub fn start(store: Arc<Store>, policy: Policy, stop: Arc<AtomicBool>) -> Result<Self, Code> {
        Self::start_with_hook(store, policy, stop, || {})
    }
    fn start_with_hook(
        store: Arc<Store>,
        policy: Policy,
        stop: Arc<AtomicBool>,
        hook: impl FnMut() + Send + 'static,
    ) -> Result<Self, Code> {
        Self::start_with_hooks(store, policy, stop, hook, || Ok(()))
    }
    fn start_with_hooks(
        store: Arc<Store>,
        policy: Policy,
        stop: Arc<AtomicBool>,
        hook: impl FnMut() + Send + 'static,
        after_commit: impl FnMut() -> Result<(), Code> + Send + 'static,
    ) -> Result<Self, Code> {
        if !policy.valid() {
            return Err(Code::InvalidLimit);
        }
        let dispatcher =
            kiarni_feeds::Dispatcher::new(policy.max_subscribers, policy.feed_capacity)
                .map_err(|_| Code::InvalidLimit)?;
        let feed_metrics = dispatcher.metrics();
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
            stop,
            policy,
            feed_metrics,
        });
        let worker_shared = Arc::clone(&shared);
        let worker = thread::Builder::new()
            .name("kiarni-writer".into())
            .stack_size(WORKER_STACK_BYTES)
            .spawn(move || run(store, worker_shared, hook, after_commit, dispatcher))
            .map_err(|_| Code::ServiceUnavailable)?;
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
    pub fn close(&self) {
        close(&self.shared, Code::ShuttingDown);
    }
    pub fn shutdown(mut self) -> Result<(), Code> {
        self.finish()
    }
    fn finish(&mut self) -> Result<(), Code> {
        self.close();
        if let Some(worker) = self.worker.take() {
            worker.join().map_err(|_| Code::StorageUnavailable)?;
        }
        Ok(())
    }
}
impl Drop for Coordinator {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}
fn deliver(job: Job, outcome: Result<Outcome, Code>, group_size: usize) {
    let Job {
        id,
        sender,
        lease,
        command,
        ..
    } = job;
    drop(command);
    let _ = sender.try_send(Reply {
        outcome,
        sequence: id,
        group_size,
        _lease: lease,
    });
}
fn close(shared: &Shared, code: Code) {
    let (jobs, code) = {
        let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
        let code = *state.closed.get_or_insert(code);
        let jobs = std::mem::take(&mut state.ready);
        shared.changed.notify_all();
        (jobs, code)
    };
    for job in jobs {
        deliver(job, Err(code), 0);
    }
}
struct WorkerGuard(Arc<Shared>);
impl Drop for WorkerGuard {
    fn drop(&mut self) {
        close(&self.0, Code::StorageUnavailable);
    }
}

fn run(
    store: Arc<Store>,
    shared: Arc<Shared>,
    mut hook: impl FnMut(),
    mut after_commit: impl FnMut() -> Result<(), Code>,
    mut dispatcher: kiarni_feeds::Dispatcher,
) {
    let _guard = WorkerGuard(Arc::clone(&shared));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_inner(
            &store,
            &shared,
            &mut hook,
            &mut dispatcher,
            &mut after_commit,
        )
    }));
    if let Err(panic) = result {
        dispatcher.interrupt();
        std::panic::resume_unwind(panic);
    }
    dispatcher.shutdown();
}
fn run_inner(
    store: &Store,
    shared: &Shared,
    hook: &mut impl FnMut(),
    dispatcher: &mut kiarni_feeds::Dispatcher,
    after_commit: &mut impl FnMut() -> Result<(), Code>,
) {
    loop {
        let mut expired = Vec::new();
        let group = {
            let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            while state.ready.is_empty()
                && state.closed.is_none()
                && !shared.stop.load(Ordering::Relaxed)
            {
                state = shared
                    .changed
                    .wait_timeout(state, STOP_POLL)
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
            }
            if shared.stop.load(Ordering::Relaxed) {
                drop(state);
                close(shared, Code::ShuttingDown);
                return;
            }
            if state.closed.is_some() {
                return;
            }
            let mut group: Vec<Job> = Vec::with_capacity(updates::MAX_OPERATIONS);
            let mut bytes = 0;
            while let Some(next) = state.ready.front() {
                if next.deadline <= Instant::now() {
                    if let Some(job) = state.ready.pop_front() {
                        expired.push(job);
                    }
                    continue;
                }
                if !group.is_empty()
                    && (!next.command.is_update()
                        || !group[0].command.is_update()
                        || bytes + next.command.body_bytes() > updates::MAX_BODY_BYTES)
                {
                    break;
                }
                if let Some(job) = state.ready.pop_front() {
                    bytes += job.command.body_bytes();
                    group.push(job);
                }
                if group.len() == updates::MAX_OPERATIONS || !group[0].command.is_update() {
                    break;
                }
            }
            group
        };
        for job in expired {
            deliver(job, Err(Code::WriteBusy), 0);
        }
        if group.is_empty() {
            continue;
        }
        hook(); // Private test seam after the group is atomically marked started.
        let size = group.len();
        let (results, fatal) = execute(
            store,
            &group,
            shared.policy.max_document_bytes,
            dispatcher,
            after_commit,
        );
        if fatal {
            dispatcher.interrupt();
            close(shared, Code::StorageUnavailable);
        }
        for (job, result) in group.into_iter().zip(results) {
            deliver(job, result, size);
        }
        if fatal {
            return;
        }
    }
}
