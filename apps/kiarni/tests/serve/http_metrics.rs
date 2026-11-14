use super::*;
use std::io::{BufRead, BufReader};
fn scrape(addr: SocketAddr, key: &str) -> String {
    let response = request(addr, "GET", "/v1/metrics", Some(key));
    assert!(response.starts_with("HTTP/1.1 200 "));
    response.split_once("\r\n\r\n").unwrap().1.to_owned()
}
fn sample(text: &str, name: &str) -> u64 {
    text.lines()
        .find_map(|line| line.strip_prefix(&format!("{name} ")))
        .unwrap()
        .parse()
        .unwrap()
}
#[test]
fn http_completion_metrics_work_with_logs_off_and_distinguish_live_streams() {
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
    assert!(create(addr, &key, "private_metrics_collection").starts_with("HTTP/1.1 201 "));
    // Wait for the startup probe EOF and completed headers to settle. Each scrape
    // includes the preceding scrape, but never itself at observation time.
    let baseline = scrape(addr, &key);
    let before = sample(
        &baseline,
        "kiarni_http_requests_total{status_class=\"2xx\"}",
    );
    assert!(request(addr, "GET", "/v1/health/live", Some(&key)).starts_with("HTTP/1.1 200 "));
    assert!(
        request(addr, "GET", "/PRIVATE_PATH?token=PRIVATE_SECRET", None)
            .starts_with("HTTP/1.1 401 ")
    );
    let after = scrape(addr, &key);
    assert_eq!(
        sample(&after, "kiarni_http_requests_total{status_class=\"2xx\"}"),
        before + 2
    );
    assert!(sample(&after, "kiarni_http_requests_total{status_class=\"4xx\"}") >= 1);
    assert_eq!(
        sample(
            &after,
            "kiarni_http_request_duration_seconds_count{kind=\"stream\"}"
        ),
        0
    );
    let mut stream = TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    write!(stream,"GET /v1/collections/private_metrics_collection/changes HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {key}\r\nAccept: text/event-stream\r\n\r\n").unwrap();
    let mut stream = BufReader::new(stream);
    loop {
        let mut line = String::new();
        assert!(stream.read_line(&mut line).unwrap() > 0);
        if line.starts_with("data: {\"snapshot\"") {
            break;
        }
    }
    let active = scrape(addr, &key);
    assert_eq!(
        sample(
            &active,
            "kiarni_http_request_duration_seconds_count{kind=\"stream\"}"
        ),
        0
    );
    assert_eq!(sample(&active, "kiarni_changefeed_subscriptions_active"), 1);
    drop(stream);
    let deadline = Instant::now() + Duration::from_secs(3);
    let final_metrics = loop {
        let text = scrape(addr, &key);
        if sample(
            &text,
            "kiarni_http_request_duration_seconds_count{kind=\"stream\"}",
        ) == 1
        {
            break text;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(
        sample(
            &final_metrics,
            "kiarni_http_transport_total{outcome=\"stream_closed\"}"
        ),
        1
    );
    for kind in ["response", "stream"] {
        let mut previous = 0;
        for bound in ["0.001", "0.01", "0.1", "1", "5", "30", "+Inf"] {
            let count = sample(
                &final_metrics,
                &format!(
                    "kiarni_http_request_duration_seconds_bucket{{kind=\"{kind}\",le=\"{bound}\"}}"
                ),
            );
            assert!(count >= previous);
            previous = count;
        }
        assert_eq!(
            previous,
            sample(
                &final_metrics,
                &format!("kiarni_http_request_duration_seconds_count{{kind=\"{kind}\"}}")
            )
        );
    }
    for secret in [
        key.as_str(),
        "private_metrics_collection",
        "PRIVATE_PATH",
        "PRIVATE_SECRET",
    ] {
        assert!(!final_metrics.contains(secret));
    }
    server.terminate();
    let mut restarted = Server(
        command(&root, addr)
            .args(["--log-level", "off"])
            .spawn()
            .unwrap(),
    );
    wait_until_listening(&mut restarted, addr);
    let reset = scrape(addr, &key);
    assert_eq!(
        sample(
            &reset,
            "kiarni_http_request_duration_seconds_count{kind=\"stream\"}"
        ),
        0
    );
    assert_eq!(
        sample(&reset, "kiarni_http_requests_total{status_class=\"2xx\"}"),
        0
    );
    restarted.terminate();
}
