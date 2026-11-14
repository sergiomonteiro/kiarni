//! Closed-loop, in-process collector experiment. No HTTP or feeds.
use kiarni_engine::writes::Operation;
use kiarni_group_commit_spike::{
    Outcome, Store,
    collector::{AdmissionError, Collector, Policy},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    sync::{Arc, Barrier},
    thread,
    time::{Duration, Instant},
};

fn document(id: &str, revision: u64) -> Vec<u8> {
    let mut text = format!(r#"{{"_id":"{id}","revision":{revision},"pad":""}}"#);
    text.insert_str(text.len() - 2, &"x".repeat(1024 - text.len()));
    text.into_bytes()
}

fn percentiles(values: &mut [u64]) -> Value {
    values.sort_unstable();
    let mut result = serde_json::Map::new();
    result.insert("samples".into(), json!(values.len()));
    for percentile in [50, 95, 99] {
        let value = if values.is_empty() {
            Value::Null
        } else {
            json!(values[(values.len() * percentile).div_ceil(100) - 1] as f64 / 1e6)
        };
        result.insert(format!("p{percentile}_ms"), value);
    }
    Value::Object(result)
}

#[derive(Default)]
struct Worker {
    success: Vec<u64>,
    full: Vec<u64>,
    queue: Vec<u64>,
    service: Vec<u64>,
    groups: BTreeSet<u64>,
    group_size_sum: usize,
    last_revision: u64,
    failures: usize,
}

fn trial(
    clients: usize,
    idle: bool,
    max_group: usize,
    window_us: u64,
    seconds: u64,
) -> Result<Value, Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("store");
    let seed: Vec<_> = (0..256)
        .map(|index| document(&format!("d{index:06}"), 0))
        .collect();
    let store = Store::create(&root, &seed)?;
    let collector = Collector::start(
        store,
        Policy {
            max_group_operations: max_group,
            collection_window: Duration::from_micros(window_us),
            ..Policy::default()
        },
    )
    .map_err(|_| "collector start failed")?;
    let barrier = Arc::new(Barrier::new(clients + 1));
    let (workers, elapsed) = thread::scope(|scope| -> Result<_, String> {
        let mut handles = Vec::new();
        for index in 0..clients {
            let client = collector.client();
            let barrier = Arc::clone(&barrier);
            handles.push(scope.spawn(move || -> Result<Worker, String> {
                let id = format!("d{index:06}");
                let mut result = Worker::default();
                barrier.wait();
                let start = Instant::now();
                let mut revision = 0;
                while start.elapsed() < Duration::from_secs(seconds) {
                    revision += 1;
                    if revision > 100_000 {
                        return Err("sample budget exceeded".into());
                    }
                    let attempt = Instant::now();
                    let body = document(&id, revision);
                    match client.try_submit(&id, &body, Operation::Replace) {
                        Ok(ticket) => match ticket.wait() {
                            Ok(confirmed)
                                if confirmed.committed && confirmed.outcome == Outcome::Changed =>
                            {
                                result.success.push(attempt.elapsed().as_nanos() as u64);
                                result.queue.push(confirmed.queue_ns);
                                result.service.push(confirmed.service_ns);
                                result.groups.insert(confirmed.group_sequence);
                                result.group_size_sum += confirmed.group_size;
                                result.last_revision = revision;
                            }
                            _ => {
                                result.failures += 1;
                                break;
                            }
                        },
                        Err(AdmissionError::Full) => {
                            result.full.push(attempt.elapsed().as_nanos() as u64);
                            // No retry of this revision. Avoid an unbounded busy loop on rejection.
                            thread::sleep(Duration::from_micros(100));
                        }
                        Err(_) => {
                            result.failures += 1;
                            break;
                        }
                    }
                    if idle {
                        thread::sleep(Duration::from_millis(5));
                    }
                }
                Ok(result)
            }));
        }
        let start = Instant::now();
        barrier.wait();
        let results = handles
            .into_iter()
            .map(|handle| handle.join().map_err(|_| "worker panic".to_owned())?)
            .collect::<Result<Vec<_>, _>>()?;
        Ok((results, start.elapsed().as_secs_f64()))
    })?;
    collector.shutdown().map_err(|_| "collector join failed")?;
    let reopened = Store::open(&root)?;
    let mut success = Vec::new();
    let mut full = Vec::new();
    let mut queue = Vec::new();
    let mut service = Vec::new();
    let mut groups = BTreeSet::new();
    let mut group_size_sum = 0;
    let mut failures = 0;
    for (index, mut worker) in workers.into_iter().enumerate() {
        let bytes = reopened
            .get(&format!("d{index:06}"))?
            .ok_or("missing document")?;
        let value: Value = serde_json::from_slice(&bytes)?;
        if value["revision"] != worker.last_revision {
            return Err("reopen verification failed".into());
        }
        success.append(&mut worker.success);
        full.append(&mut worker.full);
        queue.append(&mut worker.queue);
        service.append(&mut worker.service);
        groups.extend(worker.groups);
        group_size_sum += worker.group_size_sum;
        failures += worker.failures;
    }
    let confirmed = success.len();
    Ok(
        json!({"clients":clients,"load":if idle {"think_5ms"} else {"saturated_closed_loop"},
        "max_group_operations":max_group,"window_us":window_us,"seconds":elapsed,
        "confirmed":confirmed,"full":full.len(),"execution_failures":failures,
        "confirmed_per_second":confirmed as f64/elapsed,"groups":groups.len(),
        "mean_operations_per_group":confirmed as f64/groups.len().max(1) as f64,
        "operation_weighted_group_size":group_size_sum as f64/confirmed.max(1) as f64,
        "latency":percentiles(&mut success),"rejection_latency":percentiles(&mut full),
        "queue_and_collection":percentiles(&mut queue),"service":percentiles(&mut service),"reopen_verified":true}),
    )
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut seconds = 2;
    let mut repetitions = 3;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let value: u64 = args.next().ok_or("missing argument value")?.parse()?;
        match arg.as_str() {
            "--seconds" => seconds = value,
            "--repetitions" => repetitions = value,
            _ => return Err("unknown argument".into()),
        }
    }
    if !(1..=30).contains(&seconds) || !(1..=10).contains(&repetitions) {
        return Err("invalid duration or repetitions".into());
    }
    let policies = [(1, 0), (32, 0), (32, 50), (32, 200), (32, 1000)];
    let loads = [(1, false), (8, false), (32, false), (128, false), (1, true)];
    let mut trials = Vec::new();
    for repetition in 0..repetitions {
        for &(clients, idle) in &loads {
            for offset in 0..policies.len() {
                let (max_group, window) = policies[(offset + repetition as usize) % policies.len()];
                let mut result = trial(clients, idle, max_group, window, seconds)?;
                result["repetition"] = json!(repetition + 1);
                eprintln!(
                    "clients={clients} window={window} group={max_group} rep={} confirmed/s={:.1}",
                    repetition + 1,
                    result["confirmed_per_second"].as_f64().unwrap_or_default()
                );
                trials.push(result);
            }
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"experiment":"in-process closed-loop collector; no HTTP or feeds",
        "documents":256,"document_bytes":1024,"cache_bytes":8388608,"durability":"Immediate",
        "max_outstanding":64,"max_owned_bytes":8388608,"max_group_bytes":8388608,
        "rejection_backoff_us":100,"warmup":0,"seconds_per_trial":seconds,"repetitions":repetitions,"trials":trials})
        )?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nearest_rank_and_exact_document_size() {
        assert_eq!(percentiles(&mut [])["p95_ms"], Value::Null);
        let mut values = [3_000_000, 1_000_000, 2_000_000];
        assert_eq!(percentiles(&mut values)["p50_ms"], 2.0);
        assert_eq!(percentiles(&mut values)["p95_ms"], 3.0);
        let bytes = document("d000000", 100_000);
        assert_eq!(bytes.len(), 1024);
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["revision"], 100_000);
    }
}
