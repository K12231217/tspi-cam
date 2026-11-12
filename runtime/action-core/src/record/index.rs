//! `index.json`: the final metadata file of each recording. Written via a
//! temporary file + atomic rename, so an interrupted recording is never
//! mistaken for a complete one. See architecture §8.

use serde::Serialize;

/// Final metadata of one recording. `complete` is the only marker of a
/// complete recording.
#[derive(Debug, Serialize)]
pub struct RecordingIndex {
    pub complete: bool,
    pub started_at_wall: String,
    pub started_mono_ns: u64,
    pub ended_mono_ns: u64,
    pub frames_captured: u64,
    pub frames_dropped: u64,
    pub frames_written: u64,
    pub imu_sampled: u64,
    pub imu_dropped: u64,
    pub imu_dropped_max_consecutive: u64,
    pub imu_dropped_max_per_10s: u64,
    pub imu_written: u64,
    pub fps_actual: f64,
    pub imu_rate_actual_hz: f64,
    pub imu_interval_jitter_std_ms: f64,
    pub write_errors: u64,
    pub timestamp_semantics: String,
    pub clock_offset_ns: Option<i64>,
    pub errors: Vec<String>,
    pub app_version: String,
}
