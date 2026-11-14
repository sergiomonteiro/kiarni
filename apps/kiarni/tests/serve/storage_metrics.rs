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
#[test]
fn storage_metrics_are_authenticated_count_noops_correctly_and_reset_after_restart() {
    let _guard = SERVER_SCENARIOS.lock().unwrap_or_else(|e| e.into_inner());
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("data");
    let addr = address();
    let mut server = Server(
        command(&root, addr)
            .args(["--log-level", "off"])
            .spawn()
            .unwrap(),
    );
    wait_until_listening(&mut server, addr);
    let key = token(&root);
    assert!(request(addr, "GET", "/v1/metrics", None).starts_with("HTTP/1.1 401 "));
    let initial = metrics(addr, &key);
    let commits = sample(
        &initial,
        "kiarni_storage_commits_total{outcome=\"confirmed\"}",
    );
    assert_eq!(sample(&initial, "kiarni_storage_file_bytes_available"), 1);
    assert!(sample(&initial, "kiarni_storage_file_logical_bytes") > 0);
    assert!(create(addr, &key, "private_storage_collection").starts_with("HTTP/1.1 201 "));
    assert!(create(addr, &key, "private_storage_collection").starts_with("HTTP/1.1 409 "));
    assert!(
        insert_document(
            addr,
            &key,
            "private_storage_collection",
            r#"{"_id":"private_doc","v":0,"s":"PRIVATE_PAYLOAD"}"#
        )
        .starts_with("HTTP/1.1 201 ")
    );
    let path = "/v1/collections/private_storage_collection/documents/private_doc";
    assert!(request(addr, "GET", path, Some(&key)).starts_with("HTTP/1.1 200 "));
    assert!(
        request(
            addr,
            "GET",
            "/v1/collections/private_storage_collection/documents/absent",
            Some(&key)
        )
        .starts_with("HTTP/1.1 404 ")
    );
    let noop = r#"{"v":0}"#;
    assert!(
        request_body(
            addr,
            "PATCH",
            path,
            Some(&key),
            &format!(
                "Content-Type: application/merge-patch+json\r\nContent-Length: {}\r\n",
                noop.len()
            ),
            noop
        )
        .starts_with("HTTP/1.1 200 ")
    );
    let text = metrics(addr, &key);
    assert_eq!(
        sample(&text, "kiarni_storage_commits_total{outcome=\"confirmed\"}"),
        commits + 2
    );
    assert_eq!(
        sample(
            &text,
            "kiarni_storage_group_receipts_total{commit=\"not_required\"}"
        ),
        1
    );
    let changed = r#"{"v":1}"#;
    assert!(
        request_body(
            addr,
            "PATCH",
            path,
            Some(&key),
            &format!(
                "Content-Type: application/merge-patch+json\r\nContent-Length: {}\r\n",
                changed.len()
            ),
            changed
        )
        .starts_with("HTTP/1.1 200 ")
    );
    assert!(request(addr, "DELETE", path, Some(&key)).starts_with("HTTP/1.1 204 "));
    assert!(
        request(
            addr,
            "DELETE",
            "/v1/collections/private_storage_collection",
            Some(&key)
        )
        .starts_with("HTTP/1.1 204 ")
    );
    let text = metrics(addr, &key);
    assert_eq!(
        sample(&text, "kiarni_storage_commits_total{outcome=\"confirmed\"}"),
        commits + 5
    );
    assert_eq!(
        sample(&text, "kiarni_storage_commits_total{outcome=\"uncertain\"}"),
        0
    );
    assert_eq!(
        sample(
            &text,
            "kiarni_storage_group_items_total{outcome=\"changed\"}"
        ),
        1
    );
    assert_eq!(
        sample(&text, "kiarni_storage_group_items_total{outcome=\"noop\"}"),
        1
    );
    assert_eq!(
        sample(
            &text,
            "kiarni_storage_operations_total{operation=\"document_read\",outcome=\"success\"}"
        ),
        2
    );
    assert_eq!(
        sample(
            &text,
            "kiarni_storage_operations_total{operation=\"collection_create\",outcome=\"error\"}"
        ),
        1
    );
    let count = sample(&text, "kiarni_storage_commit_duration_seconds_count");
    assert_eq!(
        count,
        sample(
            &text,
            "kiarni_storage_commit_duration_seconds_bucket{le=\"+Inf\"}"
        )
    );
    for secret in [
        key.as_str(),
        "private_storage_collection",
        "private_doc",
        "PRIVATE_PAYLOAD",
    ] {
        assert!(!text.contains(secret));
    }
    // The fixed exposition continues to fit the dashboard's existing 64 KiB reader.
    assert!(text.len() <= 65536);
    server.terminate();
    let mut restarted = Server(
        command(&root, addr)
            .args(["--log-level", "off"])
            .spawn()
            .unwrap(),
    );
    wait_until_listening(&mut restarted, addr);
    assert_eq!(
        sample(
            &metrics(addr, &key),
            "kiarni_storage_commits_total{outcome=\"confirmed\"}"
        ),
        0
    );
    restarted.terminate();
}
