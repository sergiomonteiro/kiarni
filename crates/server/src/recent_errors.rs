//! A bounded, best-effort tail of typed request failures, independent of logging.
use crate::logging::Request;
use kiarni_http::{Code, Operation};
use std::{
    sync::{
        Mutex, TryLockError,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
const CAPACITY: usize = 8;
#[derive(Clone, Copy)]
struct Entry {
    id: [u8; 36],
    method: &'static str,
    operation: Option<Operation>,
    status: u16,
    error: Option<Code>,
    duration_us: u64,
    timestamp_ms: Option<u64>,
    transport: &'static str,
    terminal: Option<&'static str>,
}
struct Ring {
    entries: [Option<Entry>; CAPACITY],
    next: usize,
    len: usize,
}
impl Default for Ring {
    fn default() -> Self {
        Self {
            entries: [None; CAPACITY],
            next: 0,
            len: 0,
        }
    }
}
#[derive(Default)]
pub(crate) struct History {
    ring: Mutex<Ring>,
    dropped: AtomicU64,
}
impl History {
    fn drop_observation(&self) {
        let _ = self
            .dropped
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
                Some(v.saturating_add(1))
            });
    }
    pub fn observe(&self, request: &Request<'_>) {
        if request.status < 400
            && request.transport != "write_failed"
            && !matches!(request.terminal, Some("overflow" | "interrupted"))
        {
            return;
        }
        let Ok(id) = request.id.as_str().as_bytes().try_into() else {
            self.drop_observation();
            return;
        };
        let entry = Entry {
            id,
            method: request.method,
            operation: request.operation,
            status: request.status,
            error: request.error,
            duration_us: request.elapsed.as_micros().min(u64::MAX as u128) as u64,
            timestamp_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .ok()
                .and_then(|v| u64::try_from(v.as_millis()).ok()),
            transport: request.transport,
            terminal: request.terminal,
        };
        let mut ring = match self.ring.try_lock() {
            Ok(ring) => ring,
            Err(TryLockError::Poisoned(error)) => error.into_inner(),
            Err(TryLockError::WouldBlock) => {
                self.drop_observation();
                return;
            }
        };
        let next = ring.next;
        ring.entries[next] = Some(entry);
        ring.next = (next + 1) % CAPACITY;
        ring.len = (ring.len + 1).min(CAPACITY);
    }
    pub fn json(&self) -> Result<Vec<u8>, Code> {
        let (entries, next, len) = {
            let ring = self.ring.lock().unwrap_or_else(|error| error.into_inner());
            (ring.entries, ring.next, ring.len)
        };
        let mut errors = Vec::with_capacity(len);
        for offset in 0..len {
            let entry =
                entries[(next + CAPACITY - 1 - offset) % CAPACITY].ok_or(Code::InternalError)?;
            errors.push(serde_json::json!({
                "request_id": std::str::from_utf8(&entry.id).map_err(|_| Code::InternalError)?,
                "timestamp_ms": entry.timestamp_ms.map(|v| v.to_string()),
                "method": entry.method,
                "operation": entry.operation.map(|v| format!("{v:?}")),
                "status": entry.status,
                "error_code": entry.error.map(|v| v.spec().0),
                "duration_us": entry.duration_us.to_string(),
                "transport": entry.transport,
                "stream_end_reason": entry.terminal,
            }));
        }
        serde_json::to_vec(&serde_json::json!({
            "capacity": CAPACITY, "dropped_total": self.dropped.load(Ordering::Relaxed).to_string(), "errors": errors,
        })).map_err(|_| Code::InternalError)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    fn observe(
        history: &History,
        status: u16,
        transport: &'static str,
        terminal: Option<&'static str>,
    ) -> String {
        let id = kiarni_http::RequestId::generate().unwrap();
        history.observe(&Request {
            id: &id,
            method: "POST",
            operation: Some(Operation::Query),
            status,
            error: (status >= 400).then_some(Code::InvalidQuery),
            elapsed: Duration::from_micros(42),
            transport,
            terminal,
        });
        id.as_str().to_owned()
    }
    fn snapshot(history: &History) -> serde_json::Value {
        serde_json::from_slice(&history.json().unwrap()).unwrap()
    }
    #[test]
    fn bounded_tail_is_newest_first_and_has_only_safe_metadata() {
        let history = History::default();
        assert_eq!(snapshot(&history)["errors"], serde_json::json!([]));
        let ids: Vec<_> = (0..12)
            .map(|_| observe(&history, 422, "written", None))
            .collect();
        let value = snapshot(&history);
        assert_eq!(value["capacity"], 8);
        let errors = value["errors"].as_array().unwrap();
        assert_eq!(errors.len(), 8);
        for (entry, id) in errors.iter().zip(ids[4..].iter().rev()) {
            assert_eq!(entry["request_id"], *id);
            assert_eq!(entry["duration_us"], "42");
            assert_eq!(entry["operation"], "Query");
            assert_eq!(entry["error_code"], Code::InvalidQuery.spec().0);
            assert_eq!(entry.as_object().unwrap().len(), 9);
        }
        assert!(history.json().unwrap().len() < 4096);
        assert_eq!(
            snapshot(&History::default())["errors"],
            serde_json::json!([])
        );
    }
    #[test]
    fn ordinary_success_and_shutdown_are_excluded_but_transport_failures_enter() {
        let history = History::default();
        observe(&history, 200, "written", None);
        observe(&history, 200, "stream_closed", Some("shutdown"));
        assert_eq!(snapshot(&history)["errors"].as_array().unwrap().len(), 0);
        observe(&history, 200, "write_failed", None);
        observe(&history, 200, "stream_closed", Some("overflow"));
        observe(&history, 200, "stream_closed", Some("interrupted"));
        assert_eq!(snapshot(&history)["errors"].as_array().unwrap().len(), 3);
    }
    #[test]
    fn producer_never_waits_for_busy_ring_and_reports_loss() {
        let history = History::default();
        let guard = history.ring.lock().unwrap();
        observe(&history, 500, "written", None);
        drop(guard);
        assert_eq!(snapshot(&history)["dropped_total"], "1");
        assert_eq!(snapshot(&history)["errors"].as_array().unwrap().len(), 0);
        history.dropped.store(u64::MAX, Ordering::Relaxed);
        let guard = history.ring.lock().unwrap();
        observe(&history, 500, "written", None);
        drop(guard);
        assert_eq!(snapshot(&history)["dropped_total"], u64::MAX.to_string());
    }
}
