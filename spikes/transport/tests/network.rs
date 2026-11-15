#![cfg(unix)]
use kiarni_auth_spike::{Keyset, vault::Vault};
use kiarni_transport_spike::{Server, now};
use std::os::unix::fs::PermissionsExt;
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
    thread,
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

struct Fixture {
    server: Server,
    header: Zeroizing<Vec<u8>>,
    _data: tempfile::TempDir,
    _client: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        Self::with_deadline(Duration::from_millis(500))
    }

    fn with_deadline(deadline: Duration) -> Self {
        let data = tempfile::tempdir().unwrap();
        let client = tempfile::tempdir().unwrap();
        fs::set_permissions(data.path(), fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(client.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let (keys, token) = Keyset::bootstrap(now().unwrap(), 300).unwrap();
        let snapshot = Vault::open(data.path())
            .unwrap()
            .provision(&keys, &token, &Vault::open(client.path()).unwrap())
            .unwrap();
        let bytes =
            Zeroizing::new(fs::read(client.path().join(snapshot.token_file.unwrap())).unwrap());
        let mut header = Zeroizing::new(b"Authorization: Bearer ".to_vec());
        header.extend_from_slice(&bytes);
        header.extend_from_slice(b"\r\n");
        let server = Server::start(&data.path().join("db.redb"), keys, 1, deadline).unwrap();
        Self {
            server,
            header,
            _data: data,
            _client: client,
        }
    }

    fn request(
        &self,
        path: &str,
        authenticated: bool,
        extra: &str,
        receive_buffer: Option<usize>,
    ) -> Client {
        let mut stream = if let Some(bytes) = receive_buffer {
            // Advertise the deliberately small receive window during connect,
            // rather than shrinking a window already negotiated by TCP.
            let socket = rustix::net::socket(
                rustix::net::AddressFamily::INET,
                rustix::net::SocketType::STREAM,
                Some(rustix::net::ipproto::TCP),
            )
            .unwrap();
            rustix::net::sockopt::set_socket_recv_buffer_size(&socket, bytes).unwrap();
            rustix::net::connect(&socket, &self.server.address()).unwrap();
            TcpStream::from(socket)
        } else {
            TcpStream::connect(self.server.address()).unwrap()
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut request = Zeroizing::new(format!("GET {path} HTTP/1.1\r\nHost: {}\r\nAccept: text/event-stream\r\nConnection: close\r\n", self.server.address()).into_bytes());
        if authenticated {
            request.extend_from_slice(&self.header);
        }
        request.extend_from_slice(extra.as_bytes());
        request.extend_from_slice(b"\r\n");
        stream.write_all(&request).unwrap();
        Client::read_response(stream)
    }
}

struct Client {
    reader: BufReader<TcpStream>,
    status: u16,
    headers: String,
}
impl Client {
    fn read_response(stream: TcpStream) -> Self {
        // Byte-at-a-time header read deliberately avoids prefetching the SSE
        // body, especially for the consumer that will stop reading entirely.
        let mut stream = stream;
        let mut bytes = Vec::new();
        while !bytes.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            stream.read_exact(&mut byte).unwrap();
            bytes.push(byte[0]);
            assert!(bytes.len() <= 8192);
        }
        let headers = String::from_utf8(bytes).unwrap();
        let status = headers.split_whitespace().nth(1).unwrap().parse().unwrap();
        Self {
            reader: BufReader::new(stream),
            status,
            headers,
        }
    }

    fn frame(&mut self) -> String {
        let mut frame = String::new();
        loop {
            let mut line = String::new();
            let count = self.reader.read_line(&mut line).unwrap();
            if count == 0 {
                assert!(frame.is_empty(), "truncated SSE frame");
                return frame;
            }
            frame.push_str(&line);
            assert!(frame.len() <= 3 * 1024 * 1024);
            if line == "\n" {
                return frame;
            }
        }
    }

    fn event(&mut self, name: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let frame = self.frame();
            assert!(!frame.is_empty(), "stream closed before expected event");
            if frame.starts_with(&format!("event: {name}\n")) {
                return frame;
            }
            assert!(Instant::now() < deadline);
        }
    }
}

fn wait(mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !predicate() {
        assert!(Instant::now() < deadline, "condition deadline exceeded");
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn two_tcp_clients_receive_postcommit_events_with_visible_documents() {
    let fixture = Fixture::new();
    assert!(fixture.server.address().ip().is_loopback());
    let mut first = fixture.request("/v1/collections/orders/changes", true, "", None);
    let mut second = fixture.request("/v1/collections/orders/changes", true, "", None);
    assert_eq!(first.status, 200);
    assert_eq!(second.status, 200);
    assert!(
        first
            .headers
            .contains("Content-Type: text/event-stream; charset=utf-8")
    );
    assert!(first.headers.contains("Cache-Control: no-store"));
    first.event("ready");
    second.event("ready");
    assert_eq!(
        fixture
            .server
            .insert(br#"{"_id":"a","status":"paid"}"#)
            .unwrap(),
        1
    );
    let a = first.event("change");
    let b = second.event("change");
    assert_eq!(a, b);
    assert!(a.contains("\"sequence\":\"1\""));
    let mut document = fixture.request("/v1/collections/orders/documents/a", true, "", None);
    assert_eq!(document.status, 200);
    let mut body = String::new();
    document.reader.read_to_string(&mut body).unwrap();
    assert_eq!(body, "{\"_id\":\"a\",\"status\":\"paid\"}");
    drop(first);
    drop(second);
    wait(|| fixture.server.counters().snapshot()["streams"] == 0);
}

#[test]
fn authentication_resume_origin_and_limits_are_checked_before_subscription() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture
            .request("/v1/collections/orders/changes", false, "", None)
            .status,
        401
    );
    assert_eq!(fixture.request("/v1/unknown", false, "", None).status, 401);
    assert_eq!(
        fixture
            .request(
                "/v1/collections/orders/changes",
                true,
                "Last-Event-ID: 1\r\n",
                None
            )
            .status,
        400
    );
    assert_eq!(
        fixture
            .request(
                "/v1/collections/orders/changes",
                true,
                "Origin: http://example.invalid\r\n",
                None
            )
            .status,
        403
    );
    assert_eq!(fixture.server.counters().snapshot()["subscriptions"], 0);
    let mut denied = fixture.request(
        "/v1/collections/orders/changes",
        false,
        "X-Request-ID: untrusted-fixture-id\r\n",
        None,
    );
    assert_eq!(denied.status, 401);
    assert!(denied.headers.contains("WWW-Authenticate: Bearer"));
    let mut body = String::new();
    denied.reader.read_to_string(&mut body).unwrap();
    assert!(!denied.headers.contains("untrusted-fixture-id"));
    assert!(!body.contains("untrusted-fixture-id"));
    assert_eq!(
        fixture
            .request(
                "/v1/collections/orders/changes",
                true,
                "Authorization: Bearer invalid\r\n",
                None
            )
            .status,
        401
    );
    assert_eq!(fixture.server.counters().snapshot()["subscriptions"], 0);
    let mut active = Vec::new();
    for _ in 0..4 {
        let mut stream = fixture.request("/v1/collections/orders/changes", true, "", None);
        stream.event("ready");
        active.push(stream);
    }
    assert_eq!(
        fixture
            .request("/v1/collections/orders/changes", true, "", None)
            .status,
        429
    );
    drop(active);
    wait(|| fixture.server.counters().snapshot()["streams"] == 0);
}

#[test]
fn tcp_slow_consumer_hits_a_write_deadline_without_stalling_the_writer_or_fast_client() {
    let fixture = Fixture::new();
    let mut slow = fixture.request("/v1/collections/orders/changes", true, "", Some(4096));
    slow.event("ready");
    // Stop reading this socket entirely; its receive buffer cannot absorb a frame.
    let mut fast = fixture.request("/v1/collections/orders/changes", true, "", None);
    fast.event("ready");
    let start = Instant::now();
    for i in 0..6 {
        let json = format!(
            r#"{{"_id":"id{i}","status":"new","payload":"{}"}}"#,
            "x".repeat(250000)
        );
        assert_eq!(fixture.server.insert(json.as_bytes()).unwrap(), i + 1);
        assert!(
            fast.event("change")
                .contains(&format!("\"sequence\":\"{}\"", i + 1))
        );
        // Keep at most one event queued behind a blocked write. Otherwise the
        // overflow policy can close the slow stream before its socket deadline,
        // depending on kernel buffering and scheduler speed.
        wait(|| {
            let counters = fixture.server.counters().snapshot();
            counters["write_timeouts"].as_u64().unwrap() >= 1
                || counters["event_writes"].as_u64().unwrap() >= 2 * (i + 1)
        });
    }
    assert!(start.elapsed() < Duration::from_secs(3));
    wait(|| {
        fixture.server.counters().snapshot()["write_timeouts"]
            .as_u64()
            .unwrap()
            >= 1
    });
    wait(|| fixture.server.counters().snapshot()["streams"] == 1);
    assert_eq!(fixture.server.counters().snapshot()["overflows"], 0);
    assert_eq!(
        fixture
            .server
            .insert(br#"{"_id":"after-slow","status":"new"}"#)
            .unwrap(),
        7
    );
    assert!(fast.event("change").contains("after-slow"));
    drop(slow);
    drop(fast);
    wait(|| fixture.server.counters().snapshot()["streams"] == 0);
}

#[test]
fn revocation_and_shutdown_close_live_streams_and_release_workers() {
    let mut fixture = Fixture::new();
    let mut stream = fixture.request("/v1/collections/orders/changes", true, "", None);
    stream.event("ready");
    fixture.server.revoke().unwrap();
    assert!(stream.event("stream_end").contains("interrupted"));
    wait(|| fixture.server.counters().snapshot()["streams"] == 0);
    assert_eq!(
        fixture
            .request("/v1/collections/orders/changes", true, "", None)
            .status,
        401
    );
    fixture.server.shutdown();
    assert_eq!(fixture.server.counters().snapshot()["connections"], 0);
}

#[test]
fn queued_overflow_is_explicit_on_the_wire_after_paused_consumer_resumes() {
    let fixture = Fixture::new();
    let mut client = fixture.request("/v1/collections/orders/changes", true, "", None);
    client.event("ready");
    let (paused, release) = fixture.server.pause_next_event_write().unwrap();
    fixture
        .server
        .insert(br#"{"_id":"first","status":"new"}"#)
        .unwrap();
    paused.recv_timeout(Duration::from_secs(3)).unwrap();
    // Pause after dequeue but before socket I/O. Fill the one-slot queue and
    // overflow it deterministically, independent of kernel receive windows.
    fixture
        .server
        .insert(br#"{"_id":"queued","status":"new"}"#)
        .unwrap();
    fixture
        .server
        .insert(br#"{"_id":"overflow","status":"new"}"#)
        .unwrap();
    release.send(()).unwrap();
    assert!(client.event("change").contains("\"id\":\"first\""));
    assert!(client.event("stream_end").contains("overflow"));
    wait(|| fixture.server.counters().snapshot()["streams"] == 0);
    assert_eq!(fixture.server.counters().snapshot()["overflows"], 1);
    assert_eq!(fixture.server.counters().snapshot()["write_timeouts"], 0);
}

#[test]
fn shutdown_with_a_live_stream_delivers_terminal_and_joins_every_worker() {
    let mut fixture = Fixture::new();
    let mut stream = fixture.request("/v1/collections/orders/changes", true, "", None);
    stream.event("ready");
    let started = Instant::now();
    fixture.server.shutdown();
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(stream.event("stream_end").contains("shutdown"));
    assert_eq!(fixture.server.counters().snapshot()["connections"], 0);
    assert_eq!(fixture.server.counters().snapshot()["streams"], 0);
}
