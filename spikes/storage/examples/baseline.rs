//! Small reproducible baseline, not a product throughput target.
use std::{error::Error, fs, time::Instant};

use kiarni_core::{DEFAULT_MAX_DOCUMENT_BYTES, DocumentInput};
use kiarni_storage_spike::Store;

fn main() -> Result<(), Box<dyn Error>> {
    const COUNT: usize = 256;
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("baseline.redb");
    let store = Store::open(&path)?;
    let empty_size = fs::metadata(&path)?.len();
    let mut durations = Vec::with_capacity(COUNT);
    for i in 0..COUNT {
        let json = format!(
            r#"{{"_id":"id{i:04}","status":"paid","payload":"{}"}}"#,
            "x".repeat(1024)
        );
        let candidate =
            DocumentInput::parse(json.as_bytes(), DEFAULT_MAX_DOCUMENT_BYTES)?.prepare_insert()?;
        let start = Instant::now();
        store.insert("orders", &candidate)?;
        durations.push(start.elapsed().as_micros());
    }
    let full_size = fs::metadata(&path)?.len();
    durations.sort_unstable();
    let start = Instant::now();
    for i in 0..COUNT {
        if store.get("orders", &format!("id{i:04}"))?.is_none() {
            return Err("missing inserted document".into());
        }
    }
    let read_us = start.elapsed().as_micros();
    for i in 0..COUNT / 2 {
        store.delete("orders", &format!("id{i:04}"))?;
    }
    let deleted_size = fs::metadata(&path)?.len();
    // One additional read pass after normal restart.
    drop(store);
    let start = Instant::now();
    let reopened = Store::open(&path)?;
    let reopen_us = start.elapsed().as_micros();
    if reopened.get("orders", "id0255")?.is_none() {
        return Err("missing document after restart".into());
    }
    println!("redb=4.5.0 profile=release cache_bytes=8388608 durability=Immediate");
    println!("dataset={COUNT} documents; 1024 ASCII payload bytes + id/status/JSON overhead");
    println!(
        "writers=1 secondary_indexes=1 transaction_size=1 document; input prepared outside timer"
    );
    println!(
        "insert_us p50={} p95={} p99={}",
        durations[COUNT / 2],
        durations[COUNT * 95 / 100],
        durations[COUNT * 99 / 100]
    );
    println!("point_reads={COUNT} total_us={read_us} warm_cache=true");
    println!("file_bytes empty={empty_size} inserted={full_size} after_delete_half={deleted_size}");
    println!("normal_reopen_us={reopen_us}");
    Ok(())
}
