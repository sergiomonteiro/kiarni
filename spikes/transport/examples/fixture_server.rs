//! Private stdin driver for browser tests; not the kiarni binary or a remote API.
#![cfg(unix)]
use kiarni_auth_spike::{Keyset, vault::Vault};
use kiarni_transport_spike::{Server, now};
use std::os::unix::fs::PermissionsExt;
use std::{
    fs,
    io::{self, BufRead, Read, Write},
    process::ExitCode,
    time::Duration,
};

fn run() -> Result<(), ()> {
    let data = tempfile::tempdir().map_err(|_| ())?;
    let client = tempfile::tempdir().map_err(|_| ())?;
    fs::set_permissions(data.path(), fs::Permissions::from_mode(0o700)).map_err(|_| ())?;
    fs::set_permissions(client.path(), fs::Permissions::from_mode(0o700)).map_err(|_| ())?;
    let (keys, token) = Keyset::bootstrap(now().map_err(|_| ())?, 300).map_err(|_| ())?;
    let delivered = Vault::open(data.path())
        .map_err(|_| ())?
        .provision(&keys, &token, &Vault::open(client.path()).map_err(|_| ())?)
        .map_err(|_| ())?;
    drop(token);
    let token_file = client.path().join(delivered.token_file.ok_or(())?);
    let mut server = Server::start(
        &data.path().join("fixture.redb"),
        keys,
        2,
        Duration::from_millis(500),
    )
    .map_err(|_| ())?;
    println!(
        "{}",
        serde_json::json!({"url":format!("http://{}", server.address()),"token_file":token_file})
    );
    io::stdout().flush().map_err(|_| ())?;
    let mut input = io::stdin().lock();
    loop {
        let mut line = Vec::new();
        let count = input
            .by_ref()
            .take(4097)
            .read_until(b'\n', &mut line)
            .map_err(|_| ())?;
        if count == 0 {
            break;
        }
        if count > 4096 {
            return Err(());
        }
        let command: serde_json::Value = serde_json::from_slice(&line).map_err(|_| ())?;
        let response = match command["op"].as_str() {
            Some("insert") => {
                let id = command["id"].as_str().ok_or(())?;
                let size = command["payload_bytes"].as_u64().unwrap_or(0);
                if size > 900000 {
                    return Err(());
                }
                let id = serde_json::to_string(id).map_err(|_| ())?;
                let json = format!(
                    "{{\"_id\":{id},\"status\":\"new\",\"note\":\"ação\",\"total\":123456789012345678901234567890,\"payload\":\"{}\"}}",
                    "x".repeat(size as usize)
                );
                match server.insert(json.as_bytes()) {
                    Ok(sequence) => serde_json::json!({"ok":true,"sequence":sequence}),
                    Err(_) => serde_json::json!({"ok":false,"code":"fixture_insert_failed"}),
                }
            }
            Some("stats") => server.counters().snapshot(),
            Some("revoke") => serde_json::json!({"ok":server.revoke().is_ok()}),
            Some("shutdown") => {
                server.shutdown();
                println!(
                    "{}",
                    serde_json::json!({"ok":true,"stats":server.counters().snapshot()})
                );
                io::stdout().flush().map_err(|_| ())?;
                break;
            }
            _ => return Err(()),
        };
        println!("{response}");
        io::stdout().flush().map_err(|_| ())?;
    }
    // Keep private temp directories alive until all socket workers have stopped.
    server.shutdown();
    let _ = fs::metadata(token_file).map_err(|_| ())?;
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(()) => {
            eprintln!("transport_fixture_failed");
            ExitCode::FAILURE
        }
    }
}
