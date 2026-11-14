//! Loopback-only HTTP/1.1 GET fixture. Not a general HTTP server or product API.
#![cfg(unix)]

use kiarni_auth_spike::{Authority, Keyset, Session};
use kiarni_core::{DEFAULT_MAX_DOCUMENT_BYTES, DocumentInput};
use kiarni_feed_spike::{
    Coordinator, Mutation, ReceiveError, Subscription,
    sse::{self, Terminal},
};
use kiarni_http_contract_spike::{
    Code, Operation, RequestId, authenticate_and_resolve, error_response,
};
use std::{
    io::{self, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use zeroize::Zeroizing;

const WORKERS: usize = 8;
const STREAMS: usize = 4;
const HEADER_LIMIT: usize = 8192;
const POLL: Duration = Duration::from_millis(25);

#[derive(Debug, Clone, Copy)]
pub enum Error {
    Io,
    InvalidConfig,
    Auth,
    Feed,
    InvalidDocument,
}

#[derive(Default)]
pub struct Counters {
    connections: AtomicUsize,
    streams: AtomicUsize,
    subscriptions: AtomicUsize,
    unauthorized: AtomicUsize,
    write_timeouts: AtomicUsize,
    overflows: AtomicUsize,
    rejected: AtomicUsize,
    event_writes: AtomicUsize,
}

impl Counters {
    pub fn snapshot(&self) -> serde_json::Value {
        let load = |value: &AtomicUsize| value.load(Ordering::SeqCst);
        serde_json::json!({
            "connections":load(&self.connections), "streams":load(&self.streams),
            "subscriptions":load(&self.subscriptions), "unauthorized":load(&self.unauthorized),
            "write_timeouts":load(&self.write_timeouts), "overflows":load(&self.overflows),
            "rejected":load(&self.rejected),
            "event_writes":load(&self.event_writes),
        })
    }
}

struct Shared {
    coordinator: Coordinator,
    authority: Authority,
    keys: Mutex<Keyset>,
    host: String,
    stop: AtomicBool,
    counters: Arc<Counters>,
    write_deadline: Duration,
    // One-shot scheduling hook owned by the local test harness, not HTTP.
    event_write_pause: Mutex<Option<(mpsc::SyncSender<()>, mpsc::Receiver<()>)>>,
}

pub struct Server {
    shared: Arc<Shared>,
    address: SocketAddr,
    accept: Option<JoinHandle<()>>,
    workers: Vec<JoinHandle<()>>,
}

pub fn now() -> Result<u64, Error> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|time| time.as_secs())
        .map_err(|_| Error::Auth)
}

impl Server {
    pub fn start(
        path: &Path,
        keys: Keyset,
        capacity: usize,
        write_deadline: Duration,
    ) -> Result<Self, Error> {
        if write_deadline < Duration::from_millis(10) || write_deadline > Duration::from_secs(2) {
            return Err(Error::InvalidConfig);
        }
        let coordinator = Coordinator::open(path, capacity).map_err(|_| Error::Feed)?;
        let listener =
            TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).map_err(|_| Error::Io)?;
        listener.set_nonblocking(true).map_err(|_| Error::Io)?;
        let address = listener.local_addr().map_err(|_| Error::Io)?;
        let shared = Arc::new(Shared {
            coordinator,
            authority: Authority::new(keys.clone(), now()?),
            keys: Mutex::new(keys),
            host: address.to_string(),
            stop: AtomicBool::new(false),
            counters: Arc::new(Counters::default()),
            event_write_pause: Mutex::new(None),
            write_deadline,
        });
        let (sender, receiver) = mpsc::sync_channel::<TcpStream>(8);
        let receiver = Arc::new(Mutex::new(receiver));
        let mut workers = Vec::new();
        for _ in 0..WORKERS {
            let shared = Arc::clone(&shared);
            let receiver = Arc::clone(&receiver);
            workers.push(thread::spawn(move || {
                while !shared.stop.load(Ordering::SeqCst) {
                    let result = match receiver.lock() {
                        Ok(receiver) => receiver.recv_timeout(POLL),
                        Err(_) => break,
                    };
                    match result {
                        Ok(mut stream) => {
                            shared.counters.connections.fetch_add(1, Ordering::SeqCst);
                            handle(&mut stream, &shared);
                            shared.counters.connections.fetch_sub(1, Ordering::SeqCst);
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
            }));
        }
        let accept_shared = Arc::clone(&shared);
        let accept = thread::spawn(move || {
            while !accept_shared.stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        if let Err(mpsc::TrySendError::Full(mut stream)) = sender.try_send(stream) {
                            accept_shared
                                .counters
                                .rejected
                                .fetch_add(1, Ordering::SeqCst);
                            reject(&mut stream, Code::ServiceUnavailable, None);
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2))
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            shared,
            address,
            accept: Some(accept),
            workers,
        })
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }
    pub fn counters(&self) -> Arc<Counters> {
        Arc::clone(&self.shared.counters)
    }

    /// Pause one stream worker before its next event write, for deterministic
    /// overflow tests. The writer/coordinator is never paused. The worker waits
    /// at most three seconds, and a dropped release sender also ends the wait.
    pub fn pause_next_event_write(
        &self,
    ) -> Result<(mpsc::Receiver<()>, mpsc::SyncSender<()>), Error> {
        let mut slot = self
            .shared
            .event_write_pause
            .lock()
            .map_err(|_| Error::Io)?;
        if slot.is_some() {
            return Err(Error::InvalidConfig);
        }
        let (arrived, waiting) = mpsc::sync_channel(1);
        let (release, resume) = mpsc::sync_channel(1);
        *slot = Some((arrived, resume));
        Ok((waiting, release))
    }

    /// Local harness driver, not a network mutation endpoint.
    pub fn insert(&self, json: &[u8]) -> Result<u64, Error> {
        let document = DocumentInput::parse(json, DEFAULT_MAX_DOCUMENT_BYTES)
            .and_then(DocumentInput::prepare_insert)
            .map_err(|_| Error::InvalidDocument)?;
        self.shared
            .coordinator
            .apply("orders", Mutation::Insert(&document))
            .map_err(|_| Error::Feed)
    }

    pub fn revoke(&self) -> Result<(), Error> {
        let mut keys = self.shared.keys.lock().map_err(|_| Error::Auth)?;
        let revoked = keys.revoke_all().map_err(|_| Error::Auth)?;
        self.shared
            .authority
            .reload(Ok(revoked.clone()), now()?)
            .map_err(|_| Error::Auth)?;
        *keys = revoked;
        Ok(())
    }

    pub fn shutdown(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        let _ = self.shared.coordinator.shutdown();
        if let Some(accept) = self.accept.take() {
            let _ = accept.join();
        }
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn write_frame(stream: &mut TcpStream, mut bytes: &[u8], duration: Duration) -> io::Result<()> {
    let deadline = Instant::now() + duration;
    while !bytes.is_empty() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::ErrorKind::TimedOut.into());
        }
        stream.set_write_timeout(Some(remaining))?;
        match stream.write(bytes) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(count) => bytes = &bytes[count..],
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn reject(
    stream: &mut TcpStream,
    code: Code,
    allow: Option<kiarni_http_contract_spike::AllowedMethods>,
) {
    reject_with_head(stream, code, allow, false);
}

fn reject_with_head(
    stream: &mut TcpStream,
    code: Code,
    allow: Option<kiarni_http_contract_spike::AllowedMethods>,
    head: bool,
) {
    let Ok(id) = RequestId::generate() else {
        return;
    };
    let Ok(response) = error_response(code, &id, allow, head) else {
        return;
    };
    let mut bytes = format!(
        "HTTP/1.1 {} Error\r\nConnection: close\r\n",
        response.status
    )
    .into_bytes();
    for (name, value) in response.headers {
        bytes.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
    }
    bytes.extend_from_slice(b"\r\n");
    bytes.extend_from_slice(&response.body);
    let _ = write_frame(stream, &bytes, Duration::from_millis(100));
}

fn read_headers(stream: &mut TcpStream) -> Result<Zeroizing<Vec<u8>>, Code> {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut bytes = Zeroizing::new(Vec::with_capacity(HEADER_LIMIT));
    let mut chunk = Zeroizing::new([0u8; 512]);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(Code::RequestTimeout);
        }
        stream
            .set_read_timeout(Some(remaining))
            .map_err(|_| Code::InternalError)?;
        let count = stream.read(&mut *chunk).map_err(|_| Code::RequestTimeout)?;
        if count == 0 {
            return Err(Code::InvalidPath);
        }
        if bytes.len() + count > HEADER_LIMIT {
            return Err(Code::HeadersTooLarge);
        }
        bytes.extend_from_slice(&chunk[..count]);
        if bytes.windows(4).any(|value| value == b"\r\n\r\n") {
            return Ok(bytes);
        }
    }
}

fn header_values<'a>(headers: &'a [httparse::Header<'a>], name: &str) -> Vec<&'a [u8]> {
    headers
        .iter()
        .filter(|header| header.name.eq_ignore_ascii_case(name))
        .map(|header| header.value)
        .collect()
}

fn handle(stream: &mut TcpStream, shared: &Arc<Shared>) {
    // Accepted sockets can inherit the listener's nonblocking mode on macOS.
    // This fixture uses blocking I/O with absolute read/write deadlines.
    if stream.set_nonblocking(false).is_err()
        || stream.set_nodelay(true).is_err()
        || rustix::net::sockopt::set_socket_send_buffer_size(&*stream, 16384).is_err()
    {
        return;
    }
    let bytes = match read_headers(stream) {
        Ok(bytes) => bytes,
        Err(code) => {
            reject(stream, code, None);
            return;
        }
    };
    let mut headers = [httparse::EMPTY_HEADER; 32];
    let mut request = httparse::Request::new(&mut headers);
    if !matches!(request.parse(&bytes), Ok(httparse::Status::Complete(count)) if count == bytes.len())
    {
        reject(stream, Code::InvalidPath, None);
        return;
    }
    if request.version != Some(1)
        || !header_values(request.headers, "transfer-encoding").is_empty()
        || header_values(request.headers, "content-length")
            .iter()
            .any(|value| *value != b"0")
    {
        reject(stream, Code::UnexpectedBody, None);
        return;
    }
    let method = request.method.unwrap_or("");
    let path = request.path.unwrap_or("");
    let reject = |stream: &mut TcpStream, code, allow| {
        reject_with_head(stream, code, allow, method == "HEAD")
    };
    let asset = matches!(path, "/" | "/client.mjs") && method == "GET";
    let authorization = header_values(request.headers, "authorization");
    let authenticated = if asset {
        None
    } else {
        let Ok(now) = now() else {
            reject(stream, Code::ServiceUnavailable, None);
            return;
        };
        match authenticate_and_resolve(&shared.authority, &authorization, now, method, path) {
            Ok(value) => Some(value),
            Err(error) => {
                if error.code == Code::Unauthorized {
                    shared.counters.unauthorized.fetch_add(1, Ordering::SeqCst);
                }
                reject(stream, error.code, error.allow);
                return;
            }
        }
    };
    let hosts = header_values(request.headers, "host");
    let origins = header_values(request.headers, "origin");
    let origin = format!("http://{}", shared.host);
    if hosts.as_slice() != [shared.host.as_bytes()]
        || origins.len() > 1
        || origins
            .first()
            .is_some_and(|value| *value != origin.as_bytes())
    {
        reject(stream, Code::Forbidden, None);
        return;
    }
    if asset {
        let (content_type, body) = if path == "/client.mjs" {
            (
                "text/javascript",
                include_str!("../../streaming/client.mjs"),
            )
        } else {
            (
                "text/html",
                "<!doctype html><meta charset=utf-8><title>Kiarni transport fixture</title>",
            )
        };
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}; charset=utf-8\r\nCache-Control: no-store\r\nConnection: close\r\nContent-Length: {}\r\nContent-Security-Policy: default-src 'none'; script-src 'self'; connect-src 'self'; base-uri 'none'; frame-ancestors 'none'\r\n\r\n{body}",
            body.len()
        );
        let _ = write_frame(stream, response.as_bytes(), shared.write_deadline);
        return;
    }
    let Some((session, target)) = authenticated else {
        return;
    };
    if method != "GET" {
        reject(
            stream,
            Code::MethodNotAllowed,
            Some(kiarni_http_contract_spike::AllowedMethods::Changes),
        );
        return;
    }
    match target.operation() {
        Operation::Changes => {
            if !header_values(request.headers, "last-event-id").is_empty() {
                reject(stream, Code::ResumeNotSupported, None);
                return;
            }
            if header_values(request.headers, "accept").as_slice()
                != [b"text/event-stream".as_slice()]
            {
                reject(stream, Code::NotAcceptable, None);
                return;
            }
            serve_events(stream, shared, session, target.collection().unwrap_or(""));
        }
        Operation::ReadDocument => {
            let result = shared
                .coordinator
                .get(target.collection().unwrap_or(""), target.id().unwrap_or(""));
            match result {
                Ok(Some(body)) => {
                    let Ok(id) = RequestId::generate() else {
                        return;
                    };
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nCache-Control: no-store\r\nX-Request-ID: {}\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
                        id.as_str(),
                        body.len()
                    );
                    if write_frame(stream, header.as_bytes(), shared.write_deadline).is_ok()
                        && !target.is_head()
                    {
                        let _ = write_frame(stream, &body, shared.write_deadline);
                    }
                }
                Ok(None) => reject(stream, Code::DocumentNotFound, None),
                Err(error) => reject(stream, Code::from(error), None),
            }
        }
        _ => reject(stream, Code::RouteNotFound, None),
    }
}

struct StreamSlot(Arc<Counters>);
impl Drop for StreamSlot {
    fn drop(&mut self) {
        self.0.streams.fetch_sub(1, Ordering::SeqCst);
    }
}

fn serve_events(stream: &mut TcpStream, shared: &Shared, session: Session, collection: &str) {
    if shared
        .counters
        .streams
        .try_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
            (value < STREAMS).then_some(value + 1)
        })
        .is_err()
    {
        reject(stream, Code::SubscriptionLimit, None);
        return;
    }
    let _slot = StreamSlot(Arc::clone(&shared.counters));
    let subscription = match shared.coordinator.subscribe(collection) {
        Ok(subscription) => subscription,
        Err(error) => {
            reject(stream, Code::from(error), None);
            return;
        }
    };
    shared.counters.subscriptions.fetch_add(1, Ordering::SeqCst);
    let Ok(id) = RequestId::generate() else {
        return;
    };
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream; charset=utf-8\r\nCache-Control: no-store\r\nX-Request-ID: {}\r\nConnection: close\r\n\r\n{}",
        id.as_str(),
        sse::READY
    );
    if write_frame(stream, response.as_bytes(), shared.write_deadline).is_err() {
        return;
    }
    pump(stream, shared, session, subscription);
}

fn pump(stream: &mut TcpStream, shared: &Shared, session: Session, subscription: Subscription) {
    let mut heartbeat = Instant::now();
    loop {
        let terminal = if shared.stop.load(Ordering::SeqCst) {
            Some(Terminal::Shutdown)
        } else if now()
            .ok()
            .is_none_or(|now| shared.authority.revalidate(&session, now).is_err())
        {
            Some(Terminal::Interrupted)
        } else {
            None
        };
        if let Some(reason) = terminal {
            let _ = write_frame(
                stream,
                sse::encode_terminal(reason).as_bytes(),
                shared.write_deadline,
            );
            return;
        }
        let frame = match subscription.recv_timeout(POLL) {
            Ok(event) => match sse::encode_change(&event) {
                Ok(frame) => Some(frame),
                Err(_) => {
                    let _ = write_frame(
                        stream,
                        sse::encode_terminal(Terminal::Interrupted).as_bytes(),
                        shared.write_deadline,
                    );
                    return;
                }
            },
            Err(ReceiveError::Timeout) => None,
            Err(error) => {
                let reason = match error {
                    ReceiveError::Overflow => {
                        shared.counters.overflows.fetch_add(1, Ordering::SeqCst);
                        Terminal::Overflow
                    }
                    ReceiveError::Shutdown => Terminal::Shutdown,
                    _ => Terminal::Interrupted,
                };
                let _ = write_frame(
                    stream,
                    sse::encode_terminal(reason).as_bytes(),
                    shared.write_deadline,
                );
                return;
            }
        };
        if let Some(frame) = frame {
            // Authorization can change while waiting for a queued event.
            if now()
                .ok()
                .is_none_or(|now| shared.authority.revalidate(&session, now).is_err())
            {
                let _ = write_frame(
                    stream,
                    sse::encode_terminal(Terminal::Interrupted).as_bytes(),
                    shared.write_deadline,
                );
                return;
            }
            let pause = shared
                .event_write_pause
                .lock()
                .ok()
                .and_then(|mut slot| slot.take());
            if let Some((arrived, resume)) = pause {
                let _ = arrived.try_send(());
                if resume.recv_timeout(Duration::from_secs(3)).is_err() {
                    return;
                }
            }
            shared.counters.event_writes.fetch_add(1, Ordering::SeqCst);
            if let Err(error) = write_frame(stream, frame.as_bytes(), shared.write_deadline) {
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) {
                    shared
                        .counters
                        .write_timeouts
                        .fetch_add(1, Ordering::SeqCst);
                }
                return;
            }
        } else if heartbeat.elapsed() >= Duration::from_millis(100) {
            if write_frame(stream, sse::HEARTBEAT.as_bytes(), shared.write_deadline).is_err() {
                return;
            }
            heartbeat = Instant::now();
        }
    }
}
