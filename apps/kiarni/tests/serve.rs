#![cfg(unix)]

use std::{
    fs,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

// Ephemeral port selection closes its reservation before spawning the binary.
// Serialize server scenarios to prevent another scenario from reusing that port.
// Concurrency within a scenario still uses independent clients/threads.
static SERVER_SCENARIOS: std::sync::Mutex<()> = std::sync::Mutex::new(());
#[path = "serve/backup.rs"]
mod backup;
#[path = "serve/counts.rs"]
mod counts;
#[path = "serve/dashboard.rs"]
mod dashboard;
#[path = "serve/healthcheck.rs"]
mod healthcheck;
#[path = "serve/http_metrics.rs"]
mod http_metrics;
#[path = "serve/logs.rs"]
mod logs;
#[path = "serve/public_origin.rs"]
mod public_origin;
#[path = "serve/query_metrics.rs"]
mod query_metrics;
#[path = "serve/recent_errors.rs"]
mod recent_errors;
#[path = "serve/restore.rs"]
mod restore;
#[path = "serve/status.rs"]
mod status;
#[path = "serve/storage_metrics.rs"]
mod storage_metrics;
#[path = "serve/streams.rs"]
mod streams;

#[test]
fn configured_stream_limits_above_spike_bounds_still_start_and_serve() {
    let _scenario = SERVER_SCENARIOS.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    let addr = address();
    let mut server = Server(
        command(&root, addr)
            .args([
                "--max-requests",
                "16",
                "--max-streams",
                "9",
                "--feed-queue",
                "9",
            ])
            .spawn()
            .unwrap(),
    );
    wait_until_listening(&mut server, addr);
    let key = token(&root);
    assert!(request(addr, "GET", "/v1/health/ready", Some(&key)).starts_with("HTTP/1.1 200 "));
    server.terminate();
}

#[test]
fn grouped_http_updates_return_own_states_with_concurrent_snapshots_and_restart() {
    let _scenario = SERVER_SCENARIOS.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    let addr = address();
    let mut server = start(&root, addr);
    wait_until_listening(&mut server, addr);
    let key = token(&root);
    assert!(create(addr, &key, "orders").starts_with("HTTP/1.1 201 "));
    assert!(
        insert_document(addr, &key, "orders", r#"{"_id":"doc","v":0}"#)
            .starts_with("HTTP/1.1 201 ")
    );
    let barrier = std::sync::Barrier::new(33);
    let request_ids = thread::scope(|scope| {
        let clients: Vec<_> = (1..=32)
            .map(|number| {
                let barrier = &barrier;
                let key = &key;
                scope.spawn(move || {
                    barrier.wait();
                    let response = update_document(
                        addr,
                        key,
                        "PATCH",
                        "doc",
                        &format!(r#"{{"v":{number},"f{number}":{number}}}"#),
                    );
                    assert!(response.starts_with("HTTP/1.1 200 "), "update failed");
                    let (headers, body) = response.split_once("\r\n\r\n").unwrap();
                    let value: serde_json::Value = serde_json::from_str(body).unwrap();
                    assert_eq!(value["v"], number);
                    assert_eq!(value["_id"], "doc");
                    assert!(headers.contains(&format!("Content-Length: {}", body.len())));
                    assert!(headers.contains("Content-Type: application/json"));
                    headers
                        .lines()
                        .find_map(|line| line.strip_prefix("X-Request-ID: "))
                        .unwrap()
                        .to_owned()
                })
            })
            .collect();
        barrier.wait();
        for _ in 0..4 {
            assert!(
                request(addr, "GET", "/v1/health/ready", Some(&key)).starts_with("HTTP/1.1 200 ")
            );
            let response = query(addr, &key, r#"{"from":"orders","id":"doc","limit":1}"#);
            assert!(response.starts_with("HTTP/1.1 200 "));
            let value: serde_json::Value =
                serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
            assert_eq!(value["count"], 1);
            assert_eq!(value["documents"][0]["_id"], "doc");
        }
        clients
            .into_iter()
            .map(|client| client.join().unwrap())
            .collect::<std::collections::BTreeSet<_>>()
    });
    assert_eq!(request_ids.len(), 32);
    server.terminate();
    let addr = address();
    let mut restarted = start(&root, addr);
    wait_until_listening(&mut restarted, addr);
    let response = request(
        addr,
        "GET",
        "/v1/collections/orders/documents/doc",
        Some(&key),
    );
    let (_, body) = response.split_once("\r\n\r\n").unwrap();
    let value: serde_json::Value = serde_json::from_str(body).unwrap();
    for number in 1..=32 {
        assert_eq!(value[format!("f{number}")], number);
    }
    let head = request(
        addr,
        "HEAD",
        "/v1/collections/orders/documents/doc",
        Some(&key),
    );
    assert!(head.ends_with("\r\n\r\n"));
    assert!(head.contains(&format!("Content-Length: {}", body.len())));
    assert!(
        request(
            addr,
            "DELETE",
            "/v1/collections/orders/documents/doc",
            Some(&key)
        )
        .starts_with("HTTP/1.1 204 ")
    );
    assert!(
        request(addr, "DELETE", "/v1/collections/orders", Some(&key)).starts_with("HTTP/1.1 204 ")
    );
    assert!(create(addr, &key, "orders").starts_with("HTTP/1.1 201 "));
    assert!(
        insert_document(addr, &key, "orders", r#"{"_id":"doc","v":99}"#)
            .starts_with("HTTP/1.1 201 ")
    );
    let response = update_document(addr, &key, "PATCH", "doc", r#"{"v":100}"#);
    let value: serde_json::Value =
        serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(value["v"], 100);
    assert_eq!(value.as_object().unwrap().len(), 2);
    restarted.terminate();
}

struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Server {
    fn wait(&mut self) -> ExitStatus {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = self.0.try_wait().unwrap() {
                return status;
            }
            assert!(Instant::now() < deadline, "server did not exit");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn terminate(&mut self) {
        let pid = rustix::process::Pid::from_raw(self.0.id().try_into().unwrap()).unwrap();
        rustix::process::kill_process(pid, rustix::process::Signal::TERM).unwrap();
        assert!(self.wait().success());
    }

    fn failure(mut self, code: &str) {
        assert!(!self.wait().success());
        let mut stderr = String::new();
        self.0
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut stderr)
            .unwrap();
        assert!(stderr.contains(code), "{stderr}");
    }
}

fn address() -> SocketAddr {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
}

fn command(root: &Path, addr: SocketAddr) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_kiarni"));
    cmd.env_clear()
        .args(["serve", "--data-dir"])
        .arg(root)
        .args(["--bind", &addr.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    cmd
}

fn start(root: &Path, addr: SocketAddr) -> Server {
    Server(command(root, addr).spawn().unwrap())
}

fn wait_until_listening(server: &mut Server, addr: SocketAddr) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(server.0.try_wait().unwrap().is_none(), "startup failed");
        if TcpStream::connect_timeout(&addr, Duration::from_millis(50)).is_ok() {
            return;
        }
        assert!(Instant::now() < deadline, "server did not listen");
        thread::sleep(Duration::from_millis(10));
    }
}

fn credentials(root: &Path) -> PathBuf {
    root.with_file_name("data.credentials")
}

fn token(root: &Path) -> String {
    let paths: Vec<_> = fs::read_dir(credentials(root))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(paths.len(), 1);
    let path = paths[0].path();
    assert_eq!(path.metadata().unwrap().permissions().mode() & 0o777, 0o600);
    fs::read_to_string(path).unwrap()
}

fn request(addr: SocketAddr, method: &str, path: &str, token: Option<&str>) -> String {
    request_body(addr, method, path, token, "", "")
}

fn request_body(
    addr: SocketAddr,
    method: &str,
    path: &str,
    token: Option<&str>,
    extra: &str,
    body: &str,
) -> String {
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(1)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let auth = token
        .map(|value| format!("Authorization: Bearer {value}\r\n"))
        .unwrap_or_default();
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\n{auth}{extra}\r\n{body}"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

fn create(addr: SocketAddr, token: &str, name: &str) -> String {
    let body = serde_json::to_string(&serde_json::json!({"name": name})).unwrap();
    request_body(
        addr,
        "POST",
        "/v1/collections",
        Some(token),
        &format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            body.len()
        ),
        &body,
    )
}

fn insert_document(addr: SocketAddr, token: &str, collection: &str, body: &str) -> String {
    request_body(
        addr,
        "POST",
        &format!("/v1/collections/{collection}/documents"),
        Some(token),
        &format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            body.len()
        ),
        body,
    )
}

#[test]
fn documents_generated_ids_raw_numbers_and_primary_lookup_survive_kill_restart() {
    let _scenario = SERVER_SCENARIOS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    let addr = address();
    let mut server = start(&root, addr);
    wait_until_listening(&mut server, addr);
    let key = token(&root);
    assert!(create(addr, &key, "orders").starts_with("HTTP/1.1 201 "));
    assert!(create(addr, &key, "other").starts_with("HTTP/1.1 201 "));
    let source =
        r#"{"_id":"fixed","n":123456789012345678901234567890,"nested":{"x":null,"e":1e10001}}"#;
    let inserted = insert_document(addr, &key, "orders", source);
    assert!(inserted.starts_with("HTTP/1.1 201 "), "{inserted}");
    assert!(inserted.contains("Location: /v1/collections/orders/documents/fixed\r\n"));
    let persisted = inserted.split_once("\r\n\r\n").unwrap().1.to_owned();
    assert!(persisted.contains("123456789012345678901234567890"));
    assert!(persisted.contains("1e10001"));
    assert!(insert_document(addr, &key, "orders", source).starts_with("HTTP/1.1 409 "));
    assert!(insert_document(addr, &key, "other", source).starts_with("HTTP/1.1 201 "));
    let generated = insert_document(addr, &key, "orders", r#"{"value":true}"#);
    assert!(generated.starts_with("HTTP/1.1 201 "));
    let json: serde_json::Value =
        serde_json::from_str(generated.split_once("\r\n\r\n").unwrap().1).unwrap();
    let id = json["_id"].as_str().unwrap();
    assert_eq!(id.len(), 32);
    assert!(
        id.bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    );
    let path = format!("/v1/collections/orders/documents/{id}");
    assert!(
        request(addr, "GET", &path, Some(&key))
            .ends_with(generated.split_once("\r\n\r\n").unwrap().1)
    );
    let head = request(
        addr,
        "HEAD",
        "/v1/collections/orders/documents/fixed",
        Some(&key),
    );
    assert!(head.starts_with("HTTP/1.1 200 "));
    assert!(head.contains(&format!("Content-Length: {}\r\n", persisted.len())));
    assert!(head.ends_with("\r\n\r\n"));
    assert!(
        request(addr, "DELETE", "/v1/collections/orders", Some(&key))
            .contains("\"code\":\"collection_not_empty\"")
    );
    server.0.kill().unwrap();
    assert!(!server.wait().success());
    let addr = address();
    let mut restarted = start(&root, addr);
    wait_until_listening(&mut restarted, addr);
    assert!(
        request(
            addr,
            "GET",
            "/v1/collections/orders/documents/fixed",
            Some(&key)
        )
        .ends_with(&persisted)
    );
    assert!(request(addr, "GET", &path, Some(&key)).starts_with("HTTP/1.1 200 "));
    restarted.terminate();
}

#[test]
fn document_errors_are_authenticated_and_do_not_create_collections() {
    let _scenario = SERVER_SCENARIOS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    let addr = address();
    let mut server = start(&root, addr);
    wait_until_listening(&mut server, addr);
    let key = token(&root);
    assert!(
        insert_document(addr, &key, "missing", "{}").contains("\"code\":\"collection_not_found\"")
    );
    assert!(
        request(addr, "GET", "/v1/collections?limit=10", Some(&key))
            .ends_with(r#"{"collections":[],"next_after":null}"#)
    );
    assert!(create(addr, &key, "orders").starts_with("HTTP/1.1 201 "));
    for (body, code) in [
        ("{", "invalid_json"),
        (r#"{"_id":"a","_id":"b"}"#, "invalid_json"),
        ("[]", "document_must_be_object"),
        (r#"{"_id":null}"#, "invalid_id"),
        (r#"{"_id":"../bad"}"#, "invalid_id"),
    ] {
        let response = insert_document(addr, &key, "orders", body);
        assert!(
            response.contains(&format!("\"code\":\"{code}\"")),
            "{response}"
        );
    }
    assert!(
        request(
            addr,
            "GET",
            "/v1/collections/orders/documents/absent",
            Some(&key)
        )
        .contains("\"code\":\"document_not_found\"")
    );
    assert!(
        request(
            addr,
            "GET",
            "/v1/collections/missing/documents/absent",
            Some(&key)
        )
        .contains("\"code\":\"collection_not_found\"")
    );
    assert!(
        request(addr, "GET", "/v1/collections/orders/documents/absent", None)
            .starts_with("HTTP/1.1 401 ")
    );
    assert!(
        request_body(
            addr,
            "POST",
            "/v1/collections/orders/documents",
            None,
            "Content-Type: application/json\r\nContent-Length: 2\r\n",
            "{}"
        )
        .starts_with("HTTP/1.1 401 ")
    );
    assert!(
        request_body(
            addr,
            "POST",
            "/v1/collections/orders/documents",
            Some(&key),
            "Content-Type: text/plain\r\nContent-Length: 0\r\n",
            ""
        )
        .starts_with("HTTP/1.1 415 ")
    );
    assert!(
        request(addr, "GET", "/v1/collections?limit=10", Some(&key))
            .ends_with(r#"{"collections":["orders"],"next_after":null}"#)
    );
    server.terminate();
}

#[test]
fn lower_write_limit_preserves_readability_and_checks_final_generated_size() {
    let _scenario = SERVER_SCENARIOS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    let addr = address();
    let mut server = start(&root, addr);
    wait_until_listening(&mut server, addr);
    let key = token(&root);
    create(addr, &key, "orders");
    let source = format!(r#"{{"_id":"old","data":"{}"}}"#, "x".repeat(1000));
    let inserted = insert_document(addr, &key, "orders", &source);
    assert!(inserted.starts_with("HTTP/1.1 201 "));
    let persisted = inserted.split_once("\r\n\r\n").unwrap().1.to_owned();
    server.terminate();

    let addr = address();
    let mut restarted = Server(
        command(&root, addr)
            .args(["--max-document-bytes", "41"])
            .spawn()
            .unwrap(),
    );
    wait_until_listening(&mut restarted, addr);
    assert!(
        request(
            addr,
            "GET",
            "/v1/collections/orders/documents/old",
            Some(&key)
        )
        .ends_with(&persisted)
    );
    assert!(
        insert_document(addr, &key, "orders", "{}").contains("\"code\":\"document_too_large\"")
    );
    assert!(
        request_body(
            addr,
            "POST",
            "/v1/collections/orders/documents",
            Some(&key),
            "Content-Type: application/json\r\nContent-Length: 42\r\n",
            ""
        )
        .starts_with("HTTP/1.1 413 ")
    );
    let exact = r#"{"_id":"fit","v":"xxxxxxxxxxxxxxxxxxxxx"}"#;
    assert_eq!(exact.len(), 41);
    assert!(insert_document(addr, &key, "orders", exact).starts_with("HTTP/1.1 201 "));
    restarted.terminate();
}

#[test]
fn concurrent_document_insert_has_one_winner_and_never_overwrites() {
    let _scenario = SERVER_SCENARIOS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    let addr = address();
    let mut server = start(&root, addr);
    wait_until_listening(&mut server, addr);
    let key = token(&root);
    create(addr, &key, "orders");
    let responses = thread::scope(|scope| {
        let workers: Vec<_> = (0..8)
            .map(|index| {
                let key = &key;
                scope.spawn(move || {
                    insert_document(
                        addr,
                        key,
                        "orders",
                        &format!(r#"{{"_id":"one","value":{index}}}"#),
                    )
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>()
    });
    let winners: Vec<_> = responses
        .iter()
        .filter(|response| response.starts_with("HTTP/1.1 201 "))
        .collect();
    assert_eq!(winners.len(), 1);
    assert!(responses.iter().all(|response| {
        [201, 409, 503]
            .iter()
            .any(|status| response.starts_with(&format!("HTTP/1.1 {status} ")))
    }));
    let committed = winners[0].split_once("\r\n\r\n").unwrap().1;
    assert!(
        request(
            addr,
            "GET",
            "/v1/collections/orders/documents/one",
            Some(&key)
        )
        .ends_with(committed)
    );
    server.terminate();
}

#[test]
fn collection_lifecycle_pagination_and_restart_over_http() {
    let _scenario = SERVER_SCENARIOS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    let addr = address();
    let mut server = start(&root, addr);
    wait_until_listening(&mut server, addr);
    let key = token(&root);
    let empty = request(addr, "GET", "/v1/collections?limit=2", Some(&key));
    assert!(empty.ends_with(r#"{"collections":[],"next_after":null}"#));
    for name in ["zeta", "alpha", "middle"] {
        let response = create(addr, &key, name);
        assert!(response.starts_with("HTTP/1.1 201 "), "{response}");
        assert!(response.contains(&format!("Location: /v1/collections/{name}\r\n")));
    }
    assert!(create(addr, &key, "alpha").starts_with("HTTP/1.1 409 "));
    let page = request(addr, "GET", "/v1/collections?limit=2", Some(&key));
    assert!(page.ends_with(r#"{"collections":["alpha","middle"],"next_after":"middle"}"#));
    let page = request(
        addr,
        "GET",
        "/v1/collections?after=middle&limit=2",
        Some(&key),
    );
    assert!(page.ends_with(r#"{"collections":["zeta"],"next_after":null}"#));
    let head = request(addr, "HEAD", "/v1/collections?limit=2", Some(&key));
    assert!(head.starts_with("HTTP/1.1 200 "));
    assert!(head.ends_with("\r\n\r\n"));
    let deleted = request(addr, "DELETE", "/v1/collections/middle", Some(&key));
    assert!(deleted.starts_with("HTTP/1.1 204 "));
    assert!(!deleted.contains("Content-Length:"));
    assert!(deleted.ends_with("\r\n\r\n"));
    assert!(
        request(addr, "DELETE", "/v1/collections/middle", Some(&key)).starts_with("HTTP/1.1 404 ")
    );
    server.0.kill().unwrap();
    assert!(!server.wait().success());
    let addr = address();
    let mut restarted = start(&root, addr);
    wait_until_listening(&mut restarted, addr);
    assert!(
        request(addr, "GET", "/v1/collections?limit=1000", Some(&key))
            .ends_with(r#"{"collections":["alpha","zeta"],"next_after":null}"#)
    );
    restarted.terminate();
}

#[test]
fn collection_validation_authentication_and_nonempty_delete() {
    let _scenario = SERVER_SCENARIOS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
    let store = kiarni_storage::Store::create(&root.join("store")).unwrap();
    let collection = store.create_collection("populated").unwrap();
    store
        .insert_fixture(&collection, br#"{"_id":"doc"}"#)
        .unwrap();
    drop(store);
    phase(&root, "initializing_store");
    let addr = address();
    let mut server = start(&root, addr);
    wait_until_listening(&mut server, addr);
    let key = token(&root);
    assert!(request(addr, "GET", "/v1/collections?limit=1", None).starts_with("HTTP/1.1 401 "));
    assert!(
        request(addr, "DELETE", "/v1/collections/populated", Some(&key))
            .contains("\"code\":\"collection_not_empty\"")
    );
    for path in [
        "/v1/collections",
        "/v1/collections?limit=0",
        "/v1/collections?limit=1001",
        "/v1/collections?limit=1&limit=2",
        "/v1/collections?limit=2&extra=1",
        "/v1/collections?limit=1&after=a%20b",
    ] {
        let response = request(addr, "GET", path, Some(&key));
        assert!(response.contains("\"code\":\"invalid_"), "{response}");
    }
    for (body, code) in [
        (r#"{"name":"../bad"}"#, "invalid_collection"),
        (r#"{"name":"new","extra":1}"#, "invalid_collection"),
        (r#"{"name":"new","name":"other"}"#, "invalid_json"),
        ("{", "invalid_json"),
    ] {
        let response = request_body(
            addr,
            "POST",
            "/v1/collections",
            Some(&key),
            &format!(
                "Content-Type: application/json\r\nContent-Length: {}\r\n",
                body.len()
            ),
            body,
        );
        assert!(
            response.contains(&format!("\"code\":\"{code}\"")),
            "{response}"
        );
    }
    let response = request_body(
        addr,
        "POST",
        "/v1/collections",
        Some(&key),
        "Content-Type: text/plain\r\nContent-Length: 0\r\n",
        "",
    );
    assert!(response.starts_with("HTTP/1.1 415 "));
    let response = request_body(
        addr,
        "POST",
        "/v1/collections",
        Some(&key),
        "Content-Type: application/json\r\nContent-Length: 1025\r\n",
        "",
    );
    assert!(response.starts_with("HTTP/1.1 413 "));
    assert!(
        request(addr, "GET", "/v1/collections/populated", Some(&key)).contains("Allow: DELETE")
    );
    assert!(
        request(addr, "GET", "/v1/collections?limit=2", Some(&key))
            .ends_with(r#"{"collections":["populated"],"next_after":null}"#)
    );
    server.terminate();
}

#[test]
fn concurrent_collection_creation_has_one_committed_winner() {
    let _scenario = SERVER_SCENARIOS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    let addr = address();
    let mut server = start(&root, addr);
    wait_until_listening(&mut server, addr);
    let key = token(&root);
    let results = thread::scope(|scope| {
        let workers: Vec<_> = (0..8)
            .map(|_| scope.spawn(|| create(addr, &key, "shared")))
            .collect();
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(
        results
            .iter()
            .filter(|response| response.starts_with("HTTP/1.1 201 "))
            .count(),
        1
    );
    assert!(results.iter().all(|response| {
        [201, 409, 503]
            .iter()
            .any(|status| response.starts_with(&format!("HTTP/1.1 {status} ")))
    }));
    assert!(
        request(addr, "GET", "/v1/collections?limit=10", Some(&key))
            .ends_with(r#"{"collections":["shared"],"next_after":null}"#)
    );
    server.terminate();
}

#[test]
fn incomplete_collection_body_times_out_without_writing_and_split_body_succeeds() {
    let _scenario = SERVER_SCENARIOS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    let addr = address();
    let mut server = Server(
        command(&root, addr)
            .args(["--read-timeout-ms", "200"])
            .spawn()
            .unwrap(),
    );
    wait_until_listening(&mut server, addr);
    let key = token(&root);
    let body = r#"{"name":"split"}"#;
    let mut stream = TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    write!(stream, "POST /v1/collections HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {key}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{{", body.len()).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 408 "), "{response}");
    assert!(
        request(addr, "GET", "/v1/collections?limit=1", Some(&key))
            .ends_with(r#"{"collections":[],"next_after":null}"#)
    );

    let mut stream = TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    write!(stream, "POST /v1/collections HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {key}\r\nContent-Type: Application/JSON; charset=UTF-8\r\nContent-Length: {}\r\n\r\n", body.len()).unwrap();
    thread::sleep(Duration::from_millis(10));
    stream.write_all(body.as_bytes()).unwrap();
    response.clear();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 201 "), "{response}");
    assert!(
        request(
            addr,
            "GET",
            "/v1/collections?limit=1&after=split",
            Some(&key)
        )
        .ends_with(r#"{"collections":[],"next_after":null}"#)
    );
    server.terminate();
}

fn phase(root: &Path, name: &str) {
    let file = root.join("bootstrap.json");
    fs::write(
        &file,
        format!(r#"{{"version":2,"store_format":2,"phase":"{name}"}}"#),
    )
    .unwrap();
    fs::set_permissions(file, fs::Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn authenticated_health_lock_shutdown_and_restart() {
    let _scenario = SERVER_SCENARIOS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    let addr = address();
    let mut server = start(&root, addr);
    wait_until_listening(&mut server, addr);
    let original = token(&root);
    let denied = request(addr, "GET", "/v1/health/ready", None);
    assert!(denied.starts_with("HTTP/1.1 401 "));
    assert!(denied.contains("WWW-Authenticate: Bearer"));
    let ready = request(addr, "GET", "/v1/health/ready", Some(&original));
    assert!(ready.starts_with("HTTP/1.1 200 "));
    assert!(ready.ends_with(r#"{"status":"ready"}"#));
    assert!(ready.contains("X-Request-ID: req_"));
    let head = request(addr, "HEAD", "/v1/health/live", Some(&original));
    assert!(head.starts_with("HTTP/1.1 200 "));
    assert!(head.ends_with("\r\n\r\n"));
    assert!(request(addr, "POST", "/v1/health/live", Some(&original)).starts_with("HTTP/1.1 405 "));
    start(&root, address()).failure("data_directory_locked");
    let mut idle = TcpStream::connect(addr).unwrap();
    idle.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    idle.write_all(b"GET /v1/health/live HTTP/1.1\r\n").unwrap();
    server.terminate();
    match idle.read(&mut [0]) {
        Ok(0) => {}
        Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {}
        other => panic!("connection remained open after shutdown: {other:?}"),
    }

    let addr = address();
    let mut restarted = start(&root, addr);
    wait_until_listening(&mut restarted, addr);
    assert!(token(&root) == original);
    assert!(request(addr, "GET", "/v1/health/live", Some(&original)).starts_with("HTTP/1.1 200 "));
    restarted.terminate();
}

#[test]
fn resumes_existing_store_and_committed_auth_without_issuing_again() {
    let _scenario = SERVER_SCENARIOS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
    drop(kiarni_storage::Store::create(&root.join("store")).unwrap());
    phase(&root, "initializing_store");
    let addr = address();
    let mut server = start(&root, addr);
    wait_until_listening(&mut server, addr);
    let original = token(&root);
    server.0.kill().unwrap();
    assert!(!server.wait().success());

    // Disk checkpoint after auth commit and before root completion.
    phase(&root, "initializing_auth");
    let addr = address();
    let mut restarted = start(&root, addr);
    wait_until_listening(&mut restarted, addr);
    assert!(token(&root) == original);
    assert!(request(addr, "GET", "/v1/health/ready", Some(&original)).starts_with("HTTP/1.1 200 "));
    restarted.terminate();
    fs::remove_file(root.join("auth/marker.json")).unwrap();
    start(&root, address()).failure("bootstrap_incomplete");
    assert!(token(&root) == original);
}

#[test]
fn incompatible_store_fails_before_auth_and_config_precedence_is_applied() {
    let _scenario = SERVER_SCENARIOS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
    drop(kiarni_storage::Store::create(&root.join("store")).unwrap());
    phase(&root, "initializing_store");
    let manifest = root.join("store/format.json");
    let bytes = br#"{"magic":"kiarni","version":99,"engine":"redb-4.5.0"}"#;
    fs::write(&manifest, bytes).unwrap();
    start(&root, address()).failure("unsupported_format");
    assert!(!root.join("auth").exists());
    assert!(!credentials(&root).exists());
    assert_eq!(fs::read(manifest).unwrap(), bytes);

    let clean = dir.path().join("clean");
    let config = dir.path().join("config.json");
    fs::write(&config, br#"{"bind":"127.0.0.1:1","data_dir":"unused"}"#).unwrap();
    let addr = address();
    let mut server = Server(
        command(&clean, addr)
            .arg("--config")
            .arg(&config)
            .env("KIARNI_BIND", "127.0.0.1:2")
            .spawn()
            .unwrap(),
    );
    wait_until_listening(&mut server, addr);
    assert!(clean.join("store/data.redb").is_file());
    server.terminate();
}

fn update_document(addr: SocketAddr, token: &str, method: &str, id: &str, body: &str) -> String {
    let media = if method == "PATCH" {
        "application/merge-patch+json"
    } else {
        "application/json"
    };
    request_body(
        addr,
        method,
        &format!("/v1/collections/orders/documents/{id}"),
        Some(token),
        &format!(
            "Content-Type: {media}\r\nContent-Length: {}\r\n",
            body.len()
        ),
        body,
    )
}

#[test]
fn put_patch_delete_preserve_identity_and_committed_state_after_restart() {
    let _scenario = SERVER_SCENARIOS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    let addr = address();
    let mut server = start(&root, addr);
    wait_until_listening(&mut server, addr);
    let key = token(&root);
    create(addr, &key, "orders");
    insert_document(addr, &key, "orders", r#"{"_id":"doc","old":true,"v":1}"#);
    let replaced = update_document(
        addr,
        &key,
        "PUT",
        "doc",
        r#"{"profile":{"name":"Ana","age":30},"arr":[1,2],"n":1e10001}"#,
    );
    assert!(replaced.starts_with("HTTP/1.1 200 "), "{replaced}");
    let replaced_body = replaced.split_once("\r\n\r\n").unwrap().1;
    assert!(!replaced_body.contains("old"));
    assert!(replaced_body.contains("\"_id\":\"doc\""));
    let patch = r#"{"profile":{"age":null,"_id":"nested"},"arr":[null],"n":123456789012345678901234567890}"#;
    let updated = update_document(addr, &key, "PATCH", "doc", patch);
    assert!(updated.starts_with("HTTP/1.1 200 "), "{updated}");
    let committed = updated.split_once("\r\n\r\n").unwrap().1.to_owned();
    assert_eq!(
        committed,
        r#"{"_id":"doc","arr":[null],"n":123456789012345678901234567890,"profile":{"_id":"nested","name":"Ana"}}"#
    );
    assert!(update_document(addr, &key, "PATCH", "doc", "{}").ends_with(&committed));
    insert_document(addr, &key, "orders", r#"{"_id":"gone"}"#);
    let deleted = request(
        addr,
        "DELETE",
        "/v1/collections/orders/documents/gone",
        Some(&key),
    );
    assert!(deleted.starts_with("HTTP/1.1 204 "));
    assert!(!deleted.contains("Content-Length:"));
    assert!(deleted.ends_with("\r\n\r\n"));
    assert!(
        request(
            addr,
            "DELETE",
            "/v1/collections/orders/documents/gone",
            Some(&key)
        )
        .starts_with("HTTP/1.1 404 ")
    );
    server.0.kill().unwrap();
    assert!(!server.wait().success());
    let addr = address();
    let mut restarted = start(&root, addr);
    wait_until_listening(&mut restarted, addr);
    assert!(
        request(
            addr,
            "GET",
            "/v1/collections/orders/documents/doc",
            Some(&key)
        )
        .ends_with(&committed)
    );
    assert!(
        request(
            addr,
            "GET",
            "/v1/collections/orders/documents/gone",
            Some(&key)
        )
        .contains("\"code\":\"document_not_found\"")
    );
    assert!(
        request(
            addr,
            "DELETE",
            "/v1/collections/orders/documents/doc",
            Some(&key)
        )
        .starts_with("HTTP/1.1 204 ")
    );
    assert!(
        request(addr, "DELETE", "/v1/collections/orders", Some(&key)).starts_with("HTTP/1.1 204 ")
    );
    restarted.terminate();
}

#[test]
fn document_update_errors_validate_before_lookup_and_leave_data_unchanged() {
    let _scenario = SERVER_SCENARIOS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    let addr = address();
    let mut server = start(&root, addr);
    wait_until_listening(&mut server, addr);
    let key = token(&root);
    create(addr, &key, "orders");
    let inserted = insert_document(addr, &key, "orders", r#"{"_id":"doc","v":1}"#);
    let original = inserted.split_once("\r\n\r\n").unwrap().1;
    for (method, body, code) in [
        ("PUT", r#"{"_id":"other"}"#, "id_mismatch"),
        ("PATCH", r#"{"_id":"doc"}"#, "immutable_id"),
        ("PATCH", r#"{"_id":null}"#, "immutable_id"),
        ("PATCH", r#"{"v":1,"v":2}"#, "invalid_json"),
        ("PATCH", "[]", "document_must_be_object"),
        ("PUT", "{", "invalid_json"),
    ] {
        for id in ["doc", "absent"] {
            let response = update_document(addr, &key, method, id, body);
            assert!(
                response.contains(&format!("\"code\":\"{code}\"")),
                "{response}"
            );
        }
    }
    for method in ["PUT", "PATCH"] {
        assert!(
            update_document(addr, &key, method, "absent", "{}")
                .contains("\"code\":\"document_not_found\"")
        );
    }
    assert!(
        request_body(
            addr,
            "PATCH",
            "/v1/collections/orders/documents/doc",
            Some(&key),
            "Content-Type: application/json\r\nContent-Length: 2\r\n",
            "{}"
        )
        .starts_with("HTTP/1.1 415 ")
    );
    assert!(
        request(addr, "DELETE", "/v1/collections/orders/documents/doc", None)
            .starts_with("HTTP/1.1 401 ")
    );
    assert!(
        request(
            addr,
            "GET",
            "/v1/collections/orders/documents/doc",
            Some(&key)
        )
        .ends_with(original)
    );
    server.terminate();
}

#[test]
fn no_op_above_reduced_limit_returns_original_and_changes_must_fit() {
    let _scenario = SERVER_SCENARIOS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    let addr = address();
    let mut server = start(&root, addr);
    wait_until_listening(&mut server, addr);
    let key = token(&root);
    create(addr, &key, "orders");
    let source = format!(r#"{{"_id":"doc","text":"{}","v":1}}"#, "x".repeat(200));
    let inserted = insert_document(addr, &key, "orders", &source);
    let original = inserted.split_once("\r\n\r\n").unwrap().1.to_owned();
    server.terminate();
    let addr = address();
    let mut restarted = Server(
        command(&root, addr)
            .args(["--max-document-bytes", "64"])
            .spawn()
            .unwrap(),
    );
    wait_until_listening(&mut restarted, addr);
    assert!(update_document(addr, &key, "PATCH", "doc", "{}").ends_with(&original));
    assert!(
        update_document(addr, &key, "PATCH", "doc", r#"{"v":2}"#)
            .contains("\"code\":\"document_too_large\"")
    );
    assert!(
        request(
            addr,
            "GET",
            "/v1/collections/orders/documents/doc",
            Some(&key)
        )
        .ends_with(&original)
    );
    let shrunk = update_document(addr, &key, "PATCH", "doc", r#"{"text":null}"#);
    assert!(shrunk.starts_with("HTTP/1.1 200 "));
    assert!(shrunk.ends_with(r#"{"_id":"doc","v":1}"#));
    let expanded_by_id = format!(r#"{{"text":"{}"}}"#, "x".repeat(50));
    assert!(expanded_by_id.len() <= 64);
    assert!(
        update_document(addr, &key, "PUT", "doc", &expanded_by_id)
            .contains("\"code\":\"document_too_large\"")
    );
    assert!(
        request(
            addr,
            "GET",
            "/v1/collections/orders/documents/doc",
            Some(&key)
        )
        .ends_with(r#"{"_id":"doc","v":1}"#)
    );
    restarted.terminate();
}

fn query(addr: SocketAddr, token: &str, body: &str) -> String {
    request_body(
        addr,
        "POST",
        "/v1/collections/orders/query",
        Some(token),
        &format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            body.len()
        ),
        body,
    )
}

#[test]
fn persisted_queries_compose_exact_filter_sort_limit_project_and_id() {
    let _scenario = SERVER_SCENARIOS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    let addr = address();
    let mut server = start(&root, addr);
    wait_until_listening(&mut server, addr);
    let key = token(&root);
    create(addr, &key, "orders");
    for body in [
        r#"{"_id":"a","n":9007199254740993,"score":2}"#,
        r#"{"_id":"b","n":9007199254740992,"score":2}"#,
        r#"{"_id":"c","n":9007199254740994,"score":1}"#,
    ] {
        assert!(insert_document(addr, &key, "orders", body).starts_with("HTTP/1.1 201 "));
    }
    let ast = r#"{"from":"orders","filter":{"op":"gt","field":"n","value":9007199254740992},"sort":[{"field":"score","direction":"asc"}],"limit":2,"project":["n"]}"#;
    let expected = r#"{"documents":[{"_id":"c","n":9007199254740994},{"_id":"a","n":9007199254740993}],"count":2}"#;
    let response = query(addr, &key, ast);
    assert!(response.starts_with("HTTP/1.1 200 "), "{response}");
    assert!(response.ends_with(expected), "{response}");
    assert!(
        query(addr, &key, r#"{"from":"orders","id":"absent","limit":1}"#)
            .ends_with(r#"{"documents":[],"count":0}"#)
    );
    assert!(
        query(
            addr,
            &key,
            r#"{"from":"orders","id":"a","filter":{"op":"eq","field":"score","value":1},"limit":1}"#
        )
        .ends_with(r#"{"documents":[],"count":0}"#)
    );
    let by_id = query(
        addr,
        &key,
        r#"{"from":"orders","id":"b","limit":1,"project":[]}"#,
    );
    assert!(by_id.ends_with(r#"{"documents":[{"_id":"b"}],"count":1}"#));
    server.0.kill().unwrap();
    assert!(!server.wait().success());
    let addr = address();
    let mut restarted = start(&root, addr);
    wait_until_listening(&mut restarted, addr);
    assert!(query(addr, &key, ast).ends_with(expected));
    restarted.terminate();
}

#[test]
fn query_errors_return_complete_envelopes_and_require_authentication() {
    let _scenario = SERVER_SCENARIOS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    let addr = address();
    let mut server = start(&root, addr);
    wait_until_listening(&mut server, addr);
    let key = token(&root);
    assert!(
        query(addr, &key, r#"{"from":"orders","limit":1}"#)
            .contains("\"code\":\"collection_not_found\"")
    );
    create(addr, &key, "orders");
    insert_document(addr, &key, "orders", r#"{"_id":"a","n":1e10001,"v":[]}"#);
    for (ast, code) in [
        (r#"{"from":"other","limit":1}"#, "collection_mismatch"),
        (r#"{"from":"orders","limit":0}"#, "invalid_limit"),
        (
            r#"{"from":"orders","limit":1,"unknown":true}"#,
            "invalid_query",
        ),
        (r#"{"from":"orders","limit":1,"limit":2}"#, "invalid_json"),
        (
            r#"{"from":"orders","limit":1,"sort":[{"field":"v","direction":"asc"}]}"#,
            "unsupported_sort_value",
        ),
        (
            r#"{"from":"orders","limit":1,"filter":{"op":"eq","field":"n","value":1}}"#,
            "unsupported_number",
        ),
    ] {
        let response = query(addr, &key, ast);
        assert!(
            response.contains(&format!("\"code\":\"{code}\"")),
            "{response}"
        );
        assert!(!response.contains("\"documents\":"));
    }
    assert!(
        query(addr, &key, r#"{"from":"orders","limit":1,"project":["n"]}"#)
            .ends_with(r#"{"documents":[{"_id":"a","n":1e10001}],"count":1}"#)
    );
    assert!(
        request_body(
            addr,
            "POST",
            "/v1/collections/orders/query",
            None,
            "Content-Type: application/json\r\nContent-Length: 0\r\n",
            ""
        )
        .starts_with("HTTP/1.1 401 ")
    );
    assert!(
        request_body(
            addr,
            "POST",
            "/v1/collections/orders/query",
            Some(&key),
            "Content-Type: application/json\r\nContent-Length: 16385\r\n",
            ""
        )
        .contains("\"code\":\"query_too_large\"")
    );
    server.terminate();
}

#[test]
fn query_envelope_limit_is_checked_after_projection() {
    let _scenario = SERVER_SCENARIOS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
    let store = kiarni_storage::Store::create(&root.join("store")).unwrap();
    let collection = store.create_collection("orders").unwrap();
    for id in ["a", "b"] {
        store
            .insert_fixture(
                &collection,
                format!(r#"{{"_id":"{id}","large":"{}"}}"#, "x".repeat(600000)).as_bytes(),
            )
            .unwrap();
    }
    drop(store);
    phase(&root, "initializing_store");
    let addr = address();
    let mut server = Server(
        command(&root, addr)
            .args(["--max-result-bytes", "1049600"])
            .spawn()
            .unwrap(),
    );
    wait_until_listening(&mut server, addr);
    let key = token(&root);
    let failed = query(addr, &key, r#"{"from":"orders","limit":2}"#);
    assert!(failed.contains("\"code\":\"result_too_large\""), "{failed}");
    assert!(!failed.contains("\"documents\":"));
    assert!(
        query(addr, &key, r#"{"from":"orders","limit":2,"project":[]}"#)
            .ends_with(r#"{"documents":[{"_id":"a"},{"_id":"b"}],"count":2}"#)
    );
    server.terminate();
}

fn auth_command(root: &Path, operation: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_kiarni"))
        .env_clear()
        .args(["auth", operation, "--data-dir"])
        .arg(root)
        .output()
        .unwrap()
}

fn token_paths(root: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<_> = fs::read_dir(credentials(root))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    paths.sort();
    paths
}

#[test]
fn offline_auth_rotation_revocation_and_recovery_persist_across_restart() {
    let _scenario = SERVER_SCENARIOS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    let addr = address();
    let mut server = start(&root, addr);
    wait_until_listening(&mut server, addr);
    let original = token(&root);
    let initial_files = token_paths(&root);
    let active_before = fs::read(root.join("auth/active.json")).unwrap();
    let blocked = auth_command(&root, "rotate");
    assert!(!blocked.status.success());
    assert!(String::from_utf8_lossy(&blocked.stderr).contains("data_directory_locked"));
    assert_eq!(
        fs::read(root.join("auth/active.json")).unwrap(),
        active_before
    );
    assert_eq!(token_paths(&root), initial_files);
    server.terminate();

    let rotated = auth_command(&root, "rotate");
    assert!(
        rotated.status.success(),
        "{}",
        String::from_utf8_lossy(&rotated.stderr)
    );
    let paths = token_paths(&root);
    assert_eq!(paths.len(), 2);
    let new_path = paths
        .iter()
        .find(|path| !initial_files.contains(path))
        .unwrap();
    let replacement = fs::read_to_string(new_path).unwrap();
    assert!(replacement != original);
    assert!(!String::from_utf8_lossy(&rotated.stdout).contains(&replacement));
    let addr = address();
    let mut restarted = start(&root, addr);
    wait_until_listening(&mut restarted, addr);
    assert!(request(addr, "GET", "/v1/health/ready", Some(&original)).starts_with("HTTP/1.1 401 "));
    assert!(
        request(addr, "GET", "/v1/health/ready", Some(&replacement)).starts_with("HTTP/1.1 200 ")
    );
    restarted.terminate();

    let revoked = auth_command(&root, "revoke-all");
    assert!(revoked.status.success());
    let addr = address();
    let mut revoked_server = start(&root, addr);
    wait_until_listening(&mut revoked_server, addr);
    assert!(
        request(addr, "GET", "/v1/health/ready", Some(&replacement)).starts_with("HTTP/1.1 401 ")
    );
    assert_eq!(token_paths(&root), paths);
    revoked_server.terminate();

    let expired = kiarni_auth::activation::Store::open(&root, None)
        .unwrap()
        .rotate(1, 1, 0)
        .unwrap();
    let expired_token = fs::read_to_string(expired.token_file.unwrap()).unwrap();
    let before_recovery = token_paths(&root);
    let addr = address();
    let mut expired_server = start(&root, addr);
    wait_until_listening(&mut expired_server, addr);
    assert!(
        request(addr, "GET", "/v1/health/ready", Some(&expired_token)).starts_with("HTTP/1.1 401 ")
    );
    assert_eq!(token_paths(&root), before_recovery);
    expired_server.terminate();

    assert!(auth_command(&root, "rotate").status.success());
    let newest = token_paths(&root);
    assert_eq!(newest.len(), 4);
    let recovered = fs::read_to_string(
        newest
            .iter()
            .find(|path| !before_recovery.contains(path))
            .unwrap(),
    )
    .unwrap();
    let addr = address();
    let mut recovered_server = start(&root, addr);
    wait_until_listening(&mut recovered_server, addr);
    assert!(
        request(addr, "GET", "/v1/health/ready", Some(&recovered)).starts_with("HTTP/1.1 200 ")
    );
    assert!(
        request(addr, "GET", "/v1/health/ready", Some(&replacement)).starts_with("HTTP/1.1 401 ")
    );
    recovered_server.terminate();
}

#[test]
fn auth_administration_refuses_missing_incomplete_or_corrupt_installations() {
    let _scenario = SERVER_SCENARIOS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    assert!(!auth_command(&root, "rotate").status.success());
    assert!(!root.exists());
    assert!(!credentials(&root).exists());
    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
    phase(&root, "store_ready");
    assert!(!auth_command(&root, "rotate").status.success());
    assert!(!root.join("auth").exists());
    assert!(!credentials(&root).exists());

    // Restore a fresh, explicitly initialized layout for the corrupt-state case.
    let root = dir.path().join("complete");
    let addr = address();
    let mut server = start(&root, addr);
    wait_until_listening(&mut server, addr);
    server.terminate();
    let active = fs::read(root.join("auth/active.json")).unwrap();
    let manifest = root.join("store/format.json");
    fs::write(
        &manifest,
        br#"{"magic":"kiarni","version":99,"engine":"redb-4.5.0"}"#,
    )
    .unwrap();
    assert!(!auth_command(&root, "rotate").status.success());
    assert_eq!(fs::read(root.join("auth/active.json")).unwrap(), active);
}

#[test]
fn reads_and_queries_remain_consistent_during_concurrent_updates() {
    let _scenario = SERVER_SCENARIOS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    let addr = address();
    let mut server = start(&root, addr);
    wait_until_listening(&mut server, addr);
    let key = token(&root);
    create(addr, &key, "orders");
    insert_document(addr, &key, "orders", r#"{"_id":"doc","v":0}"#);
    let barrier = std::sync::Barrier::new(3);
    thread::scope(|scope| {
        let writer = scope.spawn(|| {
            barrier.wait();
            for value in 1..=20 {
                let response =
                    update_document(addr, &key, "PATCH", "doc", &format!(r#"{{"v":{value}}}"#));
                assert!(response.starts_with("HTTP/1.1 200 "), "{response}");
            }
        });
        let reader = scope.spawn(|| {
            barrier.wait();
            let mut previous = 0;
            for _ in 0..40 {
                let response = request(
                    addr,
                    "GET",
                    "/v1/collections/orders/documents/doc",
                    Some(&key),
                );
                assert!(response.starts_with("HTTP/1.1 200 "), "{response}");
                let body: serde_json::Value =
                    serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
                let value = body["v"].as_u64().unwrap();
                assert!((previous..=20).contains(&value));
                previous = value;
            }
        });
        let querying = scope.spawn(|| {
            barrier.wait();
            for _ in 0..10 {
                let response = query(
                    addr,
                    &key,
                    r#"{"from":"orders","id":"doc","limit":1,"project":["v"]}"#,
                );
                assert!(response.starts_with("HTTP/1.1 200 "), "{response}");
                let body: serde_json::Value =
                    serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
                assert_eq!(body["count"], 1);
                assert!(body["documents"][0]["v"].as_u64().unwrap() <= 20);
            }
        });
        writer.join().unwrap();
        reader.join().unwrap();
        querying.join().unwrap();
    });
    assert!(
        request(
            addr,
            "GET",
            "/v1/collections/orders/documents/doc",
            Some(&key)
        )
        .ends_with(r#"{"_id":"doc","v":20}"#)
    );
    server.terminate();
}

#[test]
fn full_worker_registry_applies_backpressure_until_a_slot_is_released() {
    let _scenario = SERVER_SCENARIOS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    let addr = address();
    let mut server = Server(
        command(&root, addr)
            .args([
                "--max-requests",
                "1",
                "--max-streams",
                "1",
                "--read-timeout-ms",
                "200",
            ])
            .spawn()
            .unwrap(),
    );
    wait_until_listening(&mut server, addr);
    let key = token(&root);
    let mut incomplete = TcpStream::connect(addr).unwrap();
    incomplete
        .write_all(b"GET /v1/health/live HTTP/1.1\r\n")
        .unwrap();
    let response = request(addr, "GET", "/v1/health/ready", Some(&key));
    assert!(response.starts_with("HTTP/1.1 200 "), "{response}");
    drop(incomplete);
    server.terminate();
}
