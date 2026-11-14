//! Subprocess fixture; opens a known format, commits, and waits to be killed.
use std::io::{self, Read, Write};
use std::path::PathBuf;

use kiarni_format_spike::Store;

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("missing root")?;
    let store = Store::open(&root)?;
    let collection = store.create_collection("committed")?;
    store.insert_fixture(&collection, br#"{"_id":"doc","n":1e10001}"#)?;
    println!("COMMITTED");
    io::stdout().flush()?;
    let mut byte = [0];
    io::stdin().read_exact(&mut byte)?;
    Ok(())
}

fn main() {
    if run().is_err() {
        eprintln!("format_worker_failed");
        std::process::exit(1);
    }
}
