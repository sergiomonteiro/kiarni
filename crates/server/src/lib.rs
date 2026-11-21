//! Local HTTP server lifecycle and request handlers.
#![cfg(unix)]
mod backup;
pub use backup::create_backup;
mod bootstrap;
mod buffers;
pub use buffers::buffer_plan_bytes;
mod collections;
mod dashboard;
mod documents;
mod gate;
mod healthcheck;
mod http;
pub use healthcheck::healthcheck;
mod logging;
mod metrics;
#[cfg(feature = "profiling")]
mod profiling;
mod query_metrics;
mod recent_errors;
mod restore;
pub use restore::restore_backup;
pub mod writer;

use kiarni_auth::Authority;
use kiarni_config::Config;
use std::{
    net::TcpListener,
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy)]
pub enum AuthCommand {
    Rotate,
    RevokeAll,
}

pub struct AuthResult {
    pub epoch: u64,
    pub token_file: Option<PathBuf>,
}

/// Experimental offline Rust entrypoint for a new format-2 installation or its
/// bootstrap recovery. Never migrates format 1 or starts HTTP. CLI still uses 1.
/// Credentials are delivered through the existing private auth journal; only
/// their path is returned. Store closes before the installation lock is released.
pub fn initialize_indexed_installation(
    config: Config,
) -> Result<AuthResult, Box<dyn std::error::Error>> {
    let root = bootstrap::Root::lock(config.data_dir().as_ref())?;
    let (store, startup) = root.start_indexed(now()?)?;
    let result = AuthResult {
        epoch: startup.keyset.epoch(),
        token_file: startup.token_file,
    };
    drop(store);
    Ok(result)
}

pub fn manage_auth(
    config: Config,
    command: AuthCommand,
) -> Result<AuthResult, Box<dyn std::error::Error>> {
    let root = bootstrap::Root::lock_existing(config.data_dir().as_ref())?;
    let startup = root.update_auth(now()?, matches!(command, AuthCommand::Rotate))?;
    Ok(AuthResult {
        epoch: startup.keyset.epoch(),
        token_file: startup.token_file,
    })
}

fn now() -> Result<u64, &'static str> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|time| time.as_secs())
        .map_err(|_| "clock_unavailable")
}

pub fn serve(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    buffer_plan_bytes(&config)?;
    let stop = Arc::new(AtomicBool::new(false));
    for signal in [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM] {
        signal_hook::flag::register(signal, Arc::clone(&stop))
            .map_err(|_| "signal_setup_failed")?;
    }
    let root = bootstrap::Root::lock(config.data_dir().as_ref())?;
    let (store, startup) = root.start(now()?)?;
    if let Some(path) = startup.token_file {
        println!("Credencial inicial em {path:?}");
    }
    let listener = TcpListener::bind(config.bind()).map_err(|_| "bind_failed")?;
    let authority = Authority::new(startup.keyset, now()?);
    let result = http::serve(listener, &config, authority, store, stop);
    #[cfg(feature = "profiling")]
    profiling::emit();
    result?;
    Ok(())
}
