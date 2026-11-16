//! The single time base. Every timestamp in the system is [`MonoTime`]
//! (`CLOCK_MONOTONIC` nanoseconds); driver timestamps are converted through
//! [`Timebase::from_driver`] using one offset measured at startup and recorded
//! as `index.json.clock_offset_ns`.
//!
//! The five distinct capture timestamps and offline alignment live in
//! `timebase.md`; this module only pins the interface and gives a monotonic
//! implementation plus a controllable mock for host tests.

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::Instant;

use crate::types::{MonoTime, TsSource};

pub trait Timebase: Send + Sync {
    fn now(&self) -> MonoTime;
    /// Driver timestamp -> monotonic time (offset applied here, once).
    // `from_driver(&self, ..)` is the reviewed interface name in
    // docs/design/interfaces.md §2; clippy's from_* heuristic does not apply.
    #[allow(clippy::wrong_self_convention)]
    fn from_driver(&self, raw: i64, src: TsSource) -> MonoTime;
    /// Reported in `index.json.timestamp_semantics`; `Unknown` until measured.
    fn semantics(&self) -> TsSource;
}

/// Monotonic clock backed by `Instant`, offset for driver timestamps.
#[derive(Debug)]
pub struct SystemTimebase {
    start: Instant,
    offset_ns: AtomicI64,
    semantics: TsSource,
}

impl SystemTimebase {
    /// `offset_ns` is the median of `driver_raw - monotonic_ns` measured over a
    /// startup burst; `semantics` stays `Unknown` until H13 is closed.
    pub fn new(offset_ns: i64, semantics: TsSource) -> Self {
        Self {
            start: Instant::now(),
            offset_ns: AtomicI64::new(offset_ns),
            semantics,
        }
    }

    pub fn set_offset_ns(&self, offset_ns: i64) {
        self.offset_ns.store(offset_ns, Ordering::Relaxed);
    }

    pub fn offset_ns(&self) -> i64 {
        self.offset_ns.load(Ordering::Relaxed)
    }
}

impl Default for SystemTimebase {
    fn default() -> Self {
        Self::new(0, TsSource::Unknown)
    }
}

impl Timebase for SystemTimebase {
    fn now(&self) -> MonoTime {
        MonoTime(self.start.elapsed().as_nanos() as u64)
    }

    fn from_driver(&self, raw: i64, _src: TsSource) -> MonoTime {
        let converted = raw.saturating_add(self.offset_ns.load(Ordering::Relaxed));
        MonoTime(converted.max(0) as u64)
    }

    fn semantics(&self) -> TsSource {
        self.semantics
    }
}

/// Deterministic clock for host tests: time only moves when asked.
#[derive(Debug)]
pub struct MockTimebase {
    now_ns: AtomicU64,
    offset_ns: i64,
    semantics: TsSource,
}

impl MockTimebase {
    pub fn new(start_ns: u64, offset_ns: i64, semantics: TsSource) -> Self {
        Self {
            now_ns: AtomicU64::new(start_ns),
            offset_ns,
            semantics,
        }
    }

    pub fn advance(&self, delta_ns: u64) {
        self.now_ns.fetch_add(delta_ns, Ordering::Relaxed);
    }
}

impl Timebase for MockTimebase {
    fn now(&self) -> MonoTime {
        MonoTime(self.now_ns.load(Ordering::Relaxed))
    }

    fn from_driver(&self, raw: i64, _src: TsSource) -> MonoTime {
        MonoTime((raw + self.offset_ns).max(0) as u64)
    }

    fn semantics(&self) -> TsSource {
        self.semantics
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_advances_only_on_request() {
        let tb = MockTimebase::new(1_000, 0, TsSource::Sof);
        assert_eq!(tb.now(), MonoTime(1_000));
        tb.advance(500);
        assert_eq!(tb.now(), MonoTime(1_500));
    }

    #[test]
    fn driver_offset_is_applied_once_and_clamped() {
        let tb = MockTimebase::new(0, -200, TsSource::Unknown);
        assert_eq!(tb.from_driver(1_000, TsSource::Eof), MonoTime(800));
        assert_eq!(tb.from_driver(-500, TsSource::Eof), MonoTime(0));
    }

    #[test]
    fn semantics_default_to_unknown_until_measured() {
        assert_eq!(SystemTimebase::default().semantics(), TsSource::Unknown);
        let tb = SystemTimebase::new(42, TsSource::Sof);
        assert_eq!(tb.semantics(), TsSource::Sof);
        assert_eq!(tb.offset_ns(), 42);
        assert!(tb.now().as_nanos() < u64::MAX / 2);
    }
}
