use super::*;

fn probe(root: &Path, addr: SocketAddr) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_kiarni"));
    cmd.env_clear()
        .args(["healthcheck", "--data-dir"])
        .arg(root)
        .args(["--bind", &addr.to_string()]);
    cmd
}

fn failed(output: std::process::Output) {
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(output.stderr, b"kiarni: healthcheck_failed\n");
}

#[test]
fn healthcheck_is_read_only_private_bounded_authenticated_and_respects_public_host() {
    let _scenario = SERVER_SCENARIOS.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    let addr = address();
    failed(probe(&root, addr).output().unwrap());
    assert!(!root.exists());
    let mut server = Server(
        command(&root, addr)
            .args(["--public-origin", "https://db.example"])
            .spawn()
            .unwrap(),
    );
    wait_until_listening(&mut server, addr);
    let credential = fs::read_dir(credentials(&root))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let key = fs::read(&credential).unwrap();
    let before: Vec<_> = fs::read_dir(root.join("auth"))
        .unwrap()
        .map(|v| v.unwrap().file_name())
        .collect();
    failed(probe(&root, addr).output().unwrap()); // public Host required
    let output = probe(&root, addr)
        .args(["--public-origin", "https://db.example"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stdout.is_empty() && output.stderr.is_empty());
    assert_eq!(fs::read(&credential).unwrap(), key);
    let after: Vec<_> = fs::read_dir(root.join("auth"))
        .unwrap()
        .map(|v| v.unwrap().file_name())
        .collect();
    assert_eq!(before, after);
    assert!(
        probe(&root, addr)
            .args(["--public-origin", "https://db.example", "--token-file"])
            .arg(&credential)
            .status()
            .unwrap()
            .success()
    );
    fs::set_permissions(&credential, fs::Permissions::from_mode(0o644)).unwrap();
    failed(
        probe(&root, addr)
            .args(["--public-origin", "https://db.example"])
            .output()
            .unwrap(),
    );
    fs::set_permissions(&credential, fs::Permissions::from_mode(0o600)).unwrap();
    let linked = dir.path().join("linked");
    std::os::unix::fs::symlink(&credential, &linked).unwrap();
    failed(
        probe(&root, addr)
            .arg("--token-file")
            .arg(&linked)
            .output()
            .unwrap(),
    );
    let hardlink = dir.path().join("hardlink");
    fs::hard_link(&credential, &hardlink).unwrap();
    failed(probe(&root, addr).output().unwrap());
    fs::remove_file(hardlink).unwrap();
    fs::write(&credential, b"kiarni_v1_secret_must_not_escape").unwrap();
    failed(probe(&root, addr).output().unwrap());
    fs::write(&credential, &key).unwrap();
    for n in 0..64 {
        fs::write(credentials(&root).join(format!("extra-{n}")), b"").unwrap();
    }
    failed(probe(&root, addr).output().unwrap());
    assert!(
        probe(&root, addr)
            .args(["--public-origin", "https://db.example", "--token-file"])
            .arg(&credential)
            .status()
            .unwrap()
            .success()
    );
    server.terminate();
    failed(
        probe(&root, addr)
            .arg("--token-file")
            .arg(&credential)
            .output()
            .unwrap(),
    );
}

#[test]
fn healthcheck_rejects_partial_status_and_bounds_a_stalled_peer() {
    let _scenario = SERVER_SCENARIOS.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    let addr = address();
    let mut server = start(&root, addr);
    wait_until_listening(&mut server, addr);
    let key = token(&root);
    server.terminate();
    assert_eq!(key.len(), 91);
    for stall in [false, true] {
        let listener = TcpListener::bind(addr).unwrap();
        thread::scope(|scope| {
            scope.spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0; 1024];
                assert!(stream.read(&mut request).unwrap() > 0);
                if stall {
                    thread::sleep(Duration::from_millis(3300));
                } else {
                    stream.write_all(b"HTTP/1.1 200 ").unwrap();
                }
            });
            let start = Instant::now();
            failed(probe(&root, addr).output().unwrap());
            assert!(start.elapsed() < Duration::from_secs(4));
        });
    }
}
