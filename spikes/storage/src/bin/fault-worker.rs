//! Subprocess used only by crash/lock tests. READY is a synchronization marker.
use std::{
    env,
    io::{self, Read, Write},
    path::PathBuf,
    process::ExitCode,
};

use kiarni_storage_spike::{DOCUMENTS, STATUS, Store};
use redb::Durability;

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args_os().skip(1);
    let mode = args.next().ok_or("mode required")?;
    let path = PathBuf::from(args.next().ok_or("path required")?);
    let store = Store::open(&path)?;
    let mut write = store.db.begin_write()?;
    write.set_durability(Durability::Immediate)?;
    write.open_table(DOCUMENTS)?.insert(
        ("orders", "pending"),
        br#"{"_id":"pending","status":"new"}"#.as_slice(),
    )?;
    if mode == "both" || mode == "committed" {
        write
            .open_table(STATUS)?
            .insert(("orders", "new", "pending"), ())?;
    }
    if mode == "committed" {
        write.commit()?;
    } else if mode != "document" && mode != "both" {
        return Err("invalid mode".into());
    }
    println!("READY");
    io::stdout().flush()?;
    // Parent kills us while handles are still live; no destructors run.
    let mut byte = [0];
    io::stdin().read_exact(&mut byte)?;
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => {
            eprintln!("fault_worker_failed");
            ExitCode::FAILURE
        }
    }
}
