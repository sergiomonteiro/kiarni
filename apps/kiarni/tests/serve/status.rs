use super::*;
use std::io::{BufRead, BufReader};

fn status(addr: SocketAddr, key: &str) -> serde_json::Value {
    let response = request(addr, "GET", "/v1/status", Some(key));
    assert!(response.starts_with("HTTP/1.1 200 "));
    serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap()
}
fn until(mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(4);
    while !predicate() {
        assert!(Instant::now() < deadline, "status observation deadline");
        thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn authenticated_status_tracks_admission_streams_uptime_and_restart() {
    let _scenario = SERVER_SCENARIOS.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    let addr = address();
    let mut server = start(&root, addr);
    wait_until_listening(&mut server, addr);
    let key = token(&root);
    assert!(request(addr, "GET", "/v1/status", None).starts_with("HTTP/1.1 401 "));
    assert!(request(addr, "POST", "/v1/status", Some(&key)).starts_with("HTTP/1.1 405 "));
    assert!(request(addr, "GET", "/v1/status?x=1", Some(&key)).starts_with("HTTP/1.1 400 "));
    assert!(
        request_body(
            addr,
            "GET",
            "/v1/status",
            Some(&key),
            "Content-Length: 1\r\n",
            "x"
        )
        .starts_with("HTTP/1.1 400 ")
    );
    assert!(
        request_body(
            addr,
            "GET",
            "/v1/status",
            Some(&key),
            "Origin: http://evil.invalid\r\n",
            ""
        )
        .starts_with("HTTP/1.1 403 ")
    );
    assert!(
        request_body(
            addr,
            "GET",
            "/v1/status",
            Some(&key),
            "Host: evil.invalid\r\n",
            ""
        )
        .starts_with("HTTP/1.1 403 ")
    );
    let head = request(addr, "HEAD", "/v1/status", Some(&key));
    assert!(head.starts_with("HTTP/1.1 200 ") && head.ends_with("\r\n\r\n"));
    assert!(head.contains("Cache-Control: no-store\r\n") && head.contains("X-Request-ID: req_"));
    let initial = status(addr, &key);
    assert_eq!(initial["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(initial.as_object().unwrap().len(), 3);
    assert_eq!(initial["connections"].as_object().unwrap().len(), 4);
    assert!(!initial.to_string().contains(&key));
    assert!(create(addr, &key, "orders").starts_with("HTTP/1.1 201 "));
    let mut stream = TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    write!(stream,"GET /v1/collections/orders/changes HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {key}\r\nAccept: text/event-stream\r\n\r\n").unwrap();
    let mut live = BufReader::new(stream);
    loop {
        let mut line = String::new();
        assert!(live.read_line(&mut line).unwrap() > 0);
        if line.starts_with("data: {\"snapshot\"") {
            break;
        }
    }
    until(|| {
        let value = status(addr, &key);
        value["connections"]["admitted"] == 2 && value["connections"]["streaming"] == 1
    });
    let mut incomplete = TcpStream::connect(addr).unwrap();
    incomplete
        .write_all(b"GET /v1/status HTTP/1.1\r\n")
        .unwrap();
    until(|| status(addr, &key)["connections"]["admitted"] == 3);
    drop(incomplete);
    until(|| status(addr, &key)["connections"]["admitted"] == 2);
    drop(live);
    until(|| {
        let value = status(addr, &key);
        value["connections"]["admitted"] == 1 && value["connections"]["streaming"] == 0
    });
    until(|| {
        status(addr, &key)["uptime_seconds"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap()
            >= 2
    });
    server.terminate();
    let mut restarted = start(&root, addr);
    wait_until_listening(&mut restarted, addr);
    assert!(
        status(addr, &key)["uptime_seconds"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap()
            < 2
    );
    restarted.terminate();
}
