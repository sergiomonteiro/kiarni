use kiarni_engine::writes::Operation;
use kiarni_group_commit_spike::{Mutation, Store};
use serde_json::json;
use std::time::Instant;

fn document(id: &str, revision: usize) -> Vec<u8> {
    let mut value = format!(r#"{{"_id":"{id}","revision":{revision},"pad":""}}"#);
    let padding = "x".repeat(1024 - value.len());
    value.insert_str(value.len() - 2, &padding);
    value.into_bytes()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut trials = Vec::new();
    for repetition in 1..=3 {
        for group_size in [1, 8, 32] {
            let temp = tempfile::tempdir()?;
            let root = temp.path().join("store");
            let ids: Vec<_> = (0..256).map(|index| format!("d{index:06}")).collect();
            let seed: Vec<_> = ids.iter().map(|id| document(id, 0)).collect();
            let store = Store::create(&root, &seed)?;
            let inputs: Vec<_> = (0..4096)
                .map(|index| document(&ids[index % 256], index / 256 + 1))
                .collect();
            let groups: Vec<Vec<_>> = inputs
                .chunks(group_size)
                .enumerate()
                .map(|(group, bodies)| {
                    bodies
                        .iter()
                        .enumerate()
                        .map(|(offset, body)| Mutation {
                            id: &ids[(group * group_size + offset) % 256],
                            body,
                            operation: Operation::Replace,
                        })
                        .collect()
                })
                .collect();
            let mut durations = Vec::new();
            let mut commit_ns = 0;
            let start = Instant::now();
            for group in &groups {
                let receipt = store.apply(group)?;
                if !receipt.committed
                    || receipt
                        .outcomes
                        .iter()
                        .any(|outcome| *outcome != kiarni_group_commit_spike::Outcome::Changed)
                {
                    return Err("unexpected outcome".into());
                }
                durations.push(receipt.timings.total_ns);
                commit_ns += receipt.timings.commit_ns;
            }
            let seconds = start.elapsed().as_secs_f64();
            drop(store);
            let reopened = Store::open(&root)?;
            for id in &ids {
                let bytes = reopened.get(id)?.ok_or("missing document")?;
                let value: serde_json::Value = serde_json::from_slice(&bytes)?;
                if value["revision"] != 16 {
                    return Err("verification failed".into());
                }
            }
            durations.sort_unstable();
            trials.push(json!({"repetition":repetition,"group_size":group_size,"operations":4096,
                "commits":groups.len(),"seconds":seconds,"confirmed_operations_per_second":4096.0/seconds,
                "group_p95_ms":durations[(durations.len()*95).div_ceil(100)-1] as f64/1e6,
                "commit_ns":commit_ns,"reopen_verified":true}));
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "experiment":"preformed groups; no HTTP, collection window or feeds",
            "durability":"Immediate","documents":256,"document_bytes":1024,"cache_bytes":8388608,
            "trials":trials
        }))?
    );
    Ok(())
}
