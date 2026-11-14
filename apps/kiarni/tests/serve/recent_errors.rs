use super::*;
const PATH: &str = "/v1/errors/recent";
fn recent(addr: SocketAddr, key: &str) -> serde_json::Value {
    let wire = request(addr, "GET", PATH, Some(key));
    assert!(wire.starts_with("HTTP/1.1 200 "));
    assert!(wire.contains("Cache-Control: no-store\r\n"));
    serde_json::from_str(wire.split_once("\r\n\r\n").unwrap().1).unwrap()
}
#[test]
fn bounded_authenticated_history_is_private_works_with_logs_off_and_resets() {
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
    assert!(request(addr, "GET", PATH, None).starts_with("HTTP/1.1 401 "));
    let wrong_method = request(addr, "POST", PATH, Some(&key));
    assert!(wrong_method.starts_with("HTTP/1.1 405 ") && wrong_method.contains("Allow: GET, HEAD"));
    assert!(request(addr, "GET", "/v1/errors/recent?x=1", Some(&key)).starts_with("HTTP/1.1 400 "));
    for header in ["Origin: http://evil.invalid\r\n", "Host: evil.invalid\r\n"] {
        assert!(
            request_body(addr, "GET", PATH, Some(&key), header, "").starts_with("HTTP/1.1 403 ")
        );
    }
    assert!(
        request_body(addr, "GET", PATH, Some(&key), "Content-Length: 1\r\n", "x")
            .starts_with("HTTP/1.1 400 ")
    );
    let head = request(addr, "HEAD", PATH, Some(&key));
    assert!(head.starts_with("HTTP/1.1 200 ") && head.ends_with("\r\n\r\n"));
    let mut ids = Vec::new();
    for _ in 0..12 {
        let wire = request(
            addr,
            "GET",
            "/v1/secret_collection/documents/secret_document",
            Some(&key),
        );
        assert!(wire.starts_with("HTTP/1.1 404 "));
        let value: serde_json::Value =
            serde_json::from_str(wire.split_once("\r\n\r\n").unwrap().1).unwrap();
        ids.push(value["request_id"].as_str().unwrap().to_owned());
    }
    let value = recent(addr, &key);
    assert_eq!(value["capacity"], 8);
    assert_eq!(value["dropped_total"], "0");
    let entries = value["errors"].as_array().unwrap();
    assert_eq!(entries.len(), 8);
    for (entry, id) in entries.iter().zip(ids[4..].iter().rev()) {
        assert_eq!(entry["request_id"], *id);
        assert_eq!(entry["status"], 404);
        assert_eq!(entry.as_object().unwrap().len(), 9);
    }
    for secret in [
        &key,
        "secret_collection",
        "secret_document",
        "Authorization",
        "evil.invalid",
    ] {
        assert!(!value.to_string().contains(secret));
    }
    assert_eq!(
        recent(addr, &key),
        value,
        "successful scrapes do not create errors"
    );
    let old_ids = ids;
    server.terminate();
    let mut restarted = Server(
        command(&root, addr)
            .args(["--log-level", "off"])
            .spawn()
            .unwrap(),
    );
    wait_until_listening(&mut restarted, addr);
    for entry in recent(addr, &key)["errors"].as_array().unwrap() {
        assert!(!old_ids.iter().any(|id| entry["request_id"] == *id));
    }
    restarted.terminate();
}
