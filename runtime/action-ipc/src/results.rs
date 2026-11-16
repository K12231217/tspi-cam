//! Typed results; the transport carries them as `serde_json::Value`, the
//! contract layer keeps them typed so server and client agree. Field set per
//! `docs/design/ipc.md` §3.2 and `record-format.md` §2.

use serde::{Deserialize, Serialize};

use crate::error::ErrorClass;
use crate::params::{Codec, RecId};

/// The only three states exposed over IPC (`starting` is internal and
/// `record.start` returns only after the device is ready).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum State {
    #[serde(rename = "idle")]
    Idle,
    #[serde(rename = "recording")]
    Recording,
    #[serde(rename = "stopping")]
    Stopping,
}

/// Drop counters for status/progress: two sources, reported separately.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dropped {
    /// `v4l2_buffer.sequence` gaps (driver side).
    pub gap_frames: u64,
    /// Application queue overflow.
    pub queue_dropped: u64,
}

/// Drop counters in `index.json`, which also tracks IMU samples.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexDropped {
    pub gap_frames: u64,
    pub queue_dropped: u64,
    pub imu_dropped: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimestampSemantics {
    #[serde(rename = "sof")]
    Sof,
    #[serde(rename = "eof")]
    Eof,
    #[serde(rename = "dequeue")]
    Dequeue,
    #[serde(rename = "unknown")]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FrameFormat {
    #[serde(rename = "nv12")]
    Nv12,
}

/// One entry in `index.json.errors`; same source as `record.event`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordError {
    pub class: ErrorClass,
    pub code: i32,
    pub message: String,
    pub at_mono_ns: u64,
}

/// Complete record descriptor, returned by `record.stop/get/list`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexSummary {
    pub rec_id: RecId,
    /// The only integrity flag: missing or `false` means the record was not
    /// finished (power loss mid-recording).
    pub complete: bool,
    pub started_mono_ns: u64,
    pub ended_mono_ns: u64,
    pub duration_ms: u64,
    pub frames: u64,
    pub imu_samples: u64,
    pub dropped: IndexDropped,
    pub bytes: u64,
    pub codec: Codec,
    pub width: u32,
    pub height: u32,
    pub timestamp_semantics: TimestampSemantics,
    /// Driver clock vs `CLOCK_MONOTONIC` offset; omitted when unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clock_offset_ns: Option<i64>,
    pub fps_actual: f64,
    pub imu_rate_actual_hz: f64,
    pub errors: Vec<RecordError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CameraInfo {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EncodeInfo {
    pub codec: Codec,
    pub bitrate_kbps: u32,
}

/// `record.status`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Status {
    pub state: State,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rec_id: Option<RecId>,
    pub elapsed_ms: u64,
    pub frames: u64,
    pub imu_samples: u64,
    pub dropped: Dropped,
    pub disk_free_mb: u64,
    pub camera: CameraInfo,
    pub encode: EncodeInfo,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

/// `record.start` result; `state` is always `recording`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Started {
    pub rec_id: RecId,
    pub state: State,
}

/// `record.list` result; `next` is absent on the last page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListResult {
    pub records: Vec<IndexSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<RecId>,
}

/// `record.delete` result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Deleted {
    pub deleted_bytes: u64,
}

/// `record.subscribe` / `media.unsubscribe` result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Subscribed {
    pub subscribed: bool,
}

/// `media.subscribe` ack; frames start only after this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaSubscribed {
    pub subscribed: bool,
    pub width: u32,
    pub height: u32,
    pub format: FrameFormat,
}

/// `hello` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelloResult {
    pub api_version: u32,
    pub server: String,
    pub build: String,
    pub pid: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SensorInfo {
    pub w: u32,
    pub h: u32,
    pub fps: u32,
    pub mode: String,
}

/// Throughput contract per codec at 1080p; measured, not guessed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EncoderLimits {
    pub h264_fps_1080p: u32,
    pub h265_fps_1080p: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImuInfo {
    pub who_am_i: u8,
    pub odr_hz: u32,
}

/// `device.info` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceInfo {
    pub sensor: SensorInfo,
    pub encoders: EncoderLimits,
    pub imu: ImuInfo,
    pub build: String,
}

/// Merged effective config (`/etc` + `/var/lib`). The schema belongs to
/// `action-config`; the contract only requires a JSON object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ConfigView(pub serde_json::Value);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LockState {
    #[serde(rename = "held")]
    Held,
}

/// `device.calibrate.enter`: the daemon released the hardware.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalibrateEntered {
    pub lock: LockState,
}

/// `device.calibrate.leave`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalibrateLeft {
    pub reopened: bool,
}

/// `system.log.tail` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogTail {
    pub lines: Vec<String>,
}

/// `system.shutdown` result (recording is stopped first).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ok {
    pub ok: bool,
}

/// Header line of a `media.frame`: followed by exactly `payload_len` bytes of
/// NV12. The payload may contain `\n`; never scan it line by line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameHeader {
    pub frame_id: u64,
    pub pts_ns: u64,
    pub width: u32,
    pub height: u32,
    pub format: FrameFormat,
    pub stride: u32,
    pub payload_len: u32,
}

impl FrameHeader {
    /// Bytes the reader must consume after this header.
    pub const fn payload_len(&self) -> usize {
        self.payload_len as usize
    }
}
