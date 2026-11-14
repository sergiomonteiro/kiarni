//! Opt-in aggregate phase measurements. Payloads and identifiers are excluded.
use std::{
    marker::PhantomData,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

use rustix::time::{ClockId, clock_gettime};
use serde::Serialize;

#[derive(Default)]
pub struct Metric {
    calls: AtomicU64,
    wall_ns: AtomicU64,
    thread_cpu_ns: AtomicU64,
}

#[derive(Serialize)]
pub struct Snapshot {
    pub calls: u64,
    pub wall_ns: u64,
    pub thread_cpu_ns: u64,
}

pub struct Timer<'a> {
    metric: &'a Metric,
    wall: Instant,
    cpu: u64,
    _thread_bound: PhantomData<Rc<()>>,
}

fn cpu_ns() -> u64 {
    let time = clock_gettime(ClockId::ThreadCPUTime);
    (time.tv_sec as u64)
        .saturating_mul(1_000_000_000)
        .saturating_add(time.tv_nsec as u64)
}

impl Metric {
    pub const fn new() -> Self {
        Self {
            calls: AtomicU64::new(0),
            wall_ns: AtomicU64::new(0),
            thread_cpu_ns: AtomicU64::new(0),
        }
    }

    pub fn start(&self) -> Timer<'_> {
        Timer {
            metric: self,
            wall: Instant::now(),
            cpu: cpu_ns(),
            _thread_bound: PhantomData,
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            calls: self.calls.load(Ordering::Relaxed),
            wall_ns: self.wall_ns.load(Ordering::Relaxed),
            thread_cpu_ns: self.thread_cpu_ns.load(Ordering::Relaxed),
        }
    }
}

impl Drop for Timer<'_> {
    fn drop(&mut self) {
        let cpu = cpu_ns().saturating_sub(self.cpu);
        self.metric
            .wall_ns
            .fetch_add(self.wall.elapsed().as_nanos() as u64, Ordering::Relaxed);
        self.metric.thread_cpu_ns.fetch_add(cpu, Ordering::Relaxed);
        self.metric.calls.fetch_add(1, Ordering::Relaxed);
    }
}

pub static BEGIN: Metric = Metric::new();
pub static TRANSFORM: Metric = Metric::new();
pub static LOOKUP: Metric = Metric::new();
pub static STAGE: Metric = Metric::new();
pub static COMMIT: Metric = Metric::new();
