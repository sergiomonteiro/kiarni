use super::*;

fn request_id(response: &str) -> &str {
    response
        .lines()
        .find_map(|line| line.strip_prefix("X-Request-ID: "))
        .unwrap()
}
#[test]
fn request_logs_correlate_success_errors_and_stream_close_without_sensitive_data() {
    let _scenario = SERVER_SCENARIOS.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    let addr = address();
    let path = dir.path().join("requests.jsonl");
    let output = fs::File::create(&path).unwrap();
    let mut server = Server(command(&root, addr).stderr(output).spawn().unwrap());
    wait_until_listening(&mut server, addr);
    let key = token(&root);
    let live = request(addr, "GET", "/v1/health/live", Some(&key));
    let denied = request(addr, "GET", "/private_path?secret_query=DO_NOT_LOG", None);
    let created = create(addr, &key, "sensitive_collection");
    let inserted = insert_document(
        addr,
        &key,
        "sensitive_collection",
        r#"{"_id":"sensitive_id","payload":"PRIVATE_DOCUMENT"}"#,
    );
    let bad = request_body(
        addr,
        "POST",
        "/v1/collections/sensitive_collection/query",
        Some(&key),
        "Content-Type: application/json\r\nContent-Length: 3\r\n",
        "bad",
    );
    assert!(bad.starts_with("HTTP/1.1 400 "));
    let incoming = request_body(
        addr,
        "GET",
        "/v1/health/live",
        Some(&key),
        "X-Request-ID: malicious_request_id\r\n",
        "",
    );
    // Start a real stream, collect its server ID, then disconnect it.
    let mut stream = TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    write!(stream,"GET /v1/collections/sensitive_collection/changes HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {key}\r\nAccept: text/event-stream\r\n\r\n").unwrap();
    use std::io::{BufRead, BufReader};
    let mut stream = BufReader::new(stream);
    let mut headers = String::new();
    loop {
        let mut line = String::new();
        assert!(stream.read_line(&mut line).unwrap() > 0);
        headers.push_str(&line);
        if line == "\r\n" {
            break;
        }
    }
    assert!(headers.starts_with("HTTP/1.1 200 "));
    let stream_id = request_id(&headers).to_owned();
    drop(stream);
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let text = fs::read_to_string(&path).unwrap();
        if text
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .any(|line| line["request_id"] == stream_id)
        {
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    server.terminate();
    let text = fs::read_to_string(&path).unwrap();
    assert!(!text.contains(&key));
    for secret in [
        "private_path",
        "DO_NOT_LOG",
        "secret_query",
        "sensitive_collection",
        "sensitive_id",
        "PRIVATE_DOCUMENT",
        "malicious_request_id",
    ] {
        assert!(!text.contains(secret));
    }
    let records: Vec<serde_json::Value> = text
        .lines()
        .filter(|line| line.starts_with('{'))
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    for (response, status, operation, error) in [
        (&live, 200, Some("Liveness"), None),
        (&denied, 401, None, Some("unauthorized")),
        (&created, 201, Some("CreateCollection"), None),
        (&inserted, 201, Some("InsertDocument"), None),
        (&bad, 400, Some("Query"), Some("invalid_json")),
        (&incoming, 200, Some("Liveness"), None),
    ] {
        let id = request_id(response);
        let matches: Vec<_> = records
            .iter()
            .filter(|record| record["request_id"] == id)
            .collect();
        assert_eq!(matches.len(), 1);
        let record = matches[0];
        assert_eq!(record["event"], "request_complete");
        assert_eq!(record["status"], status);
        assert_eq!(record["operation"].as_str(), operation);
        assert_eq!(record["error_code"].as_str(), error);
        assert_eq!(record["transport"], "written");
        record["duration_us"]
            .as_str()
            .unwrap()
            .parse::<u128>()
            .unwrap();
        assert!(record["timestamp_ms"].as_str().is_some());
    }
    let stream = records
        .iter()
        .find(|record| record["request_id"] == stream_id)
        .unwrap();
    assert_eq!(stream["operation"], "Changes");
    assert_eq!(stream["transport"], "stream_closed");
}

#[test]
fn disabled_logging_is_silent_and_an_unread_stderr_does_not_stall_requests_or_shutdown() {
    let _scenario = SERVER_SCENARIOS.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    let addr = address();
    let mut server = Server(
        command(&root, addr)
            .args(["--log-level", "off"])
            .spawn()
            .unwrap(),
    );
    wait_until_listening(&mut server, addr);
    let key = token(&root);
    assert!(request(addr, "GET", "/v1/health/live", Some(&key)).starts_with("HTTP/1.1 200 "));
    server.terminate();
    let mut stderr = String::new();
    server
        .0
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    // Optional profiling is a separate diagnostic, not a request log.
    assert!(!stderr.contains("request_complete"));
    let mut server = start(&root, addr);
    wait_until_listening(&mut server, addr);
    for _ in 0..800 {
        assert!(request(addr, "GET", "/v1/health/live", Some(&key)).starts_with("HTTP/1.1 200 "));
    }
    let metrics = request(addr, "GET", "/v1/metrics", Some(&key));
    assert!(metrics.starts_with("HTTP/1.1 200 "));
    let total = metrics
        .lines()
        .find_map(|line| line.strip_prefix("kiarni_http_requests_total{status_class=\"2xx\"} "))
        .unwrap()
        .parse::<u64>()
        .unwrap();
    assert!(
        total >= 800,
        "metrics must not depend on the stalled log sink"
    );
    server.terminate();
}
