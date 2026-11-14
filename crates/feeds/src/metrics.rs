//! Fixed-cardinality, process-local feed telemetry. No document or collection labels.
use super::*;
use std::fmt::Write;

#[derive(Default, Clone, Copy)]
pub(crate) struct Counters {
    pub subscriptions: u64,
    pub published: u64,
    pub unobserved: u64,
    pub enqueued: u64,
    pub ended: [u64; 4],
    pub buckets: [u64; 6],
    pub latency_ns: u128,
}
impl Counters {
    pub fn end(&mut self, reason: Terminal) {
        let index = reason as usize - 1;
        self.ended[index] = self.ended[index].saturating_add(1);
    }
    pub fn observe(&mut self, duration: std::time::Duration) {
        let nanos = duration.as_nanos();
        self.latency_ns = self.latency_ns.saturating_add(nanos);
        for (bucket, bound) in self.buckets.iter_mut().zip([
            100_000,
            1_000_000,
            10_000_000,
            100_000_000,
            1_000_000_000,
            u128::MAX,
        ]) {
            if nanos <= bound {
                *bucket = bucket.saturating_add(1);
            }
        }
    }
}
#[derive(Clone)]
pub struct Metrics {
    pub(crate) shared: Arc<Shared>,
    pub(crate) capacity: usize,
}
#[derive(Clone, Copy)]
pub struct Snapshot {
    pub active: usize,
    pub handles: usize,
    pub queued: u64,
    pub queue_capacity: usize,
    pub(crate) counters: Counters,
}
impl Metrics {
    pub fn snapshot(&self) -> Snapshot {
        let state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        Snapshot {
            active: state.senders.len(),
            handles: state.occupied.iter().filter(|occupied| **occupied).count(),
            queued: self
                .shared
                .queued
                .iter()
                .map(|queue| queue.load(Ordering::Relaxed))
                .sum(),
            queue_capacity: self.capacity,
            counters: state.metrics,
        }
    }
}
impl Snapshot {
    pub fn prometheus(&self) -> String {
        let mut text = String::with_capacity(4096);
        for (name, kind, help, value) in [
            (
                "subscriptions_active",
                "gauge",
                "Subscriptions eligible for events.",
                self.active as u64,
            ),
            (
                "subscription_handles",
                "gauge",
                "Handles retained, including terminal subscriptions.",
                self.handles as u64,
            ),
            (
                "queue_events",
                "gauge",
                "Events retained in subscription queues, excluding in-flight events.",
                self.queued,
            ),
            (
                "queue_capacity",
                "gauge",
                "Configured total subscription queue capacity.",
                self.queue_capacity as u64,
            ),
            (
                "subscriptions_total",
                "counter",
                "Subscriptions successfully registered.",
                self.counters.subscriptions,
            ),
            (
                "events_published_total",
                "counter",
                "Confirmed document changes, including unobserved changes.",
                self.counters.published,
            ),
            (
                "events_unobserved_total",
                "counter",
                "Confirmed changes without active collection observers.",
                self.counters.unobserved,
            ),
            (
                "events_enqueued_total",
                "counter",
                "Successful per-subscriber enqueues, not client acknowledgements.",
                self.counters.enqueued,
            ),
        ] {
            let _ = writeln!(
                text,
                "# HELP kiarni_changefeed_{name} {help}\n# TYPE kiarni_changefeed_{name} {kind}\nkiarni_changefeed_{name} {value}"
            );
        }
        text.push_str("# HELP kiarni_changefeed_subscriptions_ended_total First terminal or cancellation reason.\n# TYPE kiarni_changefeed_subscriptions_ended_total counter\n");
        for (reason, value) in ["overflow", "shutdown", "interrupted", "cancelled"]
            .into_iter()
            .zip(self.counters.ended)
        {
            let _ = writeln!(
                text,
                "kiarni_changefeed_subscriptions_ended_total{{reason=\"{reason}\"}} {value}"
            );
        }
        text.push_str("# HELP kiarni_changefeed_commit_to_dispatch_seconds Time from storage confirmation to completed observed fanout attempt, not socket delivery.\n# TYPE kiarni_changefeed_commit_to_dispatch_seconds histogram\n");
        for (bound, value) in ["0.0001", "0.001", "0.01", "0.1", "1", "+Inf"]
            .into_iter()
            .zip(self.counters.buckets)
        {
            let _ = writeln!(
                text,
                "kiarni_changefeed_commit_to_dispatch_seconds_bucket{{le=\"{bound}\"}} {value}"
            );
        }
        let _ = writeln!(
            text,
            "kiarni_changefeed_commit_to_dispatch_seconds_sum {}\nkiarni_changefeed_commit_to_dispatch_seconds_count {}",
            self.counters.latency_ns as f64 / 1e9,
            self.counters.buckets[5]
        );
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn change() -> Change {
        Change::new(
            "private_collection",
            "private_id",
            Kind::Insert,
            None,
            Some(br#"{"_id":"private_id","payload":"secret"}"#),
        )
        .unwrap()
    }
    #[test]
    fn queue_gauges_and_first_terminal_counters_follow_retained_handles() {
        let mut dispatcher = Dispatcher::new(2, 1).unwrap();
        let metrics = dispatcher.metrics();
        let one = dispatcher.subscribe("private_collection").unwrap();
        let two = dispatcher.subscribe("private_collection").unwrap();
        let mut publication = dispatcher.reserve(1).unwrap();
        publication.publish_confirmed(change()).unwrap();
        publication.finish();
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.active, 2);
        assert_eq!(snapshot.queued, 2);
        assert_eq!(snapshot.counters.enqueued, 2);
        one.try_next().unwrap().unwrap();
        assert_eq!(metrics.snapshot().queued, 1);
        let mut publication = dispatcher.reserve(1).unwrap();
        publication.publish_confirmed(change()).unwrap();
        publication.finish();
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.active, 1);
        assert_eq!(snapshot.handles, 2);
        assert_eq!(snapshot.queued, 2);
        assert_eq!(snapshot.counters.ended[0], 1);
        assert!(matches!(two.try_next(), Err(Terminal::Overflow)));
        assert_eq!(metrics.snapshot().queued, 1);
        dispatcher.shutdown();
        one.terminate(Terminal::Interrupted);
        assert_eq!(metrics.snapshot().counters.ended, [1, 1, 0, 0]);
        drop(one);
        drop(two);
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.queued, 0);
        assert_eq!(snapshot.handles, 0);
        assert_eq!(snapshot.counters.ended, [1, 1, 0, 0]);
        let text = snapshot.prometheus();
        assert!(!text.contains("secret"));
        assert!(!text.contains("private"));
        assert!(text.ends_with('\n'));
    }
    #[test]
    fn histogram_buckets_are_cumulative_and_cancellation_is_counted_once() {
        let mut counters = Counters::default();
        counters.observe(std::time::Duration::from_micros(500));
        counters.observe(std::time::Duration::from_millis(20));
        assert_eq!(counters.buckets, [0, 1, 1, 2, 2, 2]);
        assert_eq!(counters.latency_ns, 20_500_000);
        let mut dispatcher = Dispatcher::new(1, 1).unwrap();
        let metrics = dispatcher.metrics();
        let sub = dispatcher.subscribe("orders").unwrap();
        drop(sub);
        dispatcher.shutdown();
        assert_eq!(metrics.snapshot().counters.ended, [0, 0, 0, 1]);
        assert!(
            metrics
                .snapshot()
                .prometheus()
                .contains("commit_to_dispatch_seconds_bucket{le=\"+Inf\"}")
        );
    }
    #[test]
    fn unobserved_changes_are_distinct_and_unused_reservations_do_not_count() {
        let mut dispatcher = Dispatcher::new(1, 1).unwrap();
        let metrics = dispatcher.metrics();
        drop(dispatcher.reserve(1).unwrap());
        assert_eq!(metrics.snapshot().counters.published, 0);
        let mut publication = dispatcher.reserve(1).unwrap();
        publication.advance_unobserved_confirmed("orders").unwrap();
        publication.finish();
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.counters.published, 1);
        assert_eq!(snapshot.counters.unobserved, 1);
        assert_eq!(snapshot.counters.buckets[5], 0);
    }
}
