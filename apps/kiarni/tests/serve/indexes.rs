use super::*;

fn index(root: &Path, addr: SocketAddr, operation: &str, collection: bool) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_kiarni"));
    command
        .env_clear()
        .args(["index", operation])
        .arg("--data-dir")
        .arg(root)
        .args(["--bind", &addr.to_string()]);
    if collection {
        command.args(["--collection", "items"]);
    }
    if operation == "create" {
        command.args(["--field", "status"]);
    }
    command.output().unwrap()
}
fn credentials(root: &Path) -> Vec<Vec<u8>> {
    let mut paths = vec![root.join("bootstrap.json")];
    for directory in [root.join("auth"), root.with_file_name("data.credentials")] {
        paths.extend(
            fs::read_dir(directory)
                .unwrap()
                .map(|entry| entry.unwrap().path()),
        );
    }
    paths.sort();
    paths
        .into_iter()
        .map(|path| fs::read(path).unwrap())
        .collect()
}
fn query(addr: SocketAddr, key: &str) -> String {
    let body = r#"{"from":"items","limit":10,"filter":{"op":"eq","field":"status","value":"wanted"},"project":["_id"]}"#;
    request_body(
        addr,
        "POST",
        "/v1/collections/items/query",
        Some(key),
        &format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            body.len()
        ),
        body,
    )
}

#[test]
fn offline_index_commands_enable_http_refuse_live_server_and_preserve_auth() {
    let _scenario = SERVER_SCENARIOS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("data");
    let addr = address();
    let mut server = start(&root, addr);
    wait_until_listening(&mut server, addr);
    let key = token(&root);
    assert!(create(addr, &key, "items").starts_with("HTTP/1.1 201 "));
    for n in 0..16 {
        assert!(
            insert_document(
                addr,
                &key,
                "items",
                &format!(
                    r#"{{"_id":"d{n:02}","status":"{}"}}"#,
                    if n == 0 { "wanted" } else { "bulk" }
                )
            )
            .starts_with("HTTP/1.1 201 ")
        );
    }
    let before_auth = credentials(&root);
    for operation in ["create", "rebuild", "drop", "audit"] {
        let output = index(&root, addr, operation, operation != "audit");
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert_eq!(output.stderr, b"kiarni: data_directory_locked\n");
    }
    assert!(query(addr, &key).contains("d00"));
    server.terminate();
    // Offline commands validate bind but never bind it, even while occupied.
    let occupied = TcpListener::bind(addr).unwrap();
    let missing = index(&root, addr, "rebuild", true);
    assert_eq!(missing.stderr, b"kiarni: index_not_found\n");
    assert!(index(&root, addr, "create", true).status.success());
    let duplicate = index(&root, addr, "create", true);
    assert_eq!(duplicate.stderr, b"kiarni: index_already_exists\n");
    assert!(duplicate.stdout.is_empty());
    let audit = index(&root, addr, "audit", false);
    assert!(audit.status.success());
    assert!(
        String::from_utf8(audit.stdout)
            .unwrap()
            .contains("16 documentos")
    );
    assert!(audit.stderr.is_empty());
    assert!(credentials(&root) == before_auth);
    drop(occupied);
    let mut server = start(&root, addr);
    wait_until_listening(&mut server, addr);
    assert!(query(addr, &key).contains("d00"));
    assert!(
        request(addr, "GET", "/v1/metrics", Some(&key))
            .contains("kiarni_query_execution_total{access=\"secondary\"} 1\n")
    );
    let patch = r#"{"status":"bulk"}"#;
    assert!(
        request_body(
            addr,
            "PATCH",
            "/v1/collections/items/documents/d00",
            Some(&key),
            &format!(
                "Content-Type: application/merge-patch+json\r\nContent-Length: {}\r\n",
                patch.len()
            ),
            patch
        )
        .starts_with("HTTP/1.1 200 ")
    );
    assert!(
        insert_document(addr, &key, "items", r#"{"_id":"d16","status":"wanted"}"#)
            .starts_with("HTTP/1.1 201 ")
    );
    let result = query(addr, &key);
    assert!(result.contains("d16") && !result.contains("d00"));
    server.terminate();
    assert!(index(&root, addr, "rebuild", true).status.success());
    {
        let store = kiarni_storage::Store::open(&root.join("store")).unwrap();
        assert_eq!(
            store
                .index_field(&store.collection("items").unwrap())
                .unwrap()
                .as_deref(),
            Some("status")
        );
        assert_eq!(store.audit().unwrap().documents, 17);
    }
    assert!(index(&root, addr, "drop", true).status.success());
    assert!(index(&root, addr, "audit", false).status.success());
    let missing = index(&root, addr, "drop", true);
    assert_eq!(missing.stderr, b"kiarni: index_not_found\n");
    assert!(missing.stdout.is_empty());
    assert!(credentials(&root) == before_auth);
    let mut server = start(&root, addr);
    wait_until_listening(&mut server, addr);
    assert!(token(&root) == key);
    let response = query(addr, &key);
    assert!(response.contains("d16") && !response.contains("d00"));
    let metrics = request(addr, "GET", "/v1/metrics", Some(&key));
    assert!(metrics.contains("kiarni_query_execution_total{access=\"scan\"} 1\n"));
    assert!(metrics.contains("kiarni_query_execution_total{access=\"secondary\"} 0\n"));
    server.terminate();
}
