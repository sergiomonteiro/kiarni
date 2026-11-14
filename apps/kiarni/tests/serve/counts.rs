use super::*;
fn count(addr: SocketAddr, key: &str, name: &str) -> serde_json::Value {
    let response = request(
        addr,
        "GET",
        &format!("/v1/collections/{name}/count"),
        Some(key),
    );
    assert!(response.starts_with("HTTP/1.1 200 "));
    serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap()
}
#[test]
fn authenticated_count_tracks_crud_noops_conflicts_and_restart() {
    let _scenario = SERVER_SCENARIOS.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    let addr = address();
    let mut server = start(&root, addr);
    wait_until_listening(&mut server, addr);
    let key = token(&root);
    let path = "/v1/collections/orders/count";
    assert!(request(addr, "GET", path, None).starts_with("HTTP/1.1 401 "));
    assert!(request(addr, "GET", path, Some(&key)).starts_with("HTTP/1.1 404 "));
    assert!(create(addr, &key, "orders").starts_with("HTTP/1.1 201 "));
    assert_eq!(
        count(addr, &key, "orders"),
        serde_json::json!({"count":"0","exact":true,"limits":{"documents":10000,"document_bytes":8388608}})
    );
    assert!(request(addr, "POST", path, Some(&key)).starts_with("HTTP/1.1 405 "));
    assert!(
        request(addr, "GET", &format!("{path}?limit=1"), Some(&key)).starts_with("HTTP/1.1 400 ")
    );
    assert!(
        request_body(addr, "GET", path, Some(&key), "Content-Length: 1\r\n", "x")
            .starts_with("HTTP/1.1 400 ")
    );
    assert!(
        request_body(
            addr,
            "GET",
            path,
            Some(&key),
            "Origin: http://evil.invalid\r\n",
            ""
        )
        .starts_with("HTTP/1.1 403 ")
    );
    assert!(
        request_body(addr, "GET", path, Some(&key), "Host: evil.invalid\r\n", "")
            .starts_with("HTTP/1.1 403 ")
    );
    assert!(insert_document(addr, &key, "orders", r#"{"_id":"one"}"#).starts_with("HTTP/1.1 201 "));
    assert!(insert_document(addr, &key, "orders", r#"{"_id":"one"}"#).starts_with("HTTP/1.1 409 "));
    for value in [r#"{"_id":"one"}"#, r#"{"_id":"one","v":2}"#] {
        assert!(
            request_body(
                addr,
                "PUT",
                "/v1/collections/orders/documents/one",
                Some(&key),
                &format!(
                    "Content-Type: application/json\r\nContent-Length: {}\r\n",
                    value.len()
                ),
                value
            )
            .starts_with("HTTP/1.1 200 ")
        );
        assert_eq!(count(addr, &key, "orders")["count"], "1");
    }
    let head = request(addr, "HEAD", path, Some(&key));
    assert!(head.starts_with("HTTP/1.1 200 ") && head.ends_with("\r\n\r\n"));
    assert!(head.contains("Cache-Control: no-store\r\n") && head.contains("X-Request-ID: req_"));
    server.terminate();
    let mut server = start(&root, addr);
    wait_until_listening(&mut server, addr);
    assert_eq!(count(addr, &key, "orders")["count"], "1");
    assert!(
        request(
            addr,
            "DELETE",
            "/v1/collections/orders/documents/one",
            Some(&key)
        )
        .starts_with("HTTP/1.1 204 ")
    );
    assert_eq!(count(addr, &key, "orders")["count"], "0");
    assert!(
        request(addr, "DELETE", "/v1/collections/orders", Some(&key)).starts_with("HTTP/1.1 204 ")
    );
    assert!(request(addr, "GET", path, Some(&key)).starts_with("HTTP/1.1 404 "));
    assert!(create(addr, &key, "orders").starts_with("HTTP/1.1 201 "));
    assert_eq!(count(addr, &key, "orders")["count"], "0");
    server.terminate();
}
