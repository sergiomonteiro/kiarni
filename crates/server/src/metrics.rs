//! Fixed-cardinality HTTP completion counters, independent of log transport.
use std::{
    fmt::Write,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
pub(crate) const RESERVE_BYTES: u64 = 4096;
const BOUNDS_US: [u64; 7] = [
    1_000,
    10_000,
    100_000,
    1_000_000,
    5_000_000,
    30_000_000,
    u64::MAX,
];
const LABELS: [&str; 7] = ["0.001", "0.01", "0.1", "1", "5", "30", "+Inf"];
#[derive(Clone, Copy)]
pub(crate) enum Outcome {
    Written,
    WriteFailed,
    StreamClosed,
}
#[derive(Default)]
pub(crate) struct Histogram {
    buckets: [AtomicU64; 7],
    sum_us: AtomicU64,
}
#[derive(Default)]
pub(crate) struct Metrics {
    status: [AtomicU64; 5],
    transport: [AtomicU64; 3],
    duration: [Histogram; 2],
}
pub(crate) fn add(counter: &AtomicU64, amount: u64) {
    let _ = counter.try_update(Ordering::Release, Ordering::Relaxed, |value| {
        Some(value.saturating_add(amount))
    });
}
pub(crate) fn value(counter: &AtomicU64) -> u64 {
    counter.load(Ordering::Acquire)
}
impl Histogram {
    pub(crate) fn observe(&self, elapsed: Duration) {
        // A high bucket includes every completion visible in lower buckets.
        for index in (0..BOUNDS_US.len()).rev() {
            if index == 6 || elapsed.as_nanos() <= u128::from(BOUNDS_US[index]) * 1000 {
                add(&self.buckets[index], 1);
            }
        }
        add(
            &self.sum_us,
            u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX),
        );
    }
    pub(crate) fn write_to(
        &self,
        text: &mut String,
        family: &'static str,
        dimension: &'static str,
        label: &'static str,
    ) {
        // Read low to high, and use the same +Inf observation for count.
        let buckets = self.buckets.each_ref().map(value);
        for (bound, count) in LABELS.iter().zip(buckets) {
            let _ = writeln!(
                text,
                "{family}_bucket{{{dimension}=\"{label}\",le=\"{bound}\"}} {count}"
            );
        }
        let _ = writeln!(
            text,
            "{family}_sum{{{dimension}=\"{label}\"}} {}\n{family}_count{{{dimension}=\"{label}\"}} {}",
            value(&self.sum_us) as f64 / 1e6,
            buckets[6]
        );
    }
}
impl Metrics {
    pub fn observe(&self, status: u16, elapsed: Duration, outcome: Outcome) {
        let class = (status / 100).clamp(1, 5) as usize - 1;
        add(&self.status[class], 1);
        let transport = match outcome {
            Outcome::Written => 0,
            Outcome::WriteFailed => 1,
            Outcome::StreamClosed => 2,
        };
        add(&self.transport[transport], 1);
        let histogram = &self.duration[usize::from(matches!(outcome, Outcome::StreamClosed))];
        histogram.observe(elapsed);
    }
    pub fn prometheus(&self) -> String {
        let mut text = String::with_capacity(4096);
        text.push_str("# HELP kiarni_http_requests_total Completed handlers by intended status class, not delivery confirmation.\n# TYPE kiarni_http_requests_total counter\n");
        for (index, counter) in self.status.iter().enumerate() {
            let _ = writeln!(
                text,
                "kiarni_http_requests_total{{status_class=\"{}xx\"}} {}",
                index + 1,
                value(counter)
            );
        }
        text.push_str("# HELP kiarni_http_transport_total Completed local transport outcomes, not client acknowledgements.\n# TYPE kiarni_http_transport_total counter\n");
        for (label, counter) in ["written", "write_failed", "stream_closed"]
            .iter()
            .zip(&self.transport)
        {
            let _ = writeln!(
                text,
                "kiarni_http_transport_total{{outcome=\"{label}\"}} {}",
                value(counter)
            );
        }
        text.push_str("# HELP kiarni_http_request_duration_seconds Handler elapsed time; stream includes full connection lifetime.\n# TYPE kiarni_http_request_duration_seconds histogram\n");
        for (kind, histogram) in ["response", "stream"].iter().zip(&self.duration) {
            histogram.write_to(
                &mut text,
                "kiarni_http_request_duration_seconds",
                "kind",
                kind,
            );
        }
        text
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn errors_transport_and_stream_lifetime_are_distinct() {
        let metrics = Metrics::default();
        metrics.observe(200, Duration::from_micros(500), Outcome::Written);
        metrics.observe(401, Duration::from_millis(20), Outcome::Written);
        metrics.observe(503, Duration::from_secs(2), Outcome::WriteFailed);
        metrics.observe(200, Duration::from_secs(60), Outcome::StreamClosed);
        assert_eq!(metrics.status.each_ref().map(value), [0, 2, 0, 1, 1]);
        assert_eq!(metrics.transport.each_ref().map(value), [2, 1, 1]);
        assert_eq!(
            metrics.duration[0].buckets.each_ref().map(value),
            [1, 1, 2, 2, 3, 3, 3]
        );
        assert_eq!(
            metrics.duration[1].buckets.each_ref().map(value),
            [0, 0, 0, 0, 0, 0, 1]
        );
        assert_eq!(value(&metrics.duration[0].sum_us), 2_020_500);
        let text = metrics.prometheus();
        assert!(text.contains("_count{kind=\"stream\"} 1"));
        assert!(text.contains("_bucket{kind=\"response\",le=\"0.001\"} 1"));
        assert!(text.contains("_bucket{kind=\"stream\",le=\"+Inf\"} 1"));
        assert!(std::mem::size_of::<Metrics>() < RESERVE_BYTES as usize);
    }
    #[test]
    fn counters_saturate_and_concurrent_scrapes_keep_cumulative_buckets() {
        let metrics = Metrics::default();
        metrics.status[1].store(u64::MAX, Ordering::Relaxed);
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    for _ in 0..1000 {
                        metrics.observe(200, Duration::from_micros(500), Outcome::Written);
                    }
                });
            }
            for _ in 0..1000 {
                let buckets = metrics.duration[0].buckets.each_ref().map(value);
                assert!(buckets.windows(2).all(|pair| pair[0] <= pair[1]));
            }
        });
        assert_eq!(value(&metrics.status[1]), u64::MAX);
        assert_eq!(value(&metrics.duration[0].buckets[6]), 4000);
    }
}
