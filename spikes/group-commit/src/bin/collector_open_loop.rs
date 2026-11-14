//! Uniform open-loop, virtual clients, bounded pending confirmations. No HTTP.
use kiarni_engine::writes::Operation;
use kiarni_group_commit_spike::{
    Outcome, Store,
    collector::{AdmissionError, Collector, Policy, Ticket},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    process::Command,
    time::{Duration, Instant},
};

const CLIENTS: usize = 32;
const MAX_PENDING: usize = 256;

fn document(id: &str, revision: u64) -> Vec<u8> {
    let mut text = format!(r#"{{"_id":"{id}","revision":{revision},"pad":""}}"#);
    text.insert_str(text.len() - 2, &"x".repeat(1024 - text.len()));
    text.into_bytes()
}
fn quantiles(values: &mut [u64]) -> Value {
    values.sort_unstable();
    let mut output = json!({"samples":values.len()});
    for p in [50, 95, 99] {
        output[format!("p{p}_ms")] = values
            .get((values.len() * p).div_ceil(100).saturating_sub(1))
            .map(|value| json!(*value as f64 / 1e6))
            .unwrap_or(Value::Null);
    }
    output
}
fn slot_ns(slot: u64, rate: u64) -> u64 {
    (u128::from(slot) * 1_000_000_000 / u128::from(rate)) as u64
}
fn current_slot(elapsed: Duration, rate: u64) -> u64 {
    (elapsed.as_nanos() * u128::from(rate) / 1_000_000_000) as u64
}
fn sleep_before_poll(remaining: Duration) -> Option<Duration> {
    (remaining > Duration::from_micros(100))
        .then(|| (remaining - Duration::from_micros(50)).min(Duration::from_micros(50)))
}
fn rss() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status.lines().find_map(|line| {
        line.strip_prefix("VmRSS:")
            .and_then(|value| value.split_whitespace().next()?.parse::<u64>().ok())
            .map(|value| value * 1024)
    })
}
fn jain(counts: &[u64]) -> f64 {
    let sum = counts.iter().sum::<u64>() as f64;
    let squares = counts.iter().map(|n| (*n as f64).powi(2)).sum::<f64>();
    if squares == 0.0 {
        0.0
    } else {
        sum * sum / (counts.len() as f64 * squares)
    }
}

struct Pending {
    ticket: Ticket,
    client: usize,
    revision: u64,
    scheduled: Instant,
    started: Instant,
}
struct Samples {
    scheduled: Vec<u64>,
    actual: Vec<u64>,
    lateness: Vec<u64>,
    queue: Vec<u64>,
    rejection: Vec<u64>,
    groups: BTreeSet<u64>,
    confirmed: [u64; CLIENTS],
    full: [u64; CLIENTS],
    skipped: [u64; CLIENTS],
    last: [(u64, u64); CLIENTS],
    failures: u64,
}
impl Default for Samples {
    fn default() -> Self {
        Self {
            scheduled: Vec::new(),
            actual: Vec::new(),
            lateness: Vec::new(),
            queue: Vec::new(),
            rejection: Vec::new(),
            groups: BTreeSet::new(),
            confirmed: [0; CLIENTS],
            full: [0; CLIENTS],
            skipped: [0; CLIENTS],
            last: [(0, 0); CLIENTS],
            failures: 0,
        }
    }
}
fn poll(pending: &mut Vec<Pending>, samples: &mut Samples) {
    let mut index = 0;
    while index < pending.len() {
        match pending[index].ticket.try_receive() {
            Ok(None) => index += 1,
            result => {
                let entry = pending.swap_remove(index);
                match result {
                    Ok(Some(confirmation))
                        if confirmation.committed && confirmation.outcome == Outcome::Changed =>
                    {
                        let now = Instant::now();
                        samples
                            .scheduled
                            .push(now.duration_since(entry.scheduled).as_nanos() as u64);
                        samples
                            .actual
                            .push(now.duration_since(entry.started).as_nanos() as u64);
                        samples.queue.push(confirmation.queue_ns);
                        samples.groups.insert(confirmation.group_sequence);
                        samples.confirmed[entry.client] += 1;
                        if confirmation.sequence > samples.last[entry.client].0 {
                            samples.last[entry.client] = (confirmation.sequence, entry.revision);
                        }
                    }
                    _ => samples.failures += 1,
                }
            }
        }
    }
}

fn trial(rate: u64, window: u64, seconds: u64) -> Result<Value, Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("store");
    let ids: Vec<_> = (0..256).map(|n| format!("d{n:06}")).collect();
    let seed: Vec<_> = ids.iter().map(|id| document(id, 0)).collect();
    let collector = Collector::start(
        Store::create(&root, &seed)?,
        Policy {
            collection_window: Duration::from_micros(window),
            ..Policy::default()
        },
    )
    .map_err(|_| "collector start failed")?;
    let client = collector.client();
    let mut pending = Vec::with_capacity(MAX_PENDING);
    let mut samples = Samples::default();
    let target = rate * seconds;
    let mut slot = 0;
    let mut max_pending = 0;
    let baseline_rss = rss();
    let mut peak_rss = baseline_rss;
    let mut next_sample = Duration::ZERO;
    let mut rss_samples = 0;
    let start = Instant::now();
    while slot < target {
        poll(&mut pending, &mut samples);
        let elapsed = start.elapsed();
        if elapsed >= next_sample {
            if let Some(current) = rss() {
                peak_rss = Some(peak_rss.unwrap_or(0).max(current));
            }
            rss_samples += 1;
            next_sample = elapsed + Duration::from_millis(100);
        }
        let latest = current_slot(elapsed, rate).min(target);
        // Do not catch up missed arrivals in a burst or hide them in latency samples.
        if latest > slot {
            for missed in slot..latest {
                samples.skipped[missed as usize % CLIENTS] += 1;
            }
            slot = latest;
        }
        if slot == target {
            break;
        }
        let due = start + Duration::from_nanos(slot_ns(slot, rate));
        let now = Instant::now();
        if now < due {
            let remaining = due - now;
            if let Some(sleep) = sleep_before_poll(remaining) {
                std::thread::sleep(sleep);
            } else {
                std::hint::spin_loop();
            }
            continue;
        }
        let owner = slot as usize % CLIENTS;
        if pending.len() == MAX_PENDING {
            samples.skipped[owner] += 1;
            slot += 1;
            continue;
        }
        let attempted = Instant::now();
        samples
            .lateness
            .push(attempted.duration_since(due).as_nanos() as u64);
        let revision = slot + 1;
        let body = document(&ids[owner], revision);
        match client.try_submit(&ids[owner], &body, Operation::Replace) {
            Ok(ticket) => pending.push(Pending {
                ticket,
                client: owner,
                revision,
                scheduled: due,
                started: attempted,
            }),
            Err(AdmissionError::Full) => {
                samples.full[owner] += 1;
                samples
                    .rejection
                    .push(attempted.elapsed().as_nanos() as u64);
            }
            Err(_) => {
                return Err("unexpected admission error".into());
            }
        }
        max_pending = max_pending.max(pending.len());
        slot += 1;
    }
    let generator_seconds = start.elapsed().as_secs_f64();
    while !pending.is_empty() {
        poll(&mut pending, &mut samples);
        if !pending.is_empty() {
            std::thread::sleep(Duration::from_micros(50));
        }
    }
    let elapsed = start.elapsed().as_secs_f64();
    if let Some(current) = rss() {
        peak_rss = Some(peak_rss.unwrap_or(0).max(current));
    }
    collector.shutdown().map_err(|_| "worker join failed")?;
    let reopened = Store::open(&root)?;
    for (index, id) in ids.iter().enumerate().take(CLIENTS) {
        let value: Value = serde_json::from_slice(&reopened.get(id)?.ok_or("missing document")?)?;
        if value["revision"] != samples.last[index].1 {
            return Err("reopen verification failed".into());
        }
    }
    let confirmed = samples.scheduled.len();
    let full: u64 = samples.full.iter().sum();
    let skipped: u64 = samples.skipped.iter().sum();
    if confirmed as u64 + full + skipped + samples.failures != target {
        return Err("arrival accounting failed".into());
    }
    let counts = samples.confirmed;
    Ok(
        json!({"target_rate":rate,"window_us":window,"seconds_requested":seconds,"generator_seconds":generator_seconds,
        "elapsed_including_drain":elapsed,"scheduled_arrivals":target,"confirmed":confirmed,"full":full,
        "generator_skipped":skipped,"execution_failures":samples.failures,"realized_attempts_per_second":(target-skipped) as f64/generator_seconds,
        "confirmed_per_second":confirmed as f64/elapsed,"max_pending_tickets":max_pending,
        "scheduled_latency":quantiles(&mut samples.scheduled),"actual_latency":quantiles(&mut samples.actual),
        "generator_lateness":quantiles(&mut samples.lateness),"queue_and_collection":quantiles(&mut samples.queue),
        "rejection_latency":quantiles(&mut samples.rejection),"mean_operations_per_group":confirmed as f64/samples.groups.len().max(1) as f64,
        "confirmed_per_client":counts.to_vec(),"full_per_client":samples.full.to_vec(),"skipped_per_client":samples.skipped.to_vec(),
        "jain_confirmed":jain(&counts),"zero_confirmation_clients":counts.iter().filter(|n| **n==0).count(),
        "rss_baseline_bytes":baseline_rss,"rss_sampled_peak_bytes":peak_rss,"rss_samples":rss_samples,"reopen_verified":true}),
    )
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut seconds = 10;
    let mut repetitions = 3;
    let mut rate = None;
    let mut window = 0;
    let mut args = std::env::args().skip(1);
    while let Some(key) = args.next() {
        let value: u64 = args.next().ok_or("missing value")?.parse()?;
        match key.as_str() {
            "--seconds" => seconds = value,
            "--repetitions" => repetitions = value,
            "--rate" => rate = Some(value),
            "--window" => window = value,
            _ => return Err("unknown argument".into()),
        }
    }
    if !(1..=60).contains(&seconds) || !(1..=5).contains(&repetitions) || window > 1000 {
        return Err("invalid arguments".into());
    }
    if let Some(rate) = rate {
        if !(1..=20000).contains(&rate) || rate * seconds > 1_000_000 {
            return Err("sample budget exceeded".into());
        }
        println!("{}", serde_json::to_string(&trial(rate, window, seconds)?)?);
        return Ok(());
    }
    let mut trials = Vec::new();
    for repetition in 0..repetitions {
        for rate in [200, 5000, 15000] {
            for offset in 0..2 {
                let window = if (repetition + offset) % 2 == 0 {
                    0
                } else {
                    1000
                };
                // A fresh process isolates allocator history and RSS between trials.
                let output = Command::new(std::env::current_exe()?)
                    .args([
                        "--rate",
                        &rate.to_string(),
                        "--window",
                        &window.to_string(),
                        "--seconds",
                        &seconds.to_string(),
                    ])
                    .output()?;
                if !output.status.success() {
                    return Err("measurement subprocess failed".into());
                }
                let mut measured: Value = serde_json::from_slice(&output.stdout)?;
                measured["repetition"] = json!(repetition + 1);
                eprintln!(
                    "rep={} rate={rate} window={window} confirmed={} full={} skipped={}",
                    repetition + 1,
                    measured["confirmed"],
                    measured["full"],
                    measured["generator_skipped"]
                );
                trials.push(measured);
            }
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"experiment":"uniform open-loop virtual clients; fresh process per trial; no HTTP",
        "clients":CLIENTS,"max_pending_tickets":MAX_PENDING,"documents":256,"document_bytes":1024,"cache_bytes":8388608,
        "max_outstanding":64,"max_owned_bytes":8388608,"max_group_operations":32,"max_group_bytes":8388608,
        "durability":"Immediate","warmup":0,"rss_sample_interval_ms":100,"seconds_per_trial":seconds,"repetitions":repetitions,"trials":trials})
        )?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn schedule_quantiles_and_fairness() {
        assert_eq!(slot_ns(3, 200), 15_000_000);
        assert_eq!(current_slot(Duration::from_millis(17), 200), 3);
        assert_eq!(quantiles(&mut [])["p99_ms"], Value::Null);
        assert_eq!(
            quantiles(&mut [3_000_000, 1_000_000, 2_000_000])["p95_ms"],
            3.0
        );
        assert_eq!(jain(&[10, 10]), 1.0);
        assert_eq!(jain(&[10, 0]), 0.5);
        assert_eq!(jain(&[0, 0]), 0.0);
        assert_eq!(document("d000000", 1_000_000).len(), 1024);
        assert_eq!(
            sleep_before_poll(Duration::from_millis(5)),
            Some(Duration::from_micros(50))
        );
        assert_eq!(sleep_before_poll(Duration::from_micros(90)), None);
    }
    #[test]
    fn open_loop_accounts_for_every_arrival_and_verifies_reopen() {
        let measured = trial(200, 0, 1).unwrap();
        let count = |field: &str| measured[field].as_u64().unwrap();
        assert_eq!(count("scheduled_arrivals"), 200);
        assert_eq!(
            count("confirmed")
                + count("full")
                + count("generator_skipped")
                + count("execution_failures"),
            200
        );
        assert_eq!(count("execution_failures"), 0);
        assert!(count("max_pending_tickets") <= MAX_PENDING as u64);
        assert_eq!(measured["reopen_verified"], true);
    }
}
