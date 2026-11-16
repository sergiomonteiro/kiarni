//! Process-failure injection for the isolated RF-04 experiment only.
use kiarni_storage_spike::secondary::{Checkpoint, Mutation, Store};
use std::{
    env,
    io::{self, Read, Write},
    process::ExitCode,
};
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.len() != 3 {
        return Err("invalid_worker_arguments".into());
    }
    let operation = args[1].to_str().ok_or("invalid_operation")?;
    let point = match args[2].to_str() {
        Some("document") => Checkpoint::Document,
        Some("old_index") => Checkpoint::OldIndexRemoved,
        Some("new_index") => Checkpoint::NewIndexWritten,
        Some("before_commit") => Checkpoint::BeforeCommit,
        Some("committed") => Checkpoint::Committed,
        _ => return Err("invalid_checkpoint".into()),
    };
    let store = Store::open(args[0].as_ref())?;
    let change = match operation {
        "patch" => Mutation::Patch(br#"{"status":"paid"}"#),
        "delete" => Mutation::Delete,
        _ => return Err("invalid_operation".into()),
    };
    store.mutate_with_hook("orders", "target", change, &mut |current| {
        if current == point {
            println!("READY");
            io::stdout()
                .flush()
                .map_err(|_| kiarni_storage_spike::secondary::Error::Injected)?;
            let mut byte = [0];
            io::stdin()
                .read_exact(&mut byte)
                .map_err(|_| kiarni_storage_spike::secondary::Error::Injected)?;
        }
        Ok(())
    })?;
    Ok(())
}
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => {
            eprintln!("secondary_fault_worker_failed");
            ExitCode::FAILURE
        }
    }
}
