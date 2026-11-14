//! Fixed query-processing telemetry; no AST or document strings retained.
use super::metrics::{Histogram, add, value};
use kiarni_http::Code;
use std::{
    fmt::Write,
    sync::atomic::{AtomicBool, AtomicU64},
    time::{Duration, Instant},
};
const OUTCOMES: [&str; 10] = [
    "success",
    "rejected",
    "busy",
    "scan_limit",
    "result_limit",
    "not_found",
    "storage_error",
    "evaluation_error",
    "interrupted",
    "internal_error",
];
#[derive(Default)]
pub(crate) struct Metrics {
    completed: [AtomicU64; 10],
    duration: [Histogram; 3],
    access: [AtomicU64; 2],
    documents: AtomicU64,
    bytes: AtomicU64,
}
#[derive(Clone, Copy)]
pub(crate) enum Access {
    Id,
    Scan,
}
pub(crate) struct Trace {
    started: Instant,
    pub wait: Option<Duration>,
    pub execution: Option<Duration>,
    pub access: Option<Access>,
    pub scan_limit: bool,
    pub documents: u64,
    pub bytes: u64,
}
impl Trace {
    pub fn start() -> Self {
        Self {
            started: Instant::now(),
            wait: None,
            execution: None,
            access: None,
            scan_limit: false,
            documents: 0,
            bytes: 0,
        }
    }
    pub fn admit<'a>(
        &mut self,
        gate: &'a super::gate::Gate,
        stop: &AtomicBool,
    ) -> Result<super::gate::Permit<'a>, Code> {
        let started = Instant::now();
        let result = gate.enter(stop, Code::ServiceUnavailable);
        self.wait = Some(started.elapsed());
        result
    }
}
impl Metrics {
    pub fn finish(&self, trace: Trace, error: Option<Code>) {
        let outcome = match error {
            None => 0,
            Some(Code::ShuttingDown) => 8,
            Some(Code::ServiceUnavailable) if trace.wait.is_some() && trace.execution.is_none() => {
                2
            }
            Some(_) if trace.scan_limit => 3,
            Some(Code::ResultTooLarge) => 4,
            Some(Code::CollectionNotFound) => 5,
            Some(Code::StorageUnavailable) => 6,
            Some(Code::UnsupportedNumber | Code::UnsupportedSortValue) => 7,
            Some(Code::InternalError) => 9,
            Some(_) => 1,
        };
        add(&self.completed[outcome], 1);
        self.duration[0].observe(trace.started.elapsed());
        if let Some(wait) = trace.wait {
            self.duration[1].observe(wait);
        }
        if let Some(execution) = trace.execution {
            self.duration[2].observe(execution);
        }
        if let Some(access) = trace.access {
            add(
                &self.access[match access {
                    Access::Id => 0,
                    Access::Scan => 1,
                }],
                1,
            );
        }
        if error.is_none() {
            add(&self.documents, trace.documents);
            add(&self.bytes, trace.bytes);
        }
    }
    pub fn prometheus(&self) -> String {
        let mut text = String::with_capacity(4096);
        text.push_str("# HELP kiarni_query_completed_total Query processing outcomes, not socket delivery.\n# TYPE kiarni_query_completed_total counter\n");
        for (outcome, counter) in OUTCOMES.iter().zip(&self.completed) {
            let _ = writeln!(
                text,
                "kiarni_query_completed_total{{outcome=\"{outcome}\"}} {}",
                value(counter)
            );
        }
        text.push_str("# HELP kiarni_query_duration_seconds Query processing stages, excluding socket writes.\n# TYPE kiarni_query_duration_seconds histogram\n");
        for (stage, histogram) in ["total", "wait", "execution"].iter().zip(&self.duration) {
            histogram.write_to(&mut text, "kiarni_query_duration_seconds", "stage", stage);
        }
        text.push_str("# HELP kiarni_query_execution_total Selected query access paths, not examined documents.\n# TYPE kiarni_query_execution_total counter\n");
        for (access, counter) in ["id", "scan"].iter().zip(&self.access) {
            let _ = writeln!(
                text,
                "kiarni_query_execution_total{{access=\"{access}\"}} {}",
                value(counter)
            );
        }
        for (name, help, counter) in [
            (
                "documents_returned",
                "Documents in successfully built query results.",
                &self.documents,
            ),
            (
                "result_bytes",
                "JSON envelope bytes in successfully built query results.",
                &self.bytes,
            ),
        ] {
            let _ = writeln!(
                text,
                "# HELP kiarni_query_{name}_total {help}\n# TYPE kiarni_query_{name}_total counter\nkiarni_query_{name}_total {}",
                value(counter)
            );
        }
        text
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn outcomes_stages_and_success_only_payload_counts_are_distinct() {
        let metrics = Metrics::default();
        let mut success = Trace::start();
        success.wait = Some(Duration::from_millis(1));
        success.execution = Some(Duration::from_millis(2));
        success.access = Some(Access::Id);
        success.documents = 2;
        success.bytes = 100;
        metrics.finish(success, None);
        metrics.finish(Trace::start(), Some(Code::InvalidQuery));
        let mut limited = Trace::start();
        limited.execution = Some(Duration::from_millis(3));
        limited.access = Some(Access::Scan);
        limited.scan_limit = true;
        limited.documents = 10;
        limited.bytes = 999;
        metrics.finish(limited, Some(Code::QueryTooComplex));
        assert_eq!(value(&metrics.completed[0]), 1);
        assert_eq!(value(&metrics.completed[1]), 1);
        assert_eq!(value(&metrics.completed[3]), 1);
        assert_eq!(value(&metrics.documents), 2);
        assert_eq!(value(&metrics.bytes), 100);
        let text = metrics.prometheus();
        assert!(text.contains("_count{stage=\"total\"} 3"));
        assert!(text.contains("_count{stage=\"wait\"} 1"));
        assert!(text.contains("_count{stage=\"execution\"} 2"));
        assert!(
            std::mem::size_of::<Metrics>()
                + std::mem::size_of::<super::super::metrics::Metrics>()
                + std::mem::size_of::<Trace>()
                + 2 * std::mem::size_of::<super::super::recent_errors::History>()
                + kiarni_storage::TELEMETRY_STATE_BYTES
                + std::mem::size_of::<std::fs::File>()
                < super::super::metrics::RESERVE_BYTES as usize
        );
    }
    #[test]
    fn busy_and_shutdown_are_observed_on_real_gate_without_execution() {
        let metrics = Metrics::default();
        let gate = super::super::gate::Gate::default();
        let stop = AtomicBool::new(false);
        let _held = gate.enter(&stop, Code::ServiceUnavailable).unwrap();
        let mut trace = Trace::start();
        let error = trace.admit(&gate, &stop).err();
        assert_eq!(error, Some(Code::ServiceUnavailable));
        metrics.finish(trace, error);
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let mut trace = Trace::start();
        let error = trace.admit(&gate, &stop).err();
        metrics.finish(trace, error);
        assert_eq!(value(&metrics.completed[2]), 1);
        assert_eq!(value(&metrics.completed[8]), 1);
        assert!(
            metrics
                .prometheus()
                .contains("_count{stage=\"execution\"} 0")
        );
    }
    #[test]
    fn infrastructure_and_result_failures_never_add_success_payload_counters() {
        let metrics = Metrics::default();
        for code in [
            Code::StorageUnavailable,
            Code::InternalError,
            Code::ResultTooLarge,
        ] {
            let mut trace = Trace::start();
            trace.documents = 10;
            trace.bytes = 100;
            metrics.finish(trace, Some(code));
        }
        assert_eq!(value(&metrics.completed[6]), 1);
        assert_eq!(value(&metrics.completed[9]), 1);
        assert_eq!(value(&metrics.completed[4]), 1);
        assert_eq!(value(&metrics.documents), 0);
        assert_eq!(value(&metrics.bytes), 0);
    }
}
