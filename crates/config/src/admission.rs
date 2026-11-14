use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use crate::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionError {
    Busy,
    StreamLimit,
}

struct State {
    counts: AtomicU64,
    requests: u64,
    streams: u64,
}

/// One atomic observation of admission occupancy, not unique clients or TCP sockets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdmissionSnapshot {
    pub admitted: u64,
    pub streaming: u64,
    pub limit: u64,
    pub stream_limit: u64,
}

#[derive(Clone)]
pub struct Admission(Arc<State>);

/// Occupies a general slot and optionally a stream slot until dropped.
pub struct Permit {
    state: Arc<State>,
    delta: u64,
}

impl Admission {
    pub fn new(config: &Config) -> Self {
        Self(Arc::new(State {
            counts: AtomicU64::new(0),
            requests: config.max_requests(),
            streams: config.max_streams(),
        }))
    }

    pub fn snapshot(&self) -> AdmissionSnapshot {
        let counts = self.0.counts.load(Ordering::Relaxed);
        AdmissionSnapshot {
            admitted: counts & 0xffff_ffff,
            streaming: counts >> 32,
            limit: self.0.requests,
            stream_limit: self.0.streams,
        }
    }

    /// Bounded attempts, no wait queue or allocation on refusal.
    pub fn acquire(&self, stream: bool) -> Result<Permit, AdmissionError> {
        let delta = 1 + if stream { 1 << 32 } else { 0 };
        let mut counts = self.0.counts.load(Ordering::Relaxed);
        for _ in 0..32 {
            if stream && counts >> 32 >= self.0.streams {
                return Err(AdmissionError::StreamLimit);
            }
            if counts & 0xffff_ffff >= self.0.requests {
                return Err(AdmissionError::Busy);
            }
            match self.0.counts.compare_exchange_weak(
                counts,
                counts + delta,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    return Ok(Permit {
                        state: self.0.clone(),
                        delta,
                    });
                }
                Err(actual) => counts = actual,
            }
        }
        Err(AdmissionError::Busy)
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        self.state.counts.fetch_sub(self.delta, Ordering::Relaxed);
    }
}

impl Permit {
    /// Upgrade an already admitted connection after authentication/routing,
    /// retaining its general slot even if the stream limit refuses promotion.
    pub fn promote_to_stream(&mut self) -> Result<(), AdmissionError> {
        if self.delta >> 32 != 0 {
            return Ok(());
        }
        let mut counts = self.state.counts.load(Ordering::Relaxed);
        for _ in 0..32 {
            if counts >> 32 >= self.state.streams {
                return Err(AdmissionError::StreamLimit);
            }
            match self.state.counts.compare_exchange_weak(
                counts,
                counts + (1 << 32),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    self.delta += 1 << 32;
                    return Ok(());
                }
                Err(actual) => counts = actual,
            }
        }
        Err(AdmissionError::Busy)
    }
}
