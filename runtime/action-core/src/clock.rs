//! `CLOCK_MONOTONIC` helpers. All timestamps in the system use this
//! clock; wall time is only recorded once per session for display.

use std::time::{Duration, Instant};

/// Monotonic time in nanoseconds since an arbitrary point.
pub fn now_ns() -> u64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let start = START.get_or_init(Instant::now);
    start.elapsed().as_nanos() as u64
}

/// Monotonic time as a `Duration` since the process-local epoch.
pub fn now() -> Duration {
    Duration::from_nanos(now_ns())
}
