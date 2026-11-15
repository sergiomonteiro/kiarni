use super::*;
fn get(addr: SocketAddr, path: &str, host: &str, key: Option<&str>, extra: &str) -> String {
    let mut stream = TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let auth = key
        .map(|key| format!("Authorization: Bearer {key}\r\n"))
        .unwrap_or_default();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {host}\r\n{auth}{extra}\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}
#[test]
fn public_origin_is_explicit_for_assets_and_api_and_forwarded_headers_never_grant_trust() {
    let _scenario = SERVER_SCENARIOS.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    let addr = address();
    let mut server = Server(
        command(&root, addr)
            .args(["--public-origin", "https://DB.Example:9443"])
            .spawn()
            .unwrap(),
    );
    wait_until_listening(&mut server, addr);
    let key = token(&root);
    let spoof = "Forwarded: for=127.0.0.1;host=db.example:9443;proto=https\r\nX-Forwarded-Host: db.example:9443\r\nX-Forwarded-Proto: https\r\nX-Real-IP: 127.0.0.1\r\n";
    for path in [
        "/v1/status",
        "/v1/metrics",
        "/v1/errors/recent",
        "/v1/collections?limit=100",
    ] {
        assert!(get(addr, path, "db.example:9443", None, spoof).starts_with("HTTP/1.1 401 "));
        assert!(get(addr, path, &addr.to_string(), Some(&key), spoof).starts_with("HTTP/1.1 403 "));
        assert!(
            get(
                addr,
                path,
                "db.example:9443",
                Some(&key),
                "Origin: http://db.example:9443\r\n"
            )
            .starts_with("HTTP/1.1 403 ")
        );
        let accepted = get(
            addr,
            path,
            "db.example:9443",
            Some(&key),
            "Origin: https://db.example:9443\r\n",
        );
        assert!(
            accepted.starts_with("HTTP/1.1 200 "),
            "path={path} status={}",
            accepted.lines().next().unwrap_or("missing")
        );
        assert!(
            get(
                addr,
                path,
                "db.example:9443",
                Some(&key),
                "Forwarded: proto=http;host=evil.invalid\r\nX-Forwarded-Host: evil.invalid\r\n"
            )
            .starts_with("HTTP/1.1 200 ")
        );
    }
    assert!(get(addr, "/", "db.example:9443", None, "").starts_with("HTTP/1.1 200 "));
    assert!(get(addr, "/", &addr.to_string(), None, spoof).starts_with("HTTP/1.1 403 "));
    assert!(
        get(addr, "/", "db.example:9443", None, "Origin: null\r\n").starts_with("HTTP/1.1 403 ")
    );
    for origin in [
        "Origin: https://evil.invalid\r\n",
        "Origin: https://db.example:9443\r\nOrigin: https://db.example:9443\r\n",
        "Origin: \r\n",
    ] {
        assert!(
            get(addr, "/v1/status", "db.example:9443", Some(&key), origin)
                .starts_with("HTTP/1.1 403 ")
        );
    }
    server.terminate();
    let mut local = start(&root, addr);
    wait_until_listening(&mut local, addr);
    assert!(request(addr, "GET", "/v1/health/ready", Some(&key)).starts_with("HTTP/1.1 200 "));
    assert!(
        get(addr, "/v1/status", "db.example:9443", Some(&key), spoof).starts_with("HTTP/1.1 403 ")
    );
    local.terminate();
}
