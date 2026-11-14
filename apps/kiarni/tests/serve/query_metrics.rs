use super::*;
fn metrics(addr: SocketAddr, key: &str) -> String {
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
fn query_at(addr: SocketAddr, key: Option<&str>, collection: &str, body: &str) -> String {
    request_body(
        addr,
        "POST",
        &format!("/v1/collections/{collection}/query"),
        key,
        &format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            body.len()
        ),
        body,
    )
}
fn outcome(text: &str, name: &str) -> u64 {
    sample(
        text,
        &format!("kiarni_query_completed_total{{outcome=\"{name}\"}}"),
    )
}
#[test]
fn query_metrics_separate_validation_execution_access_and_result_bytes() {
    let _guard = SERVER_SCENARIOS.lock().unwrap_or_else(|e| e.into_inner());
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
    assert!(create(addr, &key, "private_query_collection").starts_with("HTTP/1.1 201 "));
    assert!(
        insert_document(
            addr,
            &key,
            "private_query_collection",
            r#"{"_id":"private_doc","v":[],"payload":"PRIVATE_PAYLOAD"}"#
        )
        .starts_with("HTTP/1.1 201 ")
    );
    let good = r#"{"from":"private_query_collection","id":"private_doc","limit":1}"#;
    assert!(query_at(addr, None, "private_query_collection", good).starts_with("HTTP/1.1 401 "));
    assert!(
        query_at(addr, Some(&key), "private_query_collection", "bad").starts_with("HTTP/1.1 400 ")
    );
    let response = query_at(addr, Some(&key), "private_query_collection", good);
    assert!(response.starts_with("HTTP/1.1 200 "));
    let first_bytes = response.split_once("\r\n\r\n").unwrap().1.len();
    let empty = query_at(
        addr,
        Some(&key),
        "private_query_collection",
        r#"{"from":"private_query_collection","id":"absent","limit":1}"#,
    );
    assert!(empty.starts_with("HTTP/1.1 200 "));
    assert!(
        query_at(addr, Some(&key), "absent", r#"{"from":"absent","limit":1}"#)
            .starts_with("HTTP/1.1 404 ")
    );
    assert!(query_at(addr,Some(&key),"private_query_collection",r#"{"from":"private_query_collection","limit":1,"sort":[{"field":"v","direction":"asc"}]}"#).starts_with("HTTP/1.1 422 "));
    let text = metrics(addr, &key);
    assert_eq!(outcome(&text, "success"), 2);
    assert_eq!(outcome(&text, "rejected"), 1);
    assert_eq!(outcome(&text, "not_found"), 1);
    assert_eq!(outcome(&text, "evaluation_error"), 1);
    assert_eq!(
        sample(
            &text,
            "kiarni_query_duration_seconds_count{stage=\"total\"}"
        ),
        5
    );
    assert_eq!(
        sample(&text, "kiarni_query_duration_seconds_count{stage=\"wait\"}"),
        4
    );
    assert_eq!(
        sample(
            &text,
            "kiarni_query_duration_seconds_count{stage=\"execution\"}"
        ),
        4
    );
    assert_eq!(
        sample(&text, "kiarni_query_execution_total{access=\"id\"}"),
        2
    );
    assert_eq!(
        sample(&text, "kiarni_query_execution_total{access=\"scan\"}"),
        2
    );
    assert_eq!(sample(&text, "kiarni_query_documents_returned_total"), 1);
    assert_eq!(
        sample(&text, "kiarni_query_result_bytes_total"),
        (first_bytes + empty.split_once("\r\n\r\n").unwrap().1.len()) as u64
    );
    for stage in ["total", "wait", "execution"] {
        let mut last = 0;
        for bound in ["0.001", "0.01", "0.1", "1", "5", "30", "+Inf"] {
            let value = sample(
                &text,
                &format!(
                    "kiarni_query_duration_seconds_bucket{{stage=\"{stage}\",le=\"{bound}\"}}"
                ),
            );
            assert!(value >= last);
            last = value;
        }
        assert_eq!(
            last,
            sample(
                &text,
                &format!("kiarni_query_duration_seconds_count{{stage=\"{stage}\"}}")
            )
        );
    }
    for secret in [
        key.as_str(),
        "private_query_collection",
        "private_doc",
        "PRIVATE_PAYLOAD",
    ] {
        assert!(!text.contains(secret));
    }
    server.terminate();
    let mut restarted = Server(
        command(&root, addr)
            .args(["--log-level", "off"])
            .spawn()
            .unwrap(),
    );
    wait_until_listening(&mut restarted, addr);
    assert_eq!(outcome(&metrics(addr, &key), "success"), 0);
    restarted.terminate();
}
#[test]
fn real_scan_and_result_limits_have_distinct_query_outcomes() {
    let _guard = SERVER_SCENARIOS.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
    let store = kiarni_storage::Store::create(&root.join("store")).unwrap();
    let result = store.create_collection("result_limit").unwrap();
    for id in ["a", "b"] {
        store
            .insert_fixture(
                &result,
                format!(r#"{{"_id":"{id}","s":"{}"}}"#, "x".repeat(600000)).as_bytes(),
            )
            .unwrap();
    }
    let scan = store.create_collection("scan_limit").unwrap();
    for id in 0..9 {
        store
            .insert_fixture(
                &scan,
                format!(r#"{{"_id":"doc_{id}","s":"{}"}}"#, "x".repeat(1000000)).as_bytes(),
            )
            .unwrap();
    }
    drop(store);
    phase(&root, "initializing_store");
    let addr = address();
    let mut server = Server(
        command(&root, addr)
            .args(["--max-result-bytes", "1049600", "--log-level", "off"])
            .spawn()
            .unwrap(),
    );
    wait_until_listening(&mut server, addr);
    let key = token(&root);
    let result = query_at(
        addr,
        Some(&key),
        "result_limit",
        r#"{"from":"result_limit","limit":2}"#,
    );
    assert!(result.contains("\"code\":\"result_too_large\""));
    let scan = query_at(
        addr,
        Some(&key),
        "scan_limit",
        r#"{"from":"scan_limit","limit":1}"#,
    );
    assert!(scan.contains("\"code\":\"query_too_complex\""));
    let text = metrics(addr, &key);
    assert_eq!(outcome(&text, "result_limit"), 1);
    assert_eq!(outcome(&text, "scan_limit"), 1);
    assert_eq!(outcome(&text, "rejected"), 0);
    assert_eq!(outcome(&text, "success"), 0);
    assert_eq!(sample(&text, "kiarni_query_documents_returned_total"), 0);
    assert_eq!(sample(&text, "kiarni_query_result_bytes_total"), 0);
    server.terminate();
}
