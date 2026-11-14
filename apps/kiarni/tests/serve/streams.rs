use super::*;
use std::io::{BufRead, BufReader};

struct Live {
    reader: BufReader<TcpStream>,
}
impl Live {
    fn frame(&mut self) -> String {
        let mut frame = String::new();
        loop {
            let mut line = String::new();
            assert!(
                self.reader.read_line(&mut line).unwrap() > 0,
                "unexpected stream EOF"
            );
            let done = line == "\n";
            frame.push_str(&line);
            assert!(frame.len() <= 3 * 1024 * 1024 + 1024);
            if done {
                return frame;
            }
        }
    }
    fn change(&mut self) -> String {
        loop {
            let frame = self.frame();
            if !frame.starts_with(':') {
                assert!(frame.starts_with("event: change\n"));
                return frame;
            }
        }
    }
}
fn open(addr: SocketAddr, key: &str) -> Result<Live, String> {
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(1)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    write!(stream,"GET /v1/collections/orders/changes HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {key}\r\nAccept: text/event-stream\r\n\r\n").unwrap();
    let mut reader = BufReader::new(stream);
    let mut headers = String::new();
    loop {
        let mut line = String::new();
        assert!(reader.read_line(&mut line).unwrap() > 0);
        let done = line == "\r\n";
        headers.push_str(&line);
        if done {
            break;
        }
    }
    if !headers.starts_with("HTTP/1.1 200 ") {
        let mut body = String::new();
        reader.read_to_string(&mut body).unwrap();
        return Err(headers + &body);
    }
    assert!(headers.contains("Content-Type: text/event-stream; charset=utf-8"));
    assert!(headers.contains("Cache-Control: no-store"));
    assert!(headers.contains("X-Request-ID: "));
    assert!(!headers.contains("Content-Length:"));
    let mut live = Live { reader };
    assert_eq!(
        live.frame(),
        "event: ready\ndata: {\"snapshot\":false,\"replay\":false}\n\n"
    );
    Ok(live)
}
fn data(frame: &str) -> serde_json::Value {
    let text = frame
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .collect::<Vec<_>>()
        .join("\n");
    serde_json::from_str(&text).unwrap()
}
fn setup(extra: &[&str]) -> (tempfile::TempDir, PathBuf, SocketAddr, Server, String) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    let addr = address();
    let mut server = Server(command(&root, addr).args(extra).spawn().unwrap());
    wait_until_listening(&mut server, addr);
    let key = token(&root);
    assert!(create(addr, &key, "orders").starts_with("HTTP/1.1 201 "));
    (dir, root, addr, server, key)
}

#[test]
fn two_authenticated_streams_receive_ordered_changes_noops_and_future_only_reconnect() {
    let _guard = SERVER_SCENARIOS.lock().unwrap_or_else(|e| e.into_inner());
    let (_dir, _root, addr, mut server, key) = setup(&["--heartbeat-ms", "1000"]);
    let mut one = open(addr, &key).unwrap();
    let mut two = open(addr, &key).unwrap();
    assert!(
        insert_document(
            addr,
            &key,
            "orders",
            r#"{"_id":"doc","n":9007199254740993}"#
        )
        .starts_with("HTTP/1.1 201 ")
    );
    for stream in [&mut one, &mut two] {
        let frame = stream.change();
        assert!(frame.contains("9007199254740993"));
        let value = data(&frame);
        assert_eq!(value["type"], "insert");
        assert_eq!(value["sequence"], "1");
        assert!(value["before"].is_null());
    }
    assert!(update_document(addr, &key, "PATCH", "doc", "{}").starts_with("HTTP/1.1 200 "));
    assert!(
        update_document(addr, &key, "PATCH", "doc", r#"{"_id":"wrong"}"#)
            .starts_with("HTTP/1.1 422 ")
    );
    assert_eq!(one.frame(), ": heartbeat\n\n");
    assert!(update_document(addr, &key, "PATCH", "doc", r#"{"v":2}"#).starts_with("HTTP/1.1 200 "));
    for stream in [&mut one, &mut two] {
        let value = data(&stream.change());
        assert_eq!(value["sequence"], "2");
        assert_eq!(value["type"], "update");
        assert_eq!(value["after"]["v"], 2);
    }
    drop(one);
    let mut replacement = open(addr, &key).unwrap();
    assert!(
        request(
            addr,
            "DELETE",
            "/v1/collections/orders/documents/doc",
            Some(&key)
        )
        .starts_with("HTTP/1.1 204 ")
    );
    for stream in [&mut replacement, &mut two] {
        let value = data(&stream.change());
        assert_eq!(value["sequence"], "3");
        assert_eq!(value["type"], "delete");
        assert!(value["after"].is_null());
    }
    server.terminate();
    assert!(replacement.frame().contains("\"code\":\"shutdown\""));
}

#[test]
fn stream_validation_returns_normal_authenticated_errors_before_success_headers() {
    let _guard = SERVER_SCENARIOS.lock().unwrap_or_else(|e| e.into_inner());
    let (_dir, _root, addr, mut server, key) = setup(&[]);
    let path = "/v1/collections/orders/changes";
    assert!(
        request_body(addr, "GET", path, None, "Accept: text/event-stream\r\n", "")
            .starts_with("HTTP/1.1 401 ")
    );
    for (extra, code) in [
        ("", "not_acceptable"),
        ("Accept: application/json\r\n", "not_acceptable"),
        (
            "Accept: text/event-stream\r\nLast-Event-ID: 1\r\n",
            "resume_not_supported",
        ),
        (
            "Accept: text/event-stream\r\nOrigin: http://other\r\n",
            "forbidden",
        ),
    ] {
        let response = request_body(addr, "GET", path, Some(&key), extra, "");
        assert!(response.contains(&format!("\"code\":\"{code}\"")));
        assert!(!response.contains("event: ready"));
    }
    assert!(
        request_body(
            addr,
            "GET",
            "/v1/collections/missing/changes",
            Some(&key),
            "Accept: text/event-stream\r\n",
            ""
        )
        .contains("\"code\":\"collection_not_found\"")
    );
    assert!(
        request_body(
            addr,
            "GET",
            "/v1/collections/orders/changes?token=fake",
            None,
            "Accept: text/event-stream\r\n",
            ""
        )
        .starts_with("HTTP/1.1 401 ")
    );
    let _live = open(addr, &key).unwrap();
    server.terminate();
}

#[test]
fn disconnect_releases_stream_capacity_and_heartbeat_keeps_idle_stream_live() {
    let _guard = SERVER_SCENARIOS.lock().unwrap_or_else(|e| e.into_inner());
    let (_dir, _root, addr, mut server, key) = setup(&[
        "--max-requests",
        "4",
        "--max-streams",
        "1",
        "--feed-queue",
        "1",
        "--heartbeat-ms",
        "1000",
    ]);
    let one = open(addr, &key).unwrap();
    assert!(
        open(addr, &key)
            .err()
            .unwrap()
            .contains("\"code\":\"subscription_limit\"")
    );
    assert!(request(addr, "GET", "/v1/health/ready", Some(&key)).starts_with("HTTP/1.1 200 "));
    drop(one);
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut replacement = loop {
        match open(addr, &key) {
            Ok(stream) => break stream,
            Err(_) => {
                assert!(Instant::now() < deadline);
                thread::sleep(Duration::from_millis(10));
            }
        }
    };
    assert_eq!(replacement.frame(), ": heartbeat\n\n");
    server.terminate();
}

#[test]
fn oversized_encoded_frame_ends_stream_without_undoing_committed_delete() {
    let _guard = SERVER_SCENARIOS.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
    let store = kiarni_storage::Store::create(&root.join("store")).unwrap();
    let collection = store.create_collection("orders").unwrap();
    store
        .insert_fixture(
            &collection,
            format!("{{{}\"_id\":\"doc\"}}", "\n".repeat(500_000)).as_bytes(),
        )
        .unwrap();
    drop(store);
    phase(&root, "initializing_store");
    let addr = address();
    let mut server = start(&root, addr);
    wait_until_listening(&mut server, addr);
    let key = token(&root);
    let mut stream = open(addr, &key).unwrap();
    assert!(
        request(
            addr,
            "DELETE",
            "/v1/collections/orders/documents/doc",
            Some(&key)
        )
        .starts_with("HTTP/1.1 204 ")
    );
    assert_eq!(data(&stream.frame())["code"], "interrupted");
    assert!(
        request(
            addr,
            "GET",
            "/v1/collections/orders/documents/doc",
            Some(&key)
        )
        .starts_with("HTTP/1.1 404 ")
    );
    server.terminate();
}

#[test]
fn nonreading_client_does_not_block_writes_and_its_slot_is_released() {
    let _guard = SERVER_SCENARIOS.lock().unwrap_or_else(|e| e.into_inner());
    let (_dir, _root, addr, mut server, key) = setup(&[
        "--max-requests",
        "4",
        "--max-streams",
        "1",
        "--feed-queue",
        "1",
        "--write-timeout-ms",
        "100",
    ]);
    let _slow = open(addr, &key).unwrap();
    let body = format!(r#"{{"_id":"doc","pad":"{}"}}"#, "x".repeat(800_000));
    assert!(insert_document(addr, &key, "orders", &body).starts_with("HTTP/1.1 201 "));
    for number in 1..=8 {
        assert!(
            update_document(addr, &key, "PATCH", "doc", &format!(r#"{{"v":{number}}}"#))
                .starts_with("HTTP/1.1 200 ")
        );
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    let _replacement = loop {
        match open(addr, &key) {
            Ok(stream) => break stream,
            Err(_) => {
                assert!(Instant::now() < deadline, "slow stream retained its slot");
                thread::sleep(Duration::from_millis(10));
            }
        }
    };
    let response = request(
        addr,
        "GET",
        "/v1/collections/orders/documents/doc",
        Some(&key),
    );
    assert!(response.starts_with("HTTP/1.1 200 "));
    assert!(response.contains("\"v\":8"));
    server.terminate();
}

#[test]
fn active_stream_is_interrupted_when_its_credential_expires() {
    let _guard = SERVER_SCENARIOS.lock().unwrap_or_else(|e| e.into_inner());
    let (_dir, root, addr, mut initial, _key) = setup(&[]);
    initial.terminate();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let activation = kiarni_auth::activation::Store::open(&root, None)
        .unwrap()
        .rotate(now, 3, 0)
        .unwrap();
    let key = fs::read_to_string(activation.token_file.unwrap()).unwrap();
    let mut server = Server(
        command(&root, addr)
            .args(["--heartbeat-ms", "1000"])
            .spawn()
            .unwrap(),
    );
    wait_until_listening(&mut server, addr);
    let mut live = open(addr, &key).unwrap();
    // The terminal frame is best-effort: expiry can interrupt a heartbeat write,
    // in which case the documented result is EOF without a terminal frame.
    let deadline = Instant::now() + Duration::from_secs(6);
    let mut remaining = String::new();
    loop {
        assert!(Instant::now() < deadline, "expired stream did not close");
        let mut line = String::new();
        if live.reader.read_line(&mut line).unwrap() == 0 {
            break;
        }
        remaining.push_str(&line);
        assert!(remaining.len() < 8192);
    }
    assert!(!remaining.contains("event: change"));
    if remaining.contains("event: stream_end\n") {
        assert!(remaining.contains("interrupted"));
    }
    assert!(request(addr, "GET", "/v1/health/ready", Some(&key)).starts_with("HTTP/1.1 401 "));
    server.terminate();
}

#[test]
fn concurrent_writers_produce_identical_ordered_transitions_for_two_real_streams() {
    let _guard = SERVER_SCENARIOS.lock().unwrap_or_else(|e| e.into_inner());
    let (_dir, _root, addr, mut server, key) = setup(&[
        "--max-requests",
        "16",
        "--max-streams",
        "2",
        "--feed-queue",
        "16",
    ]);
    assert!(
        insert_document(addr, &key, "orders", r#"{"_id":"doc","v":0}"#)
            .starts_with("HTTP/1.1 201 ")
    );
    let mut one = open(addr, &key).unwrap();
    let mut two = open(addr, &key).unwrap();
    let barrier = std::sync::Barrier::new(9);
    thread::scope(|scope| {
        let writers: Vec<_> = (1..=8)
            .map(|number| {
                let barrier = &barrier;
                let key = &key;
                scope.spawn(move || {
                    barrier.wait();
                    let response = update_document(
                        addr,
                        key,
                        "PATCH",
                        "doc",
                        &format!(r#"{{"v":{number},"f{number}":true}}"#),
                    );
                    assert!(response.starts_with("HTTP/1.1 200 "));
                    let value: serde_json::Value =
                        serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
                    assert_eq!(value["v"], number);
                })
            })
            .collect();
        barrier.wait();
        for writer in writers {
            writer.join().unwrap();
        }
    });
    let mut previous = 0;
    let mut values = std::collections::BTreeSet::new();
    for sequence in 2..=9 {
        let first = one.change();
        let second = two.change();
        assert_eq!(first, second);
        let value = data(&first);
        assert_eq!(value["sequence"], sequence.to_string());
        assert_eq!(value["before"]["v"], previous);
        previous = value["after"]["v"].as_u64().unwrap();
        values.insert(previous);
    }
    assert_eq!(values.len(), 8);
    server.terminate();
}

fn metrics(addr: SocketAddr, key: &str) -> String {
    let response = request(addr, "GET", "/v1/metrics", Some(key));
    assert!(response.starts_with("HTTP/1.1 200 "));
    assert!(response.contains("Content-Type: text/plain; version=0.0.4; charset=utf-8"));
    response.split_once("\r\n\r\n").unwrap().1.to_owned()
}
fn sample(text: &str, name: &str) -> f64 {
    text.lines()
        .find_map(|line| line.strip_prefix(&format!("{name} ")))
        .unwrap()
        .parse()
        .unwrap()
}

#[test]
fn authenticated_metrics_track_live_streams_confirmed_changes_and_cancellation() {
    let _guard = SERVER_SCENARIOS.lock().unwrap_or_else(|e| e.into_inner());
    let (_dir, _root, addr, mut server, key) = setup(&[]);
    assert!(request(addr, "GET", "/v1/metrics", None).starts_with("HTTP/1.1 401 "));
    assert!(request(addr, "POST", "/v1/metrics", Some(&key)).starts_with("HTTP/1.1 405 "));
    assert!(
        request(addr, "GET", "/v1/metrics?collection=orders", Some(&key))
            .starts_with("HTTP/1.1 400 ")
    );
    assert!(
        request_body(
            addr,
            "GET",
            "/v1/metrics",
            Some(&key),
            "Origin: http://other\r\n",
            ""
        )
        .starts_with("HTTP/1.1 403 ")
    );
    assert!(
        request_body(
            addr,
            "GET",
            "/v1/metrics",
            Some(&key),
            "Content-Length: 1\r\n",
            "x"
        )
        .starts_with("HTTP/1.1 400 ")
    );
    let mut live = open(addr, &key).unwrap();
    let text = metrics(addr, &key);
    assert_eq!(sample(&text, "kiarni_changefeed_subscriptions_active"), 1.0);
    assert_eq!(
        sample(&text, "kiarni_changefeed_events_published_total"),
        0.0
    );
    assert!(
        insert_document(
            addr,
            &key,
            "orders",
            r#"{"_id":"private_id","private_payload":"secret"}"#
        )
        .starts_with("HTTP/1.1 201 ")
    );
    live.change();
    assert!(update_document(addr, &key, "PATCH", "private_id", "{}").starts_with("HTTP/1.1 200 "));
    assert!(
        update_document(addr, &key, "PATCH", "private_id", r#"{"_id":"invalid"}"#)
            .starts_with("HTTP/1.1 422 ")
    );
    let text = metrics(addr, &key);
    assert_eq!(
        sample(&text, "kiarni_changefeed_events_published_total"),
        1.0
    );
    assert_eq!(
        sample(&text, "kiarni_changefeed_events_enqueued_total"),
        1.0
    );
    assert_eq!(
        sample(&text, "kiarni_changefeed_commit_to_dispatch_seconds_count"),
        1.0
    );
    assert_eq!(
        sample(
            &text,
            "kiarni_changefeed_commit_to_dispatch_seconds_bucket{le=\"+Inf\"}"
        ),
        1.0
    );
    assert_eq!(sample(&text, "kiarni_changefeed_queue_events"), 0.0);
    assert!(!text.contains("orders"));
    assert!(!text.contains("private"));
    assert!(!text.contains("secret"));
    assert!(!text.contains(&key));
    drop(live);
    let deadline = Instant::now() + Duration::from_secs(2);
    let text = loop {
        let text = metrics(addr, &key);
        if sample(&text, "kiarni_changefeed_subscription_handles") == 0.0 {
            break text;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(sample(&text, "kiarni_changefeed_subscriptions_active"), 0.0);
    assert_eq!(
        sample(
            &text,
            "kiarni_changefeed_subscriptions_ended_total{reason=\"cancelled\"}"
        ),
        1.0
    );
    let head = request(addr, "HEAD", "/v1/metrics", Some(&key));
    assert!(head.ends_with("\r\n\r\n"));
    assert!(head.starts_with("HTTP/1.1 200 "));
    // Each completed scrape now changes the HTTP counters and elapsed-time sum.
    // HEAD describes its own observation, not the preceding GET snapshot.
    let length = head
        .lines()
        .find_map(|line| line.strip_prefix("Content-Length: "))
        .unwrap()
        .parse::<usize>()
        .unwrap();
    assert!((1..=4 * 1024 * 1024).contains(&length));
    assert!(head.contains("Content-Type: text/plain; version=0.0.4; charset=utf-8"));
    server.terminate();
}
