use super::*;
use std::io::{BufRead, BufReader};
fn delivery(root: &Path) -> PathBuf {
    let mut name = root.file_name().unwrap().to_os_string();
    name.push(".credentials");
    root.with_file_name(name)
}
fn keys(root: &Path) -> Vec<String> {
    fs::read_dir(delivery(root))
        .unwrap()
        .map(|e| fs::read_to_string(e.unwrap().path()).unwrap())
        .collect()
}
#[test]
fn restored_server_uses_new_credentials_preserves_documents_and_resets_future_only_feeds() {
    let _scenario = SERVER_SCENARIOS.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let original = dir.path().join("data");
    let original_addr = address();
    let mut source = start(&original, original_addr);
    wait_until_listening(&mut source, original_addr);
    let revoked = token(&original);
    assert!(create(original_addr, &revoked, "orders").starts_with("HTTP/1.1 201 "));
    assert!(create(original_addr, &revoked, "removed").starts_with("HTTP/1.1 201 "));
    assert!(
        request(
            original_addr,
            "DELETE",
            "/v1/collections/removed",
            Some(&revoked)
        )
        .starts_with("HTTP/1.1 204 ")
    );
    assert!(create(original_addr, &revoked, "empty").starts_with("HTTP/1.1 201 "));
    assert!(
        insert_document(
            original_addr,
            &revoked,
            "orders",
            r#"{"_id":"doc","n":9007199254740993,"p":1.234567890123456789}"#
        )
        .starts_with("HTTP/1.1 201 ")
    );
    assert!(
        insert_document(original_addr, &revoked, "orders", r#"{"_id":"second"}"#)
            .starts_with("HTTP/1.1 201 ")
    );
    source.terminate();
    assert!(auth_command(&original, "rotate").status.success());
    let active = keys(&original)
        .into_iter()
        .find(|key| key != &revoked)
        .unwrap();
    let artifact = dir.path().join("backup");
    let output = Command::new(env!("CARGO_BIN_EXE_kiarni"))
        .env_clear()
        .arg("backup")
        .arg("--data-dir")
        .arg(&original)
        .arg("--output")
        .arg(&artifact)
        .output()
        .unwrap();
    assert!(output.status.success());
    let backup_bytes = fs::read(artifact.join("store/data.redb")).unwrap();
    let mut source = start(&original, original_addr);
    wait_until_listening(&mut source, original_addr);
    let restored = dir.path().join("restored");
    let output = Command::new(env!("CARGO_BIN_EXE_kiarni"))
        .env_clear()
        .arg("restore")
        .arg("--from")
        .arg(&artifact)
        .arg("--data-dir")
        .arg(&restored)
        .args(["--bind", &original_addr.to_string()])
        .output()
        .unwrap();
    assert!(output.status.success(), "restore CLI failed");
    assert!(output.stderr.is_empty());
    let message = String::from_utf8(output.stdout).unwrap();
    assert!(message.contains("Restauração concluída"));
    assert!(!message.contains(&revoked) && !message.contains(&active));
    assert!(!restored.join("auth").exists());
    assert_eq!(fs::read_dir(delivery(&restored)).unwrap().count(), 0);
    assert!(fs::read(artifact.join("store/data.redb")).unwrap() == backup_bytes);
    let restored_addr = address();
    let mut server = start(&restored, restored_addr);
    wait_until_listening(&mut server, restored_addr);
    let values = keys(&restored);
    assert_eq!(values.len(), 1);
    let new_key = &values[0];
    assert!(new_key != &revoked && new_key != &active);
    for old in [&revoked, &active] {
        assert!(
            request(restored_addr, "GET", "/v1/health/ready", Some(old))
                .starts_with("HTTP/1.1 401 ")
        );
    }
    let response = request(
        restored_addr,
        "GET",
        "/v1/collections/orders/documents/doc",
        Some(new_key),
    );
    assert!(response.starts_with("HTTP/1.1 200 "));
    assert!(response.contains("9007199254740993") && response.contains("1.234567890123456789"));
    assert!(
        request(
            restored_addr,
            "GET",
            "/v1/collections/empty/count",
            Some(new_key)
        )
        .contains("\"count\":\"0\"")
    );
    assert!(
        request(original_addr, "GET", "/v1/health/ready", Some(&active))
            .starts_with("HTTP/1.1 200 ")
    );
    let mut stream = TcpStream::connect(restored_addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    write!(stream,"GET /v1/collections/orders/changes HTTP/1.1\r\nHost: {restored_addr}\r\nAuthorization: Bearer {new_key}\r\nAccept: text/event-stream\r\n\r\n").unwrap();
    let mut stream = BufReader::new(stream);
    loop {
        let mut line = String::new();
        assert!(stream.read_line(&mut line).unwrap() > 0);
        if line.contains("\"replay\":false") {
            break;
        }
    }
    assert!(
        insert_document(
            restored_addr,
            new_key,
            "orders",
            r#"{"_id":"after_restore"}"#
        )
        .starts_with("HTTP/1.1 201 ")
    );
    loop {
        let mut line = String::new();
        assert!(stream.read_line(&mut line).unwrap() > 0);
        if line.starts_with("data:") {
            assert!(line.contains("\"sequence\":\"1\"") && line.contains("after_restore"));
            break;
        }
    }
    drop(stream);
    server.terminate();
    let mut restarted = start(&restored, restored_addr);
    wait_until_listening(&mut restarted, restored_addr);
    assert!(keys(&restored) == values);
    assert!(
        request(restored_addr, "GET", "/v1/health/ready", Some(new_key))
            .starts_with("HTTP/1.1 200 ")
    );
    restarted.terminate();
    source.terminate();
}
