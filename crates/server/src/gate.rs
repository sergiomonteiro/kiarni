use std::{
    collections::VecDeque,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use kiarni_http::Code;

#[derive(Default)]
struct State {
    active: bool,
    waiting: VecDeque<Arc<Condvar>>,
}

/// FIFO waiters already own connection permits, bounding the queue.
pub(crate) struct Gate {
    state: Mutex<State>,
    timeout: Duration,
}

impl Default for Gate {
    fn default() -> Self {
        Self {
            state: Mutex::default(),
            timeout: Duration::from_millis(100),
        }
    }
}

pub(crate) struct Permit<'a>(&'a Gate);

impl Gate {
    pub(crate) fn enter(&self, stop: &AtomicBool, busy: Code) -> Result<Permit<'_>, Code> {
        let deadline = Instant::now() + self.timeout;
        let mut state = self.state.lock().map_err(|_| Code::ServiceUnavailable)?;
        if stop.load(Ordering::Relaxed) {
            return Err(Code::ShuttingDown);
        }
        if !state.active && state.waiting.is_empty() {
            state.active = true;
            return Ok(Permit(self));
        }
        let waiter = Arc::new(Condvar::new());
        state.waiting.push_back(Arc::clone(&waiter));
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if stop.load(Ordering::Relaxed) || remaining.is_zero() {
                state.waiting.retain(|entry| !Arc::ptr_eq(entry, &waiter));
                if !state.active
                    && let Some(next) = state.waiting.front()
                {
                    next.notify_one();
                }
                return Err(if stop.load(Ordering::Relaxed) {
                    Code::ShuttingDown
                } else {
                    busy
                });
            }
            if !state.active
                && state
                    .waiting
                    .front()
                    .is_some_and(|entry| Arc::ptr_eq(entry, &waiter))
            {
                state.waiting.pop_front();
                state.active = true;
                return Ok(Permit(self));
            }
            let (next, _) = waiter
                .wait_timeout(state, remaining.min(Duration::from_millis(25)))
                .map_err(|_| Code::ServiceUnavailable)?;
            state = next;
        }
    }
}

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        let mut state = self
            .0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.active = false;
        if let Some(next) = state.waiting.front() {
            next.notify_one();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queued_operations_are_admitted_in_arrival_order() {
        let gate = Gate {
            timeout: Duration::from_secs(5),
            ..Gate::default()
        };
        let stop = AtomicBool::new(false);
        let held = gate.enter(&stop, Code::WriteBusy).unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            let mut workers = Vec::new();
            for index in 0..3 {
                let sender = &sender;
                let gate = &gate;
                let stop = &stop;
                workers.push(scope.spawn(move || {
                    let _permit = gate.enter(stop, Code::WriteBusy).unwrap();
                    sender.send(index).unwrap();
                }));
                let deadline = Instant::now() + Duration::from_secs(2);
                while gate.state.lock().unwrap().waiting.len() != index + 1 {
                    assert!(Instant::now() < deadline, "waiter did not enqueue");
                    std::thread::yield_now();
                }
            }
            drop(held);
            for expected in 0..3 {
                assert_eq!(
                    receiver.recv_timeout(Duration::from_secs(2)).unwrap(),
                    expected
                );
            }
            for worker in workers {
                worker.join().unwrap();
            }
        });
    }

    #[test]
    fn waiting_operation_proceeds_after_release() {
        let gate = Gate::default();
        let stop = AtomicBool::new(false);
        let held = gate.enter(&stop, Code::WriteBusy).unwrap();
        std::thread::scope(|scope| {
            let worker = scope.spawn(|| {
                let _permit = gate.enter(&stop, Code::WriteBusy).unwrap();
            });
            drop(held);
            worker.join().unwrap();
        });
        assert!(gate.enter(&stop, Code::WriteBusy).is_ok());
    }

    #[test]
    fn timeout_and_shutdown_leave_the_gate_usable() {
        let gate = Gate::default();
        let stop = AtomicBool::new(false);
        let held = gate.enter(&stop, Code::WriteBusy).unwrap();
        assert!(matches!(
            gate.enter(&stop, Code::WriteBusy),
            Err(Code::WriteBusy)
        ));
        stop.store(true, Ordering::Relaxed);
        assert!(matches!(
            gate.enter(&stop, Code::WriteBusy),
            Err(Code::ShuttingDown)
        ));
        drop(held);
        stop.store(false, Ordering::Relaxed);
        assert!(gate.enter(&stop, Code::WriteBusy).is_ok());
    }
}
