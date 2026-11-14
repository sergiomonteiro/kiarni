//! Typed request telemetry: no client strings except server-generated request IDs.
use kiarni_config::LogLevel;
use kiarni_http::{Code, Operation};
use serde::Serialize;
use std::{
    fs::File,
    io::{self, Write},
    os::fd::AsFd,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub(crate) const STACK_BYTES: usize = 128 * 1024;
const QUEUE: usize = 32;

#[derive(Serialize)]
pub(crate) struct Record {
    timestamp_ms: Option<String>,
    event: &'static str,
    level: &'static str,
    request_id: String,
    method: &'static str,
    operation: Option<String>,
    status: u16,
    error_code: Option<&'static str>,
    duration_us: String,
    transport: &'static str,
    stream_end_reason: Option<&'static str>,
    dropped_before: u64,
}

pub(crate) struct Request<'a> {
    pub id: &'a kiarni_http::RequestId,
    pub method: &'static str,
    pub operation: Option<Operation>,
    pub status: u16,
    pub error: Option<Code>,
    pub elapsed: Duration,
    pub transport: &'static str,
    pub terminal: Option<&'static str>,
}

#[derive(Clone)]
pub(crate) struct Client {
    level: LogLevel,
    sender: Option<SyncSender<Record>>,
    dropped: Arc<AtomicU64>,
}
pub(crate) struct Logger {
    client: Client,
    done: mpsc::Receiver<()>,
    worker: Option<JoinHandle<()>>,
}
impl Logger {
    pub fn start(level: LogLevel) -> io::Result<Self> {
        if level == LogLevel::Off {
            let (_, done) = mpsc::channel();
            return Ok(Self {
                client: Client {
                    level,
                    sender: None,
                    dropped: Arc::default(),
                },
                done,
                worker: None,
            });
        }
        let sink = File::from(rustix::io::dup(io::stderr().as_fd())?);
        Self::with_sink(level, sink)
    }
    fn with_sink(level: LogLevel, mut sink: impl Write + Send + 'static) -> io::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel::<Record>(QUEUE);
        let (finished, done) = mpsc::channel();
        let dropped = Arc::new(AtomicU64::new(0));
        let lost = Arc::clone(&dropped);
        let worker = thread::Builder::new()
            .name("kiarni-logs".into())
            .stack_size(STACK_BYTES)
            .spawn(move || {
                for mut record in receiver {
                    record.dropped_before = lost.swap(0, Ordering::Relaxed);
                    let Ok(mut bytes) = serde_json::to_vec(&record) else {
                        continue;
                    };
                    bytes.push(b'\n');
                    if sink.write_all(&bytes).is_err() {
                        break;
                    }
                }
                let _ = finished.send(());
            })?;
        Ok(Self {
            client: Client {
                level,
                sender: Some(sender),
                dropped,
            },
            done,
            worker: Some(worker),
        })
    }
    pub fn client(&self) -> Client {
        self.client.clone()
    }
}
impl Drop for Logger {
    fn drop(&mut self) {
        self.client.sender.take();
        if self.worker.is_some()
            && self.done.recv_timeout(Duration::from_millis(50)).is_ok()
            && let Some(worker) = self.worker.take()
        {
            let _ = worker.join();
        }
        // Drop JoinHandle detaches a blocked sink; it cannot hold process shutdown.
    }
}
impl Client {
    pub fn record(&self, request: Request<'_>) {
        let level = if request.status >= 500 {
            LogLevel::Error
        } else if request.status >= 400
            || request.transport == "write_failed"
            || matches!(request.terminal, Some("overflow" | "interrupted"))
        {
            LogLevel::Warn
        } else {
            LogLevel::Info
        };
        if self.level < level {
            return;
        }
        let Some(sender) = &self.sender else {
            return;
        };
        let record = Record {
            timestamp_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .ok()
                .map(|value| value.as_millis().to_string()),
            event: "request_complete",
            level: match level {
                LogLevel::Error => "error",
                LogLevel::Warn => "warn",
                _ => "info",
            },
            request_id: request.id.as_str().to_owned(),
            method: request.method,
            operation: request.operation.map(|value| format!("{value:?}")),
            status: request.status,
            error_code: request.error.map(|value| value.spec().0),
            duration_us: request.elapsed.as_micros().to_string(),
            transport: request.transport,
            stream_end_reason: request.terminal,
            dropped_before: 0,
        };
        if sender.try_send(record).is_err() {
            let _ = self
                .dropped
                .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                    Some(value.saturating_add(1))
                });
        }
    }
}
pub(crate) fn method(value: &str) -> &'static str {
    match value {
        "GET" => "GET",
        "HEAD" => "HEAD",
        "POST" => "POST",
        "PUT" => "PUT",
        "PATCH" => "PATCH",
        "DELETE" => "DELETE",
        "OPTIONS" => "OPTIONS",
        _ => "OTHER",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);
    impl Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    fn record(client: &Client, status: u16) {
        client.record(Request {
            id: &kiarni_http::RequestId::generate().unwrap(),
            method: "GET",
            operation: Some(Operation::Query),
            status,
            error: if status >= 400 {
                Some(Code::InvalidQuery)
            } else {
                None
            },
            elapsed: Duration::from_micros(123),
            transport: "written",
            terminal: None,
        });
    }
    #[test]
    fn severity_filtering_and_schema_do_not_accept_arbitrary_input() {
        let sink = Capture::default();
        let logger = Logger::with_sink(LogLevel::Warn, sink.clone()).unwrap();
        record(&logger.client, 200);
        record(&logger.client, 422);
        record(&logger.client, 503);
        drop(logger);
        let bytes = sink.0.lock().unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        let lines: Vec<serde_json::Value> = text
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["level"], "warn");
        assert_eq!(lines[1]["level"], "error");
        assert_eq!(lines[0]["duration_us"], "123");
        assert_eq!(lines[0]["operation"], "Query");
        assert_eq!(method("SECRET_METHOD"), "OTHER");
        assert!(text.lines().all(|line| line.len() < 512));
        assert!(std::mem::size_of::<Record>() * (QUEUE + 1) < 16 * 1024);
    }
    struct Blocked {
        entered: Option<mpsc::Sender<()>>,
        release: mpsc::Receiver<()>,
        capture: Capture,
    }
    impl Write for Blocked {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if let Some(entered) = self.entered.take() {
                let _ = entered.send(());
                let _ = self.release.recv();
            }
            self.capture.write(bytes)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    #[test]
    fn blocked_sink_drops_at_queue_bound_and_shutdown_does_not_wait_for_sink() {
        let (entered, ready) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let logger = Logger::with_sink(
            LogLevel::Info,
            Blocked {
                entered: Some(entered),
                release: wait,
                capture: Capture::default(),
            },
        )
        .unwrap();
        record(&logger.client, 200);
        ready.recv_timeout(Duration::from_secs(2)).unwrap();
        for _ in 0..QUEUE {
            record(&logger.client, 200);
        }
        record(&logger.client, 200);
        assert_eq!(logger.client.dropped.load(Ordering::Relaxed), 1);
        let started = std::time::Instant::now();
        drop(logger);
        assert!(started.elapsed() < Duration::from_secs(1));
        release.send(()).unwrap();
    }
    #[test]
    fn next_emitted_record_reports_queue_drops() {
        let (entered, ready) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let capture = Capture::default();
        let mut logger = Logger::with_sink(
            LogLevel::Info,
            Blocked {
                entered: Some(entered),
                release: wait,
                capture: capture.clone(),
            },
        )
        .unwrap();
        record(&logger.client, 200);
        ready.recv_timeout(Duration::from_secs(2)).unwrap();
        for _ in 0..QUEUE {
            record(&logger.client, 200);
        }
        record(&logger.client, 200);
        release.send(()).unwrap();
        logger.client.sender.take();
        logger.done.recv_timeout(Duration::from_secs(2)).unwrap();
        let bytes = capture.0.lock().unwrap();
        let records: Vec<serde_json::Value> = std::str::from_utf8(&bytes)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(records.len(), QUEUE + 1);
        assert_eq!(
            records
                .iter()
                .map(|value| value["dropped_before"].as_u64().unwrap())
                .sum::<u64>(),
            1
        );
    }
}
