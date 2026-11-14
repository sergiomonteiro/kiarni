use std::{
    io::{self, Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use kiarni_auth::Authority;
use kiarni_config::{Admission, Config};
use kiarni_http::{Code, Operation, Rejection, RequestId, Response, error_response, resolve};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use zeroize::Zeroizing;

const HEADER_LIMIT: usize = 8192;
const POLL: Duration = Duration::from_millis(25);

struct Health {
    metrics: super::metrics::Metrics,
    query_metrics: super::query_metrics::Metrics,
    logs: super::logging::Client,
    recent_errors: super::recent_errors::History,
    started: Instant,
    admission: Admission,
    store: Arc<kiarni_storage::Store>,
    writer: super::writer::Client,
    query_gate: super::gate::Gate,
    max_result: usize,
    max_document: usize,
    max_query: usize,
    authority: Authority,
    host: String,
    origin: String,
    read_timeout: Duration,
    write_timeout: Duration,
    heartbeat: Duration,
    stop: Arc<AtomicBool>,
}

struct RequestState {
    query_trace: Option<super::query_metrics::Trace>,
    method: &'static str,
    operation: Option<Operation>,
    head: bool,
    body_consumed: bool,
    // Held through header/body writes, including errors and slow clients.
    reply: Option<super::writer::Reply>,
    permit: Arc<kiarni_config::Permit>,
    session: Option<kiarni_auth::Session>,
    streaming: bool,
}

struct Worker {
    completed: Arc<AtomicBool>,
    handle: JoinHandle<()>,
}

pub fn serve(
    listener: TcpListener,
    config: &Config,
    authority: Authority,
    store: kiarni_storage::Store,
    stop: Arc<AtomicBool>,
) -> io::Result<()> {
    let started = Instant::now();
    listener.set_nonblocking(true)?;
    let address = listener.local_addr()?;
    let store = Arc::new(store);
    let writer = super::writer::Coordinator::start(
        Arc::clone(&store),
        super::writer::Policy {
            max_requests: config.max_requests() as usize,
            max_document_bytes: config.max_document_bytes() as usize,
            max_subscribers: config.max_streams() as usize,
            feed_capacity: config.feed_queue() as usize,
        },
        Arc::clone(&stop),
    )
    .map_err(|_| io::Error::other("writer_start_failed"))?;
    let admission = Admission::new(config);
    let logger = super::logging::Logger::start(config.log_level())?;
    let shared = Arc::new(Health {
        metrics: super::metrics::Metrics::default(),
        query_metrics: super::query_metrics::Metrics::default(),
        logs: logger.client(),
        recent_errors: super::recent_errors::History::default(),
        started,
        admission: admission.clone(),
        store,
        writer: writer.client(),
        query_gate: super::gate::Gate::default(),
        max_result: config.max_result_bytes() as usize,
        max_document: config.max_document_bytes() as usize,
        max_query: config.max_query_bytes() as usize,
        authority,
        host: address.to_string(),
        origin: format!("http://{address}"),
        read_timeout: Duration::from_millis(config.read_timeout_ms()),
        write_timeout: Duration::from_millis(config.write_timeout_ms()),
        heartbeat: Duration::from_millis(config.heartbeat_ms()),
        stop,
    });
    let mut workers: Vec<Worker> = Vec::new();
    let acceptor = thread::current();
    if !shared.stop.load(Ordering::Relaxed) {
        println!("Escutando em http://{address}");
    }
    let result = loop {
        let mut index = 0;
        while index < workers.len() {
            if workers[index].completed.load(Ordering::Acquire)
                || workers[index].handle.is_finished()
            {
                let _ = workers.swap_remove(index).handle.join();
            } else {
                index += 1;
            }
        }
        if shared.stop.load(Ordering::Relaxed) {
            break Ok(());
        }
        // Leave pending connections in the kernel backlog until a worker slot
        // is released. Completion wakes this wait without a polling delay.
        if workers.len() as u64 >= config.max_requests() {
            thread::park_timeout(POLL);
            continue;
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                let Ok(permit) = admission.acquire(false) else {
                    continue;
                };
                let health = Arc::clone(&shared);
                let completed = Arc::new(AtomicBool::new(false));
                let completion = Arc::clone(&completed);
                let acceptor = acceptor.clone();
                if let Ok(worker) = thread::Builder::new().spawn(move || {
                    let permit = Arc::new(permit);
                    if stream.set_nonblocking(false).is_ok() {
                        health.handle(&mut stream, permit);
                    }
                    completion.store(true, Ordering::Release);
                    acceptor.unpark();
                }) {
                    workers.push(Worker {
                        completed,
                        handle: worker,
                    });
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                let mut events = [PollFd::new(&listener, PollFlags::IN)];
                let timeout = Timespec {
                    tv_sec: 0,
                    tv_nsec: 25_000_000,
                };
                match poll(&mut events, Some(&timeout)) {
                    Ok(_) => {}
                    Err(rustix::io::Errno::INTR) => {}
                    Err(_) => break Err(io::Error::other("accept_poll_failed")),
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => break Err(io::Error::other("accept_failed")),
        }
    };
    shared.stop.store(true, Ordering::Relaxed);
    writer.close();
    drop(listener);
    for worker in workers {
        let _ = worker.handle.join();
    }
    let writer_result = writer
        .shutdown()
        .map_err(|_| io::Error::other("writer_join_failed"));
    drop(shared);
    drop(logger);
    result.and(writer_result)
}

fn header_values<'a>(headers: &[httparse::Header<'a>], name: &str) -> Vec<&'a [u8]> {
    headers
        .iter()
        .filter(|header| header.name.eq_ignore_ascii_case(name))
        .map(|header| header.value)
        .collect()
}

impl Health {
    fn record_completion(&self, request: super::logging::Request<'_>) {
        self.recent_errors.observe(&request);
        self.logs.record(request);
    }

    fn handle(&self, stream: &mut TcpStream, permit: Arc<kiarni_config::Permit>) {
        let started = Instant::now();
        let Ok(id) = RequestId::generate() else {
            return;
        };
        let mut state = RequestState {
            query_trace: None,
            method: "UNKNOWN",
            operation: None,
            head: false,
            body_consumed: false,
            reply: None,
            permit,
            session: None,
            streaming: false,
        };
        let deadline = Instant::now() + self.read_timeout;
        let response = self
            .read_headers(stream, deadline)
            .map_err(Rejection::from)
            .and_then(|bytes| self.response(stream, &bytes, &id, deadline, &mut state));
        if let Some(trace) = state.query_trace.take() {
            self.query_metrics
                .finish(trace, response.as_ref().err().map(|error| error.code));
        }
        let mut error_code = None;
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                error_code = Some(error.code);
                let Ok(response) = error_response(error.code, &id, error.allow, state.head) else {
                    return;
                };
                response
            }
        };
        if state.streaming {
            self.stream(stream, &id, &state);
            let terminal = state
                .reply
                .as_ref()
                .and_then(|reply| match reply.outcome() {
                    Ok(super::writer::Outcome::Subscribed(sub)) => sub.terminal_reason(),
                    _ => None,
                })
                .map(|value| match value {
                    kiarni_feeds::Terminal::Overflow => "overflow",
                    kiarni_feeds::Terminal::Shutdown => "shutdown",
                    kiarni_feeds::Terminal::Interrupted => "interrupted",
                });
            let elapsed = started.elapsed();
            self.metrics
                .observe(200, elapsed, super::metrics::Outcome::StreamClosed);
            self.record_completion(super::logging::Request {
                id: &id,
                method: state.method,
                operation: state.operation,
                status: 200,
                error: None,
                elapsed,
                transport: "stream_closed",
                terminal,
            });
            return;
        }
        let reason = if response.status < 400 { "OK" } else { "Error" };
        let mut wire = format!(
            "HTTP/1.1 {} {reason}\r\nConnection: close\r\n",
            response.status
        )
        .into_bytes();
        for (name, value) in response.headers {
            wire.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
        }
        wire.extend_from_slice(b"\r\n");
        let deadline = Instant::now() + self.write_timeout;
        // Keep one ordinary send for small JSON responses; large bodies are
        // vectored without an additional payload-sized allocation.
        let written = if response.body.len() <= 4096 {
            wire.extend_from_slice(&response.body);
            self.write_response(stream, &wire, &[], deadline)
        } else {
            self.write_response(stream, &wire, &response.body, deadline)
        };
        let elapsed = started.elapsed();
        self.metrics.observe(
            response.status,
            elapsed,
            if written.is_ok() {
                super::metrics::Outcome::Written
            } else {
                super::metrics::Outcome::WriteFailed
            },
        );
        self.record_completion(super::logging::Request {
            id: &id,
            method: state.method,
            operation: state.operation,
            status: response.status,
            error: error_code,
            elapsed,
            transport: if written.is_ok() {
                "written"
            } else {
                "write_failed"
            },
            terminal: None,
        });
        // Send EOF before draining a bounded amount of unread request bytes.
        // Early errors (such as 415) can otherwise reset TCP with a body in flight.
        let _ = stream.shutdown(Shutdown::Write);
        if state.body_consumed {
            return;
        }
        let deadline = Instant::now() + POLL;
        let mut remaining = self.max_document + HEADER_LIMIT;
        let mut discard = Zeroizing::new([0u8; 512]);
        while remaining > 0 && !self.stop.load(Ordering::Relaxed) {
            let timeout = deadline.saturating_duration_since(Instant::now());
            if timeout.is_zero() || stream.set_read_timeout(Some(timeout)).is_err() {
                break;
            }
            let wanted = remaining.min(discard.len());
            match stream.read(&mut discard[..wanted]) {
                Ok(0) | Err(_) => break,
                Ok(count) => remaining -= count,
            }
        }
    }

    fn read_headers(
        &self,
        stream: &mut TcpStream,
        deadline: Instant,
    ) -> Result<Zeroizing<Vec<u8>>, Code> {
        let mut bytes = Zeroizing::new(Vec::with_capacity(HEADER_LIMIT));
        let mut chunk = Zeroizing::new([0u8; 512]);
        loop {
            if self.stop.load(Ordering::Relaxed) {
                return Err(Code::ShuttingDown);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(Code::RequestTimeout);
            }
            stream
                .set_read_timeout(Some(remaining.min(POLL)))
                .map_err(|_| Code::InternalError)?;
            match stream.read(&mut *chunk) {
                Ok(0) => return Err(Code::InvalidPath),
                Ok(count) => {
                    bytes.extend_from_slice(&chunk[..count]);
                    if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                        if end + 4 > HEADER_LIMIT {
                            return Err(Code::HeadersTooLarge);
                        }
                        return Ok(bytes);
                    }
                    if bytes.len() >= HEADER_LIMIT {
                        return Err(Code::HeadersTooLarge);
                    }
                }
                Err(error) if retryable(&error) => {}
                Err(_) => return Err(Code::InvalidPath),
            }
        }
    }

    fn response(
        &self,
        stream: &mut TcpStream,
        bytes: &[u8],
        id: &RequestId,
        deadline: Instant,
        state: &mut RequestState,
    ) -> Result<Response, Rejection> {
        let mut headers = [httparse::EMPTY_HEADER; 32];
        let mut request = httparse::Request::new(&mut headers);
        let offset = match request.parse(bytes) {
            Ok(httparse::Status::Complete(count)) => count,
            Err(httparse::Error::TooManyHeaders) => return Err(Code::HeadersTooLarge.into()),
            _ => return Err(Code::InvalidPath.into()),
        };
        let method = request.method.ok_or(Code::InvalidPath)?;
        state.method = super::logging::method(method);
        let path = request.path.ok_or(Code::InvalidPath)?;
        state.head = method == "HEAD";
        if request.version != Some(1) || path.len() > 2048 {
            return Err(Code::InvalidPath.into());
        }
        let length = header_values(request.headers, "content-length");
        if length.len() > 1 || !header_values(request.headers, "transfer-encoding").is_empty() {
            return Err(Code::UnexpectedBody.into());
        }
        let body_length = match length.first() {
            None => 0,
            Some(value) => {
                if value.is_empty() || !value.iter().all(u8::is_ascii_digit) {
                    return Err(Code::UnexpectedBody.into());
                }
                std::str::from_utf8(value)
                    .ok()
                    .and_then(|value| value.parse::<usize>().ok())
                    .ok_or(Code::RequestTooLarge)?
            }
        };
        // Only fixed, data-free assets bypass API authentication. Host/Origin
        // checks still prevent an alternate origin from using this shell.
        if super::dashboard::asset(path.split('?').next().unwrap_or(path)).is_some() {
            let origins = header_values(request.headers, "origin");
            if header_values(request.headers, "host").as_slice() != [self.host.as_bytes()]
                || (!origins.is_empty() && origins.as_slice() != [self.origin.as_bytes()])
            {
                return Err(Code::Forbidden.into());
            }
            if path.contains('?') {
                return Err(Code::InvalidPath.into());
            }
            if body_length != 0 || bytes.len() != offset {
                return Err(Code::UnexpectedBody.into());
            }
            if !header_values(request.headers, "content-encoding").is_empty() {
                return Err(Code::UnsupportedMediaType.into());
            }
            state.body_consumed = true;
            return super::dashboard::response(method, path, id);
        }
        let now = super::now().map_err(|_| Code::ServiceUnavailable)?;
        let session = self
            .authority
            .authenticate(&header_values(request.headers, "authorization"), now)
            .map_err(Code::from)?;
        state.session = Some(session);
        let origins = header_values(request.headers, "origin");
        if header_values(request.headers, "host").as_slice() != [self.host.as_bytes()]
            || (!origins.is_empty() && origins.as_slice() != [self.origin.as_bytes()])
        {
            return Err(Code::Forbidden.into());
        }
        let (raw_path, query) = path
            .split_once('?')
            .map_or((path, None), |(path, query)| (path, Some(query)));
        let target = resolve(method, raw_path)?;
        let operation = target.operation();
        state.operation = Some(operation);
        if operation == Operation::Query {
            state.query_trace = Some(super::query_metrics::Trace::start());
        }
        if !matches!(
            operation,
            Operation::Liveness
                | Operation::Readiness
                | Operation::ListCollections
                | Operation::CreateCollection
                | Operation::DeleteCollection
                | Operation::InsertDocument
                | Operation::ReadDocument
                | Operation::ReplaceDocument
                | Operation::PatchDocument
                | Operation::DeleteDocument
                | Operation::Query
                | Operation::Changes
                | Operation::Metrics
                | Operation::RecentErrors
                | Operation::Status
                | Operation::Count
        ) {
            return Err(Code::RouteNotFound.into());
        }
        if query.is_some() && operation != Operation::ListCollections {
            return Err(Code::InvalidPath.into());
        }
        let cap = match operation {
            Operation::CreateCollection => 1024,
            Operation::Query => self.max_query,
            Operation::InsertDocument | Operation::ReplaceDocument | Operation::PatchDocument => {
                self.max_document
            }
            _ => 0,
        };
        if body_length > cap {
            return Err(if cap == 0 {
                Code::UnexpectedBody
            } else if operation == Operation::Query {
                Code::QueryTooLarge
            } else if operation != Operation::CreateCollection {
                Code::DocumentTooLarge
            } else {
                Code::RequestTooLarge
            }
            .into());
        }
        if !header_values(request.headers, "content-encoding").is_empty() {
            return Err(Code::UnsupportedMediaType.into());
        }
        let media = if matches!(
            operation,
            Operation::CreateCollection
                | Operation::InsertDocument
                | Operation::ReplaceDocument
                | Operation::PatchDocument
                | Operation::Query
        ) {
            super::collections::json_media_type(
                &header_values(request.headers, "content-type"),
                operation == Operation::PatchDocument,
            )?
        } else {
            None
        };
        let initial = &bytes[offset..];
        if initial.len() > body_length {
            return Err(Code::UnexpectedBody.into());
        }
        let mut body = self.read_body(stream, initial, body_length, deadline)?;
        state.body_consumed = true;
        #[cfg(feature = "profiling")]
        let validation = (operation == Operation::ReplaceDocument)
            .then(|| super::profiling::PUT_VALIDATION.start());
        target.validate_body(media, &body)?;
        #[cfg(feature = "profiling")]
        drop(validation);
        let mut location = None;
        let (status, body) = match operation {
            Operation::RecentErrors => (200, self.recent_errors.json()?),
            Operation::Status => {
                let connections = self.admission.snapshot();
                (
                    200,
                    serde_json::to_vec(&serde_json::json!({
                        "version": env!("CARGO_PKG_VERSION"),
                        "uptime_seconds": self.started.elapsed().as_secs().to_string(),
                        "connections": {
                            "admitted": connections.admitted,
                            "streaming": connections.streaming,
                            "limit": connections.limit,
                            "stream_limit": connections.stream_limit,
                        }
                    }))
                    .map_err(|_| Code::InternalError)?,
                )
            }
            Operation::Metrics => {
                let mut text = self.metrics.prometheus();
                text.push_str(&self.query_metrics.prometheus());
                text.push_str(&self.store.prometheus_metrics());
                text.push_str(&self.writer.feed_metrics().prometheus());
                (200, text.into_bytes())
            }
            Operation::Changes => {
                if !header_values(request.headers, "last-event-id").is_empty() {
                    return Err(Code::ResumeNotSupported.into());
                }
                let accepts = header_values(request.headers, "accept");
                if accepts.len() != 1
                    || !std::str::from_utf8(accepts[0])
                        .is_ok_and(|value| value.trim().eq_ignore_ascii_case("text/event-stream"))
                {
                    return Err(Code::NotAcceptable.into());
                }
                Arc::get_mut(&mut state.permit)
                    .ok_or(Code::InternalError)?
                    .promote_to_stream()
                    .map_err(|_| Code::SubscriptionLimit)?;
                self.mutate(
                    super::writer::Command::Subscribe {
                        collection: target.collection().ok_or(Code::InternalError)?.to_owned(),
                    },
                    state,
                )?;
                self.authority
                    .revalidate(
                        state.session.as_ref().ok_or(Code::InternalError)?,
                        super::now().map_err(|_| Code::ServiceUnavailable)?,
                    )
                    .map_err(Code::from)?;
                state.streaming = true;
                (200, Vec::new())
            }
            Operation::Liveness => (200, br#"{"status":"live"}"#.to_vec()),
            Operation::Readiness => {
                check_readiness(&self.store)?;
                (200, br#"{"status":"ready"}"#.to_vec())
            }
            Operation::CreateCollection => {
                #[derive(serde::Deserialize)]
                struct Input {
                    name: String,
                }
                let input: Input =
                    serde_json::from_slice(&body).map_err(|_| Code::InvalidCollection)?;
                self.mutate(
                    super::writer::Command::CreateCollection {
                        name: input.name.as_str().to_owned(),
                    },
                    state,
                )?;
                location = Some(format!("/v1/collections/{}", input.name));
                (
                    201,
                    serde_json::to_vec(&serde_json::json!({"name":input.name}))
                        .map_err(|_| Code::InternalError)?,
                )
            }
            Operation::ListCollections => {
                let (limit, after) = super::collections::pagination(query)?;
                let page = self
                    .store
                    .list_collections(limit, after)
                    .map_err(super::collections::storage_error)?;
                (
                    200,
                    serde_json::to_vec(&page).map_err(|_| Code::InternalError)?,
                )
            }
            Operation::Count => {
                let _query = self
                    .query_gate
                    .enter(&self.stop, Code::ServiceUnavailable)?;
                let count = self
                    .store
                    .count_documents(target.collection().ok_or(Code::InternalError)?)
                    .map_err(super::collections::storage_error)?;
                (
                    200,
                    serde_json::to_vec(&serde_json::json!({
                        "count": count.count.to_string(), "exact": count.exact,
                        "limits": {"documents": kiarni_storage::COUNT_MAX_DOCUMENTS,
                                   "document_bytes": kiarni_storage::COUNT_MAX_BYTES}
                    }))
                    .map_err(|_| Code::InternalError)?,
                )
            }
            Operation::Query => {
                let trace = state.query_trace.as_mut().ok_or(Code::InternalError)?;
                let _query = trace.admit(&self.query_gate, &self.stop)?;
                let started = Instant::now();
                let result = self.query_body(
                    target.collection().ok_or(Code::InternalError)?,
                    &body,
                    trace,
                );
                trace.execution = Some(started.elapsed());
                (200, result?)
            }
            Operation::InsertDocument => {
                let name = target.collection().ok_or(Code::InternalError)?;
                let outcome = self.mutate(
                    super::writer::Command::Insert {
                        collection: name.to_owned(),
                        body: std::mem::take(&mut *body),
                    },
                    state,
                )?;
                let super::writer::Outcome::Inserted(candidate) = outcome else {
                    return Err(Code::InternalError.into());
                };
                location = Some(format!(
                    "/v1/collections/{name}/documents/{}",
                    candidate.id()
                ));
                (201, candidate.json().to_vec())
            }
            Operation::ReplaceDocument | Operation::PatchDocument => {
                let operation = if operation == Operation::ReplaceDocument {
                    kiarni_engine::writes::Operation::Replace
                } else {
                    kiarni_engine::writes::Operation::MergePatch
                };
                let outcome = self.mutate(
                    super::writer::Command::Update {
                        collection: target.collection().ok_or(Code::InternalError)?.to_owned(),
                        id: target.id().ok_or(Code::InternalError)?.to_owned(),
                        operation,
                        body: std::mem::take(&mut *body),
                    },
                    state,
                )?;
                let super::writer::Outcome::Updated(candidate) = outcome else {
                    return Err(Code::InternalError.into());
                };
                (200, candidate.json().to_vec())
            }
            Operation::DeleteDocument => {
                self.mutate(
                    super::writer::Command::Delete {
                        collection: target.collection().ok_or(Code::InternalError)?.to_owned(),
                        id: target.id().ok_or(Code::InternalError)?.to_owned(),
                    },
                    state,
                )?;
                (204, Vec::new())
            }
            Operation::ReadDocument => {
                let store = &self.store;
                let collection = store
                    .collection(target.collection().ok_or(Code::InternalError)?)
                    .map_err(super::collections::storage_error)?;
                let json = store
                    .get_document(&collection, target.id().ok_or(Code::InternalError)?)
                    .map_err(|error| match error {
                        kiarni_storage::Error::CollectionMissing => Code::CollectionNotFound,
                        _ => Code::InternalError,
                    })?
                    .ok_or(Code::DocumentNotFound)?;
                (200, json)
            }
            Operation::DeleteCollection => {
                self.mutate(
                    super::writer::Command::DropCollection {
                        name: target.collection().ok_or(Code::InternalError)?.to_owned(),
                    },
                    state,
                )?;
                (204, Vec::new())
            }
        };
        if body.len() > self.max_result {
            return Err(Code::ResultTooLarge.into());
        }
        let mut headers = [
            ("Content-Length", body.len().to_string()),
            ("Cache-Control", "no-store".into()),
            ("X-Request-ID", id.as_str().into()),
        ]
        .into_iter()
        .collect::<std::collections::BTreeMap<_, _>>();
        if status != 204 {
            headers.insert(
                "Content-Type",
                if operation == Operation::Metrics {
                    "text/plain; version=0.0.4; charset=utf-8"
                } else {
                    "application/json"
                }
                .into(),
            );
        } else {
            headers.remove("Content-Length");
        }
        if let Some(location) = location {
            headers.insert("Location", location);
        }
        Ok(Response {
            status,
            headers,
            body: if state.head { Vec::new() } else { body },
        })
    }

    fn query_body(
        &self,
        name: &str,
        body: &[u8],
        trace: &mut super::query_metrics::Trace,
    ) -> Result<Vec<u8>, Code> {
        let query = kiarni_engine::query::Query::parse(body).map_err(Code::from)?;
        trace.access = Some(if query.id().is_some() {
            super::query_metrics::Access::Id
        } else {
            super::query_metrics::Access::Scan
        });
        let collection = self
            .store
            .collection(name)
            .map_err(super::collections::storage_error)?;
        let documents = self
            .store
            .query_documents(&collection, query.id())
            .map_err(|error| {
                if error == kiarni_storage::Error::QueryLimit {
                    trace.scan_limit = true;
                }
                match error {
                    kiarni_storage::Error::InvalidLayout => Code::InternalError,
                    _ => super::collections::storage_error(error),
                }
            })?;
        if self.stop.load(Ordering::Relaxed) {
            return Err(Code::ShuttingDown);
        }
        let output = query
            .evaluate_documents(&documents)
            .map_err(|error| match error {
                kiarni_engine::query::Error::DatasetLimit => {
                    trace.scan_limit = true;
                    Code::QueryTooComplex
                }
                _ => Code::from(error),
            })?;
        if self.stop.load(Ordering::Relaxed) {
            return Err(Code::ShuttingDown);
        }
        #[derive(serde::Serialize)]
        struct QueryResult<'a> {
            documents: Vec<&'a serde_json::value::RawValue>,
            count: usize,
        }
        let result = QueryResult {
            documents: output
                .iter()
                .map(|json| {
                    serde_json::from_slice::<&serde_json::value::RawValue>(json)
                        .map_err(|_| Code::InternalError)
                })
                .collect::<Result<_, _>>()?,
            count: output.len(),
        };
        let bytes = serde_json::to_vec(&result).map_err(|_| Code::InternalError)?;
        if bytes.len() > self.max_result {
            return Err(Code::ResultTooLarge);
        }
        trace.documents = output.len() as u64;
        trace.bytes = bytes.len() as u64;
        Ok(bytes)
    }

    fn mutate<'a>(
        &self,
        command: super::writer::Command,
        state: &'a mut RequestState,
    ) -> Result<&'a super::writer::Outcome, Code> {
        #[cfg(feature = "profiling")]
        let admission = matches!(
            command,
            super::writer::Command::Update {
                operation: kiarni_engine::writes::Operation::Replace,
                ..
            }
        )
        .then(|| super::profiling::PUT_ADMISSION.start());
        let ticket = self
            .writer
            .submit_with_permit(command, Arc::clone(&state.permit))?;
        #[cfg(feature = "profiling")]
        drop(admission);
        state.reply = Some(ticket.wait()?);
        state.reply.as_ref().ok_or(Code::InternalError)?.outcome()
    }

    fn stream(&self, stream: &mut TcpStream, id: &RequestId, state: &RequestState) {
        use kiarni_feeds::{Terminal, sse};
        let Some(session) = state.session.as_ref() else {
            return;
        };
        let Some(reply) = state.reply.as_ref() else {
            return;
        };
        let Ok(super::writer::Outcome::Subscribed(subscription)) = reply.outcome() else {
            return;
        };
        let fail = || {
            subscription.terminate(if self.stop.load(Ordering::Relaxed) {
                Terminal::Shutdown
            } else {
                Terminal::Interrupted
            })
        };
        let headers = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream; charset=utf-8\r\nCache-Control: no-store\r\nConnection: close\r\nX-Request-ID: {}\r\nX-Accel-Buffering: no\r\n\r\n",
            id.as_str()
        );
        let send = |stream: &mut TcpStream, bytes: &[u8], deadline: Instant| {
            self.stream_write(stream, bytes, deadline, Some((session, subscription)))
        };
        let deadline = Instant::now() + self.write_timeout;
        if self
            .stream_write(stream, headers.as_bytes(), deadline, None)
            .and_then(|()| self.stream_write(stream, sse::READY, deadline, None))
            .is_err()
        {
            fail();
            return;
        }
        let mut heartbeat = Instant::now() + self.heartbeat;
        let mut buffer = Vec::with_capacity(8192);
        let reason = loop {
            if self.stop.load(Ordering::Relaxed) {
                break Terminal::Shutdown;
            }
            if super::now()
                .ok()
                .is_none_or(|now| self.authority.revalidate(session, now).is_err())
            {
                break Terminal::Interrupted;
            }
            let mut events = [PollFd::new(&*stream, PollFlags::IN)];
            if poll(
                &mut events,
                Some(&Timespec {
                    tv_sec: 0,
                    tv_nsec: 0,
                }),
            )
            .is_err()
            {
                fail();
                return;
            }
            if !events[0].revents().is_empty() {
                let mut byte = [0];
                let _ = stream.set_read_timeout(Some(POLL));
                match stream.peek(&mut byte) {
                    Err(error) if retryable(&error) => {}
                    _ => return,
                }
            }
            match subscription.recv_timeout(POLL) {
                Err(reason) => break reason,
                Ok(None) => {
                    if Instant::now() >= heartbeat {
                        if send(stream, sse::HEARTBEAT, Instant::now() + self.write_timeout)
                            .is_err()
                        {
                            fail();
                            return;
                        }
                        heartbeat = Instant::now() + self.heartbeat;
                    }
                }
                Ok(Some(event)) => {
                    let Ok(frame) = sse::Frame::new(&event) else {
                        break Terminal::Interrupted;
                    };
                    let deadline = Instant::now() + self.write_timeout;
                    buffer.clear();
                    let result = frame
                        .write_with(|mut part| -> io::Result<()> {
                            while !part.is_empty() {
                                let count = (8192 - buffer.len()).min(part.len());
                                buffer.extend_from_slice(&part[..count]);
                                part = &part[count..];
                                if buffer.len() == 8192 {
                                    send(stream, &buffer, deadline)?;
                                    buffer.clear();
                                }
                            }
                            Ok(())
                        })
                        .and_then(|()| send(stream, &buffer, deadline));
                    // A partial frame cannot be followed by a terminal frame: close as interrupted EOF.
                    if result.is_err() {
                        fail();
                        return;
                    }
                }
            }
        };
        subscription.terminate(reason);
        let reason = subscription.terminal_reason().unwrap_or(reason);
        let _ = self.stream_write(stream, sse::terminal(reason), Instant::now() + POLL, None);
        let _ = stream.shutdown(Shutdown::Write);
    }

    fn stream_write(
        &self,
        stream: &mut TcpStream,
        mut bytes: &[u8],
        deadline: Instant,
        active: Option<(&kiarni_auth::Session, &kiarni_feeds::Subscription)>,
    ) -> io::Result<()> {
        while !bytes.is_empty() {
            if let Some((session, subscription)) = active
                && (self.stop.load(Ordering::Relaxed)
                    || subscription.terminal_reason().is_some()
                    || super::now()
                        .ok()
                        .is_none_or(|now| self.authority.revalidate(session, now).is_err()))
            {
                return Err(io::ErrorKind::Interrupted.into());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(io::ErrorKind::TimedOut.into());
            }
            stream.set_write_timeout(Some(remaining.min(POLL)))?;
            match stream.write(bytes) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(count) => bytes = &bytes[count..],
                Err(error) if retryable(&error) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    fn read_body(
        &self,
        stream: &mut TcpStream,
        initial: &[u8],
        length: usize,
        deadline: Instant,
    ) -> Result<Zeroizing<Vec<u8>>, Code> {
        let mut body = Zeroizing::new(Vec::with_capacity(length));
        body.extend_from_slice(initial);
        let mut chunk = Zeroizing::new([0u8; 512]);
        while body.len() < length {
            if self.stop.load(Ordering::Relaxed) {
                return Err(Code::ShuttingDown);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(Code::RequestTimeout);
            }
            stream
                .set_read_timeout(Some(remaining.min(POLL)))
                .map_err(|_| Code::InternalError)?;
            let wanted = (length - body.len()).min(chunk.len());
            match stream.read(&mut chunk[..wanted]) {
                Ok(0) => return Err(Code::InvalidJson),
                Ok(count) => body.extend_from_slice(&chunk[..count]),
                Err(error) if retryable(&error) => {}
                Err(_) => return Err(Code::InvalidJson),
            }
        }
        Ok(body)
    }

    fn write_response(
        &self,
        stream: &mut TcpStream,
        headers: &[u8],
        body: &[u8],
        deadline: Instant,
    ) -> io::Result<()> {
        // One vectored send avoids both a body copy and a separate tiny header write.
        let mut buffers = [io::IoSlice::new(headers), io::IoSlice::new(body)];
        let mut remaining_buffers = &mut buffers[..];
        while !remaining_buffers.is_empty() {
            if self.stop.load(Ordering::Relaxed) {
                return Err(io::ErrorKind::Interrupted.into());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(io::ErrorKind::TimedOut.into());
            }
            stream.set_write_timeout(Some(remaining.min(POLL)))?;
            let written = if body.is_empty() {
                stream.write(remaining_buffers[0].as_ref())
            } else {
                stream.write_vectored(remaining_buffers)
            };
            match written {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(count) => io::IoSlice::advance_slices(&mut remaining_buffers, count),
                Err(error) if retryable(&error) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
}

fn retryable(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
    )
}

fn check_readiness(store: &kiarni_storage::Store) -> Result<(), Code> {
    store.check_readable().map_err(|_| Code::StorageUnavailable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_uses_read_snapshot_independently_of_query_gate() {
        let temp = tempfile::tempdir().unwrap();
        let store = kiarni_storage::Store::create(&temp.path().join("store")).unwrap();
        let query = crate::gate::Gate::default();
        let stop = AtomicBool::new(false);
        let _held = query.enter(&stop, Code::ServiceUnavailable).unwrap();
        check_readiness(&store).unwrap();
    }
}
