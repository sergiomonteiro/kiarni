use super::*;

#[test]
fn embedded_dashboard_is_data_free_and_api_remains_authenticated() {
    let _scenario = SERVER_SCENARIOS.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    let addr = address();
    let mut server = start(&root, addr);
    wait_until_listening(&mut server, addr);
    let key = token(&root);
    assert!(create(addr, &key, "private_collection").starts_with("HTTP/1.1 201 "));
    for path in [
        "/",
        "/dashboard/app.js",
        "/dashboard/style.css",
        "/dashboard/feed.js",
        "/dashboard/metrics.js",
    ] {
        let response = request(addr, "GET", path, None);
        assert!(response.starts_with("HTTP/1.1 200 "));
        assert!(response.contains("Cache-Control: no-store\r\n"));
        assert!(response.contains("Content-Security-Policy: default-src 'none';"));
        assert!(response.contains("X-Content-Type-Options: nosniff\r\n"));
        assert!(!response.contains(&key));
        assert!(!response.contains("private_collection"));
        let head = request(addr, "HEAD", path, None);
        assert!(head.ends_with("\r\n\r\n"));
        assert_eq!(
            head.lines().find(|l| l.starts_with("Content-Length:")),
            response.lines().find(|l| l.starts_with("Content-Length:"))
        );
        assert!(request(addr, "POST", path, None).starts_with("HTTP/1.1 405 "));
        assert!(
            request(addr, "GET", &format!("{path}?token=ignored"), None)
                .starts_with("HTTP/1.1 400 ")
        );
        assert!(
            request_body(
                addr,
                "GET",
                path,
                None,
                "Origin: http://evil.invalid\r\n",
                ""
            )
            .starts_with("HTTP/1.1 403 ")
        );
        assert!(
            request_body(addr, "GET", path, None, "Content-Length: 1\r\n", "x")
                .starts_with("HTTP/1.1 400 ")
        );
        assert!(
            request_body(addr, "GET", path, None, "Host: evil.invalid\r\n", "")
                .starts_with("HTTP/1.1 403 ")
        );
    }
    for path in [
        "/v1/health/live",
        "/v1/health/ready",
        "/v1/collections?limit=100",
    ] {
        assert!(request(addr, "GET", path, None).starts_with("HTTP/1.1 401 "));
        assert!(request(addr, "GET", path, Some(&key)).starts_with("HTTP/1.1 200 "));
    }
    assert!(request(addr, "GET", "/dashboard/missing.js", None).starts_with("HTTP/1.1 401 "));
    server.terminate();
}
