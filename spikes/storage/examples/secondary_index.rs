//! Paired RF-04 experiment, not a server benchmark or product index implementation.
use kiarni_engine::query::Query;
use kiarni_storage_spike::secondary::{Mutation, Store};
use serde_json::{Value, json};
use std::{env, error::Error, fs, hint::black_box, path::Path, time::Instant};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
fn timing<F: FnOnce() -> Result<()>>(operation: F) -> Result<u64> {
    let start = Instant::now();
    operation()?;
    Ok(start.elapsed().as_nanos().min(u64::MAX as u128) as u64)
}
fn stats(samples: &[u64]) -> Value {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let rank = |p: usize| sorted[(sorted.len() * p).div_ceil(100) - 1];
    json!({"operations":samples.len(),"total_ns":samples.iter().sum::<u64>(),"p50_ns":rank(50),"p95_ns":rank(95),"p99_ns":rank(99)})
}
fn run(
    path: &Path,
    indexed: bool,
    documents: &[Vec<u8>],
    query: &Query,
    reads: usize,
) -> Result<(Value, Vec<Vec<u8>>)> {
    let store = Store::create(path, indexed)?;
    let empty = fs::metadata(path)?.len();
    let mut insert = Vec::new();
    for (i, document) in documents.iter().enumerate() {
        let id = format!("id{i:05}");
        insert.push(timing(|| {
            assert!(store.mutate("orders", &id, Mutation::Insert(document))?);
            Ok(())
        })?);
    }
    let inserted = fs::metadata(path)?.len();
    assert_eq!(store.snapshot()?.audit()?, documents.len());
    let reference = store.query(query, true)?.documents;
    let selected = store.query(query, false)?;
    assert_eq!(selected.documents, reference);
    for _ in 0..3 {
        black_box(store.query(query, false)?);
    }
    let mut query_ns = Vec::new();
    for _ in 0..reads {
        let start = Instant::now();
        let output = store.query(query, false)?;
        let ns = start.elapsed().as_nanos().min(u64::MAX as u128) as u64;
        assert_eq!(output.documents, reference);
        black_box(output);
        query_ns.push(ns);
    }
    let mutations = documents.len().min(128);
    let mut indexed_update = Vec::new();
    for i in 0..mutations {
        let id = format!("id{i:05}");
        indexed_update.push(timing(|| {
            assert!(store.mutate("orders", &id, Mutation::Patch(br#"{"status":"changed"}"#))?);
            Ok(())
        })?);
    }
    assert_eq!(store.snapshot()?.audit()?, documents.len());
    assert_eq!(
        store.query(query, false)?.documents,
        store.query(query, true)?.documents
    );
    let mut other_update = Vec::new();
    for i in 0..mutations {
        let id = format!("id{i:05}");
        other_update.push(timing(|| {
            assert!(store.mutate("orders", &id, Mutation::Patch(br#"{"touch":1}"#))?);
            Ok(())
        })?);
    }
    let mut delete = Vec::new();
    for i in 0..mutations {
        let id = format!("id{i:05}");
        delete.push(timing(|| {
            assert!(store.mutate("orders", &id, Mutation::Delete)?);
            Ok(())
        })?);
    }
    assert_eq!(store.snapshot()?.audit()?, documents.len() - mutations);
    let final_documents = store.query(query, false)?.documents;
    assert_eq!(final_documents, store.query(query, true)?.documents);
    let deleted = fs::metadata(path)?.len();
    drop(store);
    let start = Instant::now();
    let reopened = Store::open(path)?;
    let reopen_ns = start.elapsed().as_nanos();
    assert_eq!(reopened.snapshot()?.audit()?, documents.len() - mutations);
    assert_eq!(reopened.query(query, false)?.documents, final_documents);
    Ok((
        json!({"indexed":indexed,"insert":stats(&insert),"query":stats(&query_ns),"indexed_field_update":stats(&indexed_update),"other_field_update":stats(&other_update),"delete":stats(&delete),"query_plan":selected.plan,"query_examined_documents":selected.examined,"query_document_bytes":selected.document_bytes,"returned_documents":reference.len(),"file_logical_bytes":{"empty":empty,"inserted":inserted,"after_delete":deleted},"reopen_ns":reopen_ns}),
        final_documents,
    ))
}
fn main() -> Result<()> {
    let mut repetitions = 3;
    let mut reads = 30;
    let mut args = env::args().skip(1);
    while let Some(key) = args.next() {
        let value: usize = args.next().ok_or("missing value")?.parse()?;
        match key.as_str() {
            "--repetitions" if (1..=5).contains(&value) => repetitions = value,
            "--queries" if (10..=200).contains(&value) => reads = value,
            _ => return Err("invalid benchmark option".into()),
        }
    }
    let directory = tempfile::tempdir()?;
    let mut results = Vec::new();
    for (count, cardinality) in [(256, 16), (4096, 1), (4096, 16), (4096, 128)] {
        let input:Vec<Vec<u8>>=(0..count).map(|i|serde_json::to_vec(&json!({"_id":format!("id{i:05}"),"status":format!("s{:03}",i%cardinality),"n":i,"total":9007199254740993u64,"payload":"p".repeat(512)}))).collect::<std::result::Result<_,_>>()?;
        let query=Query::parse(br#"{"from":"orders","filter":{"op":"eq","field":"status","value":"s000"},"sort":[{"field":"n","direction":"desc"}],"limit":25,"project":["status","n","total"]}"#)?;
        for repetition in 0..repetitions {
            let order = if repetition % 2 == 0 {
                [false, true]
            } else {
                [true, false]
            };
            let mut final_reference = None;
            for indexed in order {
                let path = directory
                    .path()
                    .join(format!("{count}-{cardinality}-{repetition}-{indexed}.redb"));
                let (measurements, final_documents) = run(&path, indexed, &input, &query, reads)?;
                if let Some(reference) = &final_reference {
                    assert_eq!(&final_documents, reference);
                } else {
                    final_reference = Some(final_documents);
                }
                results.push(json!({"documents":count,"status_cardinality":cardinality,"repetition":repetition,"input_json_bytes":input.iter().map(Vec::len).sum::<usize>(),"measurements":measurements}));
            }
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"schema":1,"redb":"4.5.0","cache_bytes_per_store":8388608,"durability":"Immediate","writers":1,"transactions":"one document per commit","query_samples_per_run":reads,"warmup_queries":3,"repetitions":repetitions,"payload_ascii_bytes":512,"index":"partial decoded status string <=128 UTF-8 bytes; eq only","notes":"prebuilt input and validated AST outside timers; document validation, transaction, index maintenance and engine evaluation inside operation timers; result assertions/drop outside query timer; warm caches, no HTTP/group commit/cold-cache or power-loss claim","runs":results})
        )?
    );
    Ok(())
}
