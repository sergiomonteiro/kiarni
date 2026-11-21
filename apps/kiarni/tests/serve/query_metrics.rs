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
                format!(
                    r#"{{"_id":"doc_{id}","status":"{}","s":"{}"}}"#,
                    if id == 0 { "wanted" } else { "bulk" },
                    "x".repeat(1000000)
                )
                .as_bytes(),
            )
            .unwrap();
        if id == 7 {
            store.create_index(&scan, "status").unwrap();
        }
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
        r#"{"from":"scan_limit","limit":1,"filter":{"op":"eq","field":"status","value":"wanted"}}"#,
    );
    assert!(scan.contains("\"code\":\"query_too_complex\""));
    let text = metrics(addr, &key);
    assert_eq!(outcome(&text, "result_limit"), 1);
    assert_eq!(outcome(&text, "scan_limit"), 1);
    assert_eq!(outcome(&text, "rejected"), 0);
    assert_eq!(outcome(&text, "success"), 0);
    assert_eq!(sample(&text, "kiarni_query_documents_returned_total"), 0);
    assert_eq!(sample(&text, "kiarni_query_result_bytes_total"), 0);
    assert_eq!(
        sample(&text, "kiarni_query_execution_total{access=\"secondary\"}"),
        0
    );
    let id = query_at(
        addr,
        Some(&key),
        "scan_limit",
        r#"{"from":"scan_limit","id":"doc_0","limit":1,"project":["_id"]}"#,
    );
    assert!(id.starts_with("HTTP/1.1 200 "));
    let text = metrics(addr, &key);
    assert_eq!(outcome(&text, "scan_limit"), 1);
    assert_eq!(outcome(&text, "success"), 1);
    assert_eq!(
        sample(&text, "kiarni_query_execution_total{access=\"id\"}"),
        1
    );
    server.terminate();
}

#[test]
fn secondary_http_matches_scan_and_reports_the_selected_path_even_on_errors() {
    let _guard = SERVER_SCENARIOS.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
    let store = kiarni_storage::Store::create(&root.join("store")).unwrap();
    let collection = store.create_collection("private_indexed").unwrap();
    for n in 0..16 {
        let status = if n < 2 { "wanted" } else { "bulk" };
        let number = if n == 0 {
            "9007199254740993"
        } else {
            "9007199254740992"
        };
        let json = format!(
            r#"{{"_id":"d{n:02}","status":"{status}","n":{number},"bad_sort":[],"payload":"{}"}}"#,
            if n < 2 {
                "x".repeat(600_000)
            } else {
                String::new()
            }
        );
        store.insert_fixture(&collection, json.as_bytes()).unwrap();
    }
    drop(store);
    phase(&root, "initializing_store");
    let queries = [
        r#"{"from":"private_indexed","limit":2,"filter":{"op":"eq","field":"status","value":"wanted"},"sort":[{"field":"n","direction":"desc"}],"project":["_id","n"]}"#,
        r#"{"from":"private_indexed","limit":100,"filter":{"op":"eq","field":"status","value":"bulk"},"project":["_id"]}"#,
        r#"{"from":"private_indexed","limit":10,"filter":{"op":"eq","field":"status","value":"absent"}}"#,
        r#"{"from":"private_indexed","limit":10,"filter":{"op":"eq","field":"status","value":true}}"#,
        r#"{"from":"private_indexed","id":"d00","limit":1,"filter":{"op":"eq","field":"status","value":"wanted"},"project":["_id","n"]}"#,
        r#"{"from":"private_indexed","limit":2,"filter":{"op":"eq","field":"status","value":"want\u0065d"},"project":["_id","n"]}"#,
        r#"{"from":"private_indexed","limit":2,"filter":{"op":"eq","field":"status","value":"wanted"},"sort":[{"field":"bad_sort","direction":"asc"}]}"#,
        r#"{"from":"private_indexed","limit":2,"filter":{"op":"eq","field":"status","value":"wanted"}}"#,
    ];
    let normalized = |response: &str| {
        let (headers, body) = response.split_once("\r\n\r\n").unwrap();
        let status = headers.lines().next().unwrap().to_owned();
        if status.starts_with("HTTP/1.1 200 ") {
            return (status, body.to_owned());
        }
        let mut error: serde_json::Value = serde_json::from_str(body).unwrap();
        error.as_object_mut().unwrap().remove("request_id");
        (status, serde_json::to_string(&error).unwrap())
    };
    let addr = address();
    let start_fixture = || {
        Server(
            command(&root, addr)
                .args(["--max-result-bytes", "1049600", "--log-level", "off"])
                .spawn()
                .unwrap(),
        )
    };
    let mut server = start_fixture();
    wait_until_listening(&mut server, addr);
    let key = token(&root);
    let baseline: Vec<_> = queries
        .iter()
        .map(|query| normalized(&query_at(addr, Some(&key), "private_indexed", query)))
        .collect();
    assert!(
        baseline[0].1.find("9007199254740993").unwrap()
            < baseline[0].1.find("9007199254740992").unwrap()
    );
    assert!(baseline[6].1.contains("unsupported_sort_value"));
    assert!(baseline[7].1.contains("result_too_large"));
    assert_eq!(
        sample(
            &metrics(addr, &key),
            "kiarni_query_execution_total{access=\"secondary\"}"
        ),
        0
    );
    server.terminate();
    {
        let store = kiarni_storage::Store::open(&root.join("store")).unwrap();
        store
            .create_index(&store.collection("private_indexed").unwrap(), "status")
            .unwrap();
    }
    let mut server = start_fixture();
    wait_until_listening(&mut server, addr);
    for (query, expected) in queries.iter().zip(&baseline) {
        assert_eq!(
            &normalized(&query_at(addr, Some(&key), "private_indexed", query)),
            expected
        );
    }
    let text = metrics(addr, &key);
    assert_eq!(
        sample(&text, "kiarni_query_execution_total{access=\"secondary\"}"),
        5
    );
    assert_eq!(
        sample(&text, "kiarni_query_execution_total{access=\"scan\"}"),
        2
    );
    assert_eq!(
        sample(&text, "kiarni_query_execution_total{access=\"id\"}"),
        1
    );
    assert_eq!(outcome(&text, "success"), 6);
    assert_eq!(outcome(&text, "evaluation_error"), 1);
    assert_eq!(outcome(&text, "result_limit"), 1);
    // Evaluation/result failures happen after a successful storage load.
    assert_eq!(
        sample(
            &text,
            "kiarni_storage_operations_total{operation=\"query_read\",outcome=\"success\"}"
        ),
        8
    );
    assert_eq!(
        sample(
            &text,
            "kiarni_storage_operations_total{operation=\"query_read\",outcome=\"error\"}"
        ),
        0
    );
    for private in [&key, "private_indexed", "wanted", "d00", "payload"] {
        assert!(!text.contains(private));
    }
    server.terminate();
    let mut restarted = start_fixture();
    wait_until_listening(&mut restarted, addr);
    assert_eq!(
        sample(
            &metrics(addr, &key),
            "kiarni_query_execution_total{access=\"secondary\"}"
        ),
        0
    );
    assert_eq!(
        normalized(&query_at(addr, Some(&key), "private_indexed", queries[0])),
        baseline[0]
    );
    assert_eq!(
        sample(
            &metrics(addr, &key),
            "kiarni_query_execution_total{access=\"secondary\"}"
        ),
        1
    );
    restarted.terminate();
}
