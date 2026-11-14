//! Private M0-F2 CLI/fault worker. Not the product's kiarni serve.
#![cfg(unix)]
use kiarni_auth_spike::{
    Error,
    activation::{Checkpoint, DEFAULT_LIFETIME, Store},
};
use std::{
    env,
    io::{self, Read, Write},
    path::PathBuf,
    process::ExitCode,
};

fn run() -> Result<(), Error> {
    let mut args = env::args_os().skip(1);
    let mode = args.next().ok_or(Error::InvalidConfig)?;
    let mut data = None;
    let mut delivery = None;
    let mut stop = None;
    while let Some(arg) = args.next() {
        let value = args.next().ok_or(Error::InvalidConfig)?;
        if arg == "--data-dir" {
            data = Some(PathBuf::from(value));
        } else if arg == "--delivery-dir" {
            delivery = Some(PathBuf::from(value));
        } else if arg == "--stop-at" {
            stop = Some(value);
        } else {
            return Err(Error::InvalidConfig);
        }
    }
    let data = data.ok_or(Error::InvalidConfig)?;
    let store = Store::open(&data, delivery.as_deref())?;
    let mut hook = |point: Checkpoint| {
        if stop
            .as_ref()
            .is_some_and(|name| name == format!("{point:?}").as_str())
        {
            println!("READY");
            io::stdout().flush().map_err(|_| Error::FileAccess)?;
            let mut byte = [0];
            io::stdin()
                .read_exact(&mut byte)
                .map_err(|_| Error::FileAccess)?;
        }
        Ok(())
    };
    let now = 1000; // Deterministic fixture clock, never a product default.
    let startup = if mode == "serve" {
        store.bootstrap_with_hook(now, &mut hook)?
    } else if mode == "rotate" {
        store.update_with_hook(Some((now, DEFAULT_LIFETIME, 0)), &mut hook)?
    } else if mode == "revoke" {
        store.update_with_hook(None, &mut hook)?
    } else {
        return Err(Error::InvalidConfig);
    };
    let token_file = startup
        .token_file
        .as_ref()
        .map(|path| path.to_str().ok_or(Error::CommitUncertain))
        .transpose()?;
    println!(
        "{}",
        serde_json::json!({"epoch":startup.keyset.epoch(), "created":startup.created,"recovered":startup.recovered,"token_file":token_file})
    );
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{}", error.code());
            ExitCode::FAILURE
        }
    }
}
