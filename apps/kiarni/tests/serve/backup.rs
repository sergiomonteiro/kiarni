use super::*;
#[test]
fn backup_cli_refuses_running_server_then_exports_without_opening_network_or_reissuing_auth() {
    let _scenario = SERVER_SCENARIOS.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let root = dir.path().join("data");
    let addr = address();
    let mut server = start(&root, addr);
    wait_until_listening(&mut server, addr);
    let key = token(&root);
    assert!(create(addr, &key, "orders").starts_with("HTTP/1.1 201 "));
    assert!(
        insert_document(
            addr,
            &key,
            "orders",
            r#"{"_id":"doc","amount":9007199254740993}"#
        )
        .starts_with("HTTP/1.1 201 ")
    );
    let out = dir.path().join("backup");
    let execute = || {
        Command::new(env!("CARGO_BIN_EXE_kiarni"))
            .env_clear()
            .env("KIARNI_DATA_DIR", &root)
            .arg("backup")
            .arg("--output")
            .arg(&out)
            .output()
            .unwrap()
    };
    let refused = execute();
    assert!(!refused.status.success());
    assert!(refused.stdout.is_empty());
    assert_eq!(refused.stderr, b"kiarni: data_directory_locked\n");
    assert!(!out.exists());
    server.terminate();
    // Explicitly select an occupied address. An offline command must not bind it.
    let occupied = TcpListener::bind("127.0.0.1:0").unwrap();
    let success = Command::new(env!("CARGO_BIN_EXE_kiarni"))
        .env_clear()
        .arg("backup")
        .arg("--data-dir")
        .arg(&root)
        .arg("--output")
        .arg(&out)
        .args(["--bind", &occupied.local_addr().unwrap().to_string()])
        .output()
        .unwrap();
    assert!(success.status.success(), "backup command failed");
    assert!(success.stderr.is_empty());
    assert_eq!(
        success.stdout,
        "Backup concluído (1 coleções, 1 documentos).\n".as_bytes()
    );
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(out.join("backup.json")).unwrap()).unwrap();
    assert_eq!(manifest["documents"], "1");
    assert_eq!(manifest["files"].as_array().unwrap().len(), 2);
    assert!(!String::from_utf8(success.stdout).unwrap().contains(&key));
    let exists = execute();
    assert!(!exists.status.success());
    assert_eq!(exists.stderr, b"kiarni: backup_destination_exists\n");
    let mut restarted = start(&root, addr);
    wait_until_listening(&mut restarted, addr);
    assert_eq!(token(&root), key);
    assert!(
        request(
            addr,
            "GET",
            "/v1/collections/orders/documents/doc",
            Some(&key)
        )
        .contains("9007199254740993")
    );
    restarted.terminate();
}
