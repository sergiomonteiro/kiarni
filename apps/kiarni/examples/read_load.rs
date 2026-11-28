//! Bounded, read-only benchmark client. Not linked into the Kiarni server.
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    sync::{
        Arc, Barrier, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

const INPUT_LIMIT: usize = 8 * 1024 * 1024;
const HEADER_LIMIT: usize = 8192;
const BODY_LIMIT: usize = 64 * 1024;
const CAP: usize = 100_000;
const TIMEOUT: Duration = Duration::from_secs(5);

struct Config {
    address: SocketAddr,
    token: String,
    documents: Vec<Value>,
    clients: usize,
    warmup: usize,
    duration: Duration,
}

fn integer(value: &Value, key: &str, min: u64, max: u64) -> Result<usize, &'static str> {
    let n = value
        .get(key)
        .and_then(Value::as_u64)
        .ok_or("invalid_config")?;
    if !(min..=max).contains(&n) {
        return Err("invalid_config");
    }
    Ok(n as usize)
}

fn config(value: Value) -> Result<Config, &'static str> {
    let address: SocketAddr = value
        .get("address")
        .and_then(Value::as_str)
        .ok_or("invalid_address")?
        .parse()
        .map_err(|_| "invalid_address")?;
    if !address.ip().is_loopback() || address.port() == 0 {
        return Err("non_loopback_address");
    }
    let token = value
        .get("token")
        .and_then(Value::as_str)
        .ok_or("invalid_token")?;
    if token.is_empty() || token.len() > 512 || !token.bytes().all(|c| (33..=126).contains(&c)) {
        return Err("invalid_token");
    }
    let documents = value
        .get("documents")
        .and_then(Value::as_array)
        .ok_or("invalid_documents")?;
    if documents.is_empty() || documents.len() > 1024 {
        return Err("invalid_documents");
    }
    for (i, doc) in documents.iter().enumerate() {
        if !doc.is_object()
            || doc.get("_id").and_then(Value::as_str) != Some(format!("d{i:06}").as_str())
            || serde_json::to_vec(doc)
                .map_err(|_| "invalid_documents")?
                .len()
                > 4096
        {
            return Err("invalid_documents");
        }
    }
    Ok(Config {
        address,
        token: token.to_owned(),
        documents: documents.clone(),
        clients: integer(&value, "clients", 1, 32)?,
        warmup: integer(&value, "warmup", 0, 1000)?,
        duration: Duration::from_millis(integer(&value, "duration_ms", 100, 10_000)? as u64),
    })
}

#[derive(Clone, Copy)]
struct Failure {
    status: u16,
    code: &'static str,
}

fn failure(code: &'static str) -> Failure {
    Failure { status: 0, code }
}
fn network(error: std::io::Error) -> Failure {
    failure(
        if matches!(
            error.kind(),
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
        ) {
            "timeout"
        } else {
            "transport_error"
        },
    )
}
fn remaining(deadline: Instant) -> Result<Duration, Failure> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        Err(failure("timeout"))
    } else {
        Ok(remaining)
    }
}

fn headers(bytes: &[u8]) -> Result<(u16, usize), Failure> {
    let text = std::str::from_utf8(bytes).map_err(|_| failure("invalid_headers"))?;
    let mut lines = text.split("\r\n");
    let mut status = lines
        .next()
        .ok_or(failure("invalid_headers"))?
        .split_whitespace();
    if status.next() != Some("HTTP/1.1") {
        return Err(failure("invalid_headers"));
    }
    let status: u16 = status
        .next()
        .ok_or(failure("invalid_headers"))?
        .parse()
        .map_err(|_| failure("invalid_headers"))?;
    if !(100..=599).contains(&status) {
        return Err(failure("invalid_headers"));
    }
    let mut length = None;
    for line in lines.filter(|line| !line.is_empty()) {
        let (name, value) = line.split_once(':').ok_or(failure("invalid_headers"))?;
        if name.eq_ignore_ascii_case("transfer-encoding") {
            return Err(failure("invalid_framing"));
        }
        if name.eq_ignore_ascii_case("content-length") {
            let value = value.trim();
            if length.is_some() || value.is_empty() || !value.bytes().all(|v| v.is_ascii_digit()) {
                return Err(failure("invalid_framing"));
            }
            let n: usize = value.parse().map_err(|_| failure("invalid_framing"))?;
            if n > BODY_LIMIT {
                return Err(failure("oversized_response"));
            }
            length = Some(n);
        }
    }
    Ok((status, length.ok_or(failure("invalid_framing"))?))
}

fn request(
    config: &Config,
    request: &[u8],
    expected: &Value,
    bytes: &mut Vec<u8>,
) -> Result<(), Failure> {
    let deadline = Instant::now() + TIMEOUT;
    let mut stream = TcpStream::connect_timeout(&config.address, TIMEOUT).map_err(network)?;
    let mut sent = 0;
    while sent < request.len() {
        stream
            .set_write_timeout(Some(remaining(deadline)?))
            .map_err(network)?;
        match stream.write(&request[sent..]) {
            Ok(0) => return Err(failure("write_zero")),
            Ok(n) => sent += n,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(network(error)),
        }
    }
    bytes.clear();
    let mut buffer = [0; 8192];
    let end = loop {
        if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
            if end + 4 > HEADER_LIMIT {
                return Err(failure("oversized_headers"));
            }
            break end + 4;
        }
        if bytes.len() >= HEADER_LIMIT {
            return Err(failure("oversized_headers"));
        }
        stream
            .set_read_timeout(Some(remaining(deadline)?))
            .map_err(network)?;
        let n = stream.read(&mut buffer).map_err(network)?;
        if n == 0 {
            return Err(failure("truncated_headers"));
        }
        bytes.extend_from_slice(&buffer[..n]);
    };
    let (status, length) = headers(&bytes[..end])?;
    if bytes.len() > end + length {
        return Err(failure("invalid_framing"));
    }
    while bytes.len() < end + length {
        stream
            .set_read_timeout(Some(remaining(deadline)?))
            .map_err(network)?;
        let limit = (end + length - bytes.len()).min(buffer.len());
        let n = stream.read(&mut buffer[..limit]).map_err(network)?;
        if n == 0 {
            return Err(failure("truncated_body"));
        }
        bytes.extend_from_slice(&buffer[..n]);
    }
    if status != 200 {
        return Err(Failure {
            status,
            code: "unexpected_http_status",
        });
    }
    let document: Value =
        serde_json::from_slice(&bytes[end..]).map_err(|_| failure("invalid_json"))?;
    if &document != expected {
        return Err(Failure {
            status: 200,
            code: "invalid_success_response",
        });
    }
    Ok(())
}

#[derive(Default)]
struct Outcome {
    good: Vec<u64>,
    bad: Vec<u64>,
    statuses: BTreeMap<String, usize>,
    codes: BTreeMap<&'static str, usize>,
    warmup_errors: usize,
    cap_hit: bool,
    ended: Duration,
}

fn record(result: Result<(), Failure>, elapsed: Duration, outcome: &mut Outcome) {
    let ns = elapsed.as_nanos().min(u64::MAX as u128) as u64;
    match result {
        Ok(()) => {
            *outcome.statuses.entry("200".into()).or_default() += 1;
            outcome.good.push(ns);
        }
        Err(error) => {
            let status = if error.status == 0 {
                "transport_error".into()
            } else {
                error.status.to_string()
            };
            *outcome.statuses.entry(status).or_default() += 1;
            *outcome.codes.entry(error.code).or_default() += 1;
            outcome.bad.push(ns);
        }
    }
}

fn percentiles(samples: &mut [u64]) -> Value {
    if samples.is_empty() {
        return Value::Null;
    }
    samples.sort_unstable();
    let at = |p: usize| samples[(samples.len() * p).div_ceil(100) - 1] as f64 / 1_000_000.;
    json!({"p50_ms": at(50), "p95_ms": at(95), "p99_ms": at(99), "max_ms": at(100)})
}

fn cpu_list(text: &str) -> Option<Vec<usize>> {
    let list = text
        .lines()
        .find_map(|line| line.strip_prefix("Cpus_allowed_list:"))?
        .trim();
    let mut cpus = Vec::new();
    for part in list.split(',') {
        let (first, last) = if let Some((first, last)) = part.split_once('-') {
            (first.parse::<usize>().ok()?, last.parse::<usize>().ok()?)
        } else {
            let n = part.parse::<usize>().ok()?;
            (n, n)
        };
        if first > last || last > 4095 {
            return None;
        }
        cpus.extend(first..=last);
    }
    Some(cpus)
}

fn run(config: Config) -> Result<Value, &'static str> {
    let clients = config.clients;
    let config = Arc::new(config);
    let ready = Arc::new(Barrier::new(clients + 1));
    let begin = Arc::new(Barrier::new(clients + 1));
    let clock = Arc::new(OnceLock::<Instant>::new());
    let failed_warmup = Arc::new(AtomicBool::new(false));
    let mut workers = Vec::with_capacity(clients);
    for client in 0..clients {
        let (config, ready, begin, clock, failed) = (
            Arc::clone(&config),
            Arc::clone(&ready),
            Arc::clone(&begin),
            Arc::clone(&clock),
            Arc::clone(&failed_warmup),
        );
        workers.push(thread::Builder::new().stack_size(256 * 1024).spawn(move || {
            let requests: Vec<Vec<u8>> = (0..config.documents.len()).map(|i| {
                format!("GET /v1/collections/bench/documents/d{i:06} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nAccept-Encoding: identity\r\nConnection: close\r\n\r\n", config.address, config.token).into_bytes()
            }).collect();
            let mut bytes = Vec::with_capacity(16 * 1024);
            let mut outcome = Outcome::default();
            for step in 0..config.warmup {
                let i = (client * 97 + step) % requests.len();
                if request(&config, &requests[i], &config.documents[i], &mut bytes).is_err() {
                    outcome.warmup_errors += 1; failed.store(true, Ordering::Relaxed); break;
                }
            }
            ready.wait();
            begin.wait();
            let Some(start) = clock.get().copied() else { return Err("missing_clock"); };
            if !failed.load(Ordering::Relaxed) {
                let deadline = start + config.duration;
                let mut step = 0;
                while Instant::now() < deadline && step < CAP {
                    let i = (client * 97 + config.warmup + step) % requests.len();
                    let now = Instant::now();
                    let result = request(&config, &requests[i], &config.documents[i], &mut bytes);
                    record(result, now.elapsed(), &mut outcome);
                    step += 1;
                }
                outcome.cap_hit = step == CAP;
            }
            outcome.ended = start.elapsed();
            Ok(outcome)
        }).map_err(|_| "spawn_failed")?);
    }
    ready.wait();
    clock.set(Instant::now()).map_err(|_| "clock_already_set")?;
    begin.wait();
    let mut combined = Outcome::default();
    for worker in workers {
        let mut outcome = worker.join().map_err(|_| "worker_panicked")??;
        combined.good.append(&mut outcome.good);
        combined.bad.append(&mut outcome.bad);
        for (key, n) in outcome.statuses {
            *combined.statuses.entry(key).or_default() += n;
        }
        for (key, n) in outcome.codes {
            *combined.codes.entry(key).or_default() += n;
        }
        combined.warmup_errors += outcome.warmup_errors;
        combined.cap_hit |= outcome.cap_hit;
        combined.ended = combined.ended.max(outcome.ended);
    }
    let elapsed = combined.ended.as_secs_f64();
    let confirmed = combined.good.len();
    let attempts = confirmed + combined.bad.len();
    Ok(
        json!({"clients": clients, "elapsed_seconds": elapsed, "attempts": attempts,
        "confirmed": confirmed, "confirmed_ops_per_second": confirmed as f64 / elapsed,
        "success_latency": percentiles(&mut combined.good), "failure_latency": percentiles(&mut combined.bad),
        "statuses": combined.statuses, "error_codes": combined.codes, "warmup_errors": combined.warmup_errors,
        "operation_cap_hit": combined.cap_hit,
        "client_affinity": std::fs::read_to_string("/proc/self/status").ok().and_then(|s| cpu_list(&s)),
        "valid": confirmed > 0 && confirmed == attempts && combined.warmup_errors == 0 && !combined.cap_hit}),
    )
}

fn execute() -> Result<Value, &'static str> {
    let mut input = Vec::new();
    std::io::stdin()
        .lock()
        .take((INPUT_LIMIT + 1) as u64)
        .read_to_end(&mut input)
        .map_err(|_| "stdin_failed")?;
    if input.len() > INPUT_LIMIT {
        return Err("oversized_config");
    }
    let value = serde_json::from_slice(&input).map_err(|_| "invalid_config_json")?;
    run(config(value)?)
}

fn main() {
    match execute() {
        Ok(value) => println!("{value}"),
        Err(code) => {
            eprintln!("{code}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framing_rejects_ambiguous_and_unbounded_responses() {
        assert_eq!(
            headers(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\n").ok(),
            Some((200, 3))
        );
        for bytes in [
            b"HTTP/1.1 200 OK\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nContent-Length: 3\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Length: 3\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nContent-Length: 1048577\r\n\r\n",
        ] {
            assert!(headers(bytes).is_err());
        }
    }

    #[test]
    fn expected_json_and_config_are_bounded_and_loopback_only() {
        let value = json!({"address":"127.0.0.1:1234", "token":"fixture", "clients":1,
            "warmup":0,"duration_ms":100,"documents":[{"_id":"d000000"}]});
        assert!(config(value.clone()).is_ok());
        for (key, replacement) in [
            ("clients", json!(33)),
            ("address", json!("192.0.2.1:1234")),
            ("token", json!("fixture\r\ninjected")),
            ("documents", json!([])),
        ] {
            let mut changed = value.clone();
            changed[key] = replacement;
            assert!(config(changed).is_err());
        }
    }

    #[test]
    fn affinity_metadata_handles_linux_ranges_without_assuming_cpu_numbering() {
        assert_eq!(cpu_list("Cpus_allowed_list:\t2-3,7\n"), Some(vec![2, 3, 7]));
        assert!(cpu_list("Cpus_allowed_list: 7-2").is_none());
        assert!(cpu_list("Cpus_allowed_list: 0-999999").is_none());
    }

    #[test]
    fn errors_do_not_enter_success_percentiles() {
        let mut outcome = Outcome::default();
        record(Ok(()), Duration::from_millis(2), &mut outcome);
        record(
            Err(Failure {
                status: 503,
                code: "unexpected_http_status",
            }),
            Duration::from_millis(50),
            &mut outcome,
        );
        assert_eq!(outcome.good.len(), 1);
        assert_eq!(outcome.bad.len(), 1);
        assert_eq!(percentiles(&mut outcome.good)["p95_ms"], 2.);
        assert_eq!(outcome.statuses["503"], 1);
    }

    #[test]
    fn real_socket_validates_fragmented_json_and_rejects_wrong_document() {
        use std::net::TcpListener;
        for (body, valid) in [
            (br#"{"_id":"d000000","revision":0}"#.as_slice(), true),
            (br#"{"_id":"d000000","revision":1}"#.as_slice(), false),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let body = body.to_vec();
            let server = thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buffer = [0; 2048];
                let mut received = Vec::new();
                while !received.windows(4).any(|v| v == b"\r\n\r\n") {
                    let n = stream.read(&mut buffer).unwrap();
                    assert!(n > 0);
                    received.extend_from_slice(&buffer[..n]);
                }
                let header = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len());
                stream.write_all(header.as_bytes()).unwrap();
                for chunk in body.chunks(3) {
                    stream.write_all(chunk).unwrap();
                }
            });
            let expected = json!({"_id":"d000000","revision":0});
            let config = Config {
                address,
                token: "fixture".into(),
                documents: vec![expected.clone()],
                clients: 1,
                warmup: 0,
                duration: Duration::from_millis(100),
            };
            let result = request(
                &config,
                b"GET / HTTP/1.1\r\n\r\n",
                &expected,
                &mut Vec::new(),
            );
            server.join().unwrap();
            assert_eq!(result.is_ok(), valid);
        }
    }

    #[test]
    fn truncated_body_is_rejected_instead_of_accepted_as_json() {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 20\r\n\r\n{}")
                .unwrap();
        });
        let expected = json!({"_id":"d000000"});
        let config = Config {
            address,
            token: "fixture".into(),
            documents: vec![expected.clone()],
            clients: 1,
            warmup: 0,
            duration: Duration::from_millis(100),
        };
        let error = request(
            &config,
            b"GET / HTTP/1.1\r\n\r\n",
            &expected,
            &mut Vec::new(),
        )
        .err()
        .unwrap();
        assert_eq!(error.code, "truncated_body");
        server.join().unwrap();
    }

    #[test]
    fn failed_warmup_releases_all_workers_without_running_a_measurement() {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                stream
                    .write_all(b"HTTP/1.1 503 Unavailable\r\nContent-Length: 2\r\n\r\n{}")
                    .unwrap();
            }
        });
        let result = run(Config {
            address,
            token: "fixture".into(),
            documents: vec![json!({"_id":"d000000"})],
            clients: 2,
            warmup: 1,
            duration: Duration::from_millis(100),
        })
        .unwrap();
        assert_eq!(result["valid"], false);
        assert_eq!(result["attempts"], 0);
        assert_eq!(result["warmup_errors"], 2);
        server.join().unwrap();
    }
}
