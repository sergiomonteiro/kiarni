//! Explicit buffer planning for the HTTP composition, not an RSS ceiling.
use crate::writer::WORKER_STACK_BYTES;
use kiarni_config::{BUFFER_PLAN_CAP, Config};

const LOG_RESERVE: u64 = (crate::logging::STACK_BYTES + 16 * 1024) as u64;
#[cfg(feature = "profiling")]
const PROFILE_RESERVE: u64 =
    (crate::profiling::FLUSH_STACK_BYTES + crate::profiling::FLUSH_BUFFER_BYTES) as u64;
#[cfg(not(feature = "profiling"))]
const PROFILE_RESERVE: u64 = 0;
const MIB: u64 = 1024 * 1024;
pub fn buffer_plan_bytes(config: &Config) -> Result<u64, &'static str> {
    if config.max_result_bytes() < MIB + 1024 {
        return Err("config_result_below_format_limit");
    }
    let requests = config.max_requests();
    let streams = config.max_streams() * (2 * config.feed_queue() * MIB + 3 * MIB);
    let legacy = requests * (4 * MIB + config.max_result_bytes() + 2 * config.max_query_bytes())
        + streams
        + 8 * MIB
        + 32 * MIB;
    // No wire-body copy. Charge request metadata and bounded coordinator overhead,
    // including the dedicated stack, explicitly within the former reservation.
    let plan = requests
        * (3 * MIB + config.max_result_bytes() + 2 * config.max_query_bytes() + 1024)
        + streams
        + 8 * MIB
        + 32 * MIB
        + WORKER_STACK_BYTES as u64
        + 64 * 1024
        + LOG_RESERVE
        + PROFILE_RESERVE
        + crate::metrics::RESERVE_BYTES;
    if legacy > BUFFER_PLAN_CAP || plan > legacy {
        return Err("config_buffer_plan_exceeded");
    }
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn new_overhead_fits_saved_copy_even_with_a_single_request() {
        for requests in 1..=128 {
            assert!(
                WORKER_STACK_BYTES as u64
                    + 64 * 1024
                    + LOG_RESERVE
                    + PROFILE_RESERVE
                    + crate::metrics::RESERVE_BYTES
                    + requests * 1024
                    <= requests * MIB
            );
            for result in [MIB + 1024, 4 * MIB] {
                let requests = requests.to_string();
                let result = result.to_string();
                if let Ok(config) = kiarni_config::resolve(
                    &[
                        ("--max-requests", &requests),
                        ("--max-streams", "1"),
                        ("--feed-queue", "1"),
                        ("--max-result-bytes", &result),
                    ],
                    &[],
                    None,
                ) {
                    let legacy = config.max_requests()
                        * (4 * MIB + config.max_result_bytes() + 2 * config.max_query_bytes())
                        + config.max_streams() * (2 * config.feed_queue() * MIB + 3 * MIB)
                        + 8 * MIB
                        + 32 * MIB;
                    if legacy <= BUFFER_PLAN_CAP {
                        assert!(buffer_plan_bytes(&config).unwrap() <= legacy);
                    }
                }
            }
        }
    }
}
