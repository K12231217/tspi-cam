//! Server -> client notifications (`Notification`). Not part of `Call`:
//! a client must ignore notifications it does not recognise so that new
//! events never break old clients.

use serde::{Deserialize, Serialize};

use crate::error::ErrorClass;
use crate::params::RecId;
use crate::results::{Dropped, FrameHeader};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method", content = "params")]
pub enum Event {
    #[serde(rename = "record.progress")]
    RecordProgress(Progress),
    #[serde(rename = "record.event")]
    RecordEvent(RecordEvent),
    /// Only on `media.sock`, only after a `media.subscribe` ack; the header
    /// line is followed by exactly `payload_len` bytes.
    #[serde(rename = "media.frame")]
    MediaFrame(FrameHeader),
}

impl Event {
    pub const fn method(&self) -> &'static str {
        match self {
            Self::RecordProgress(_) => "record.progress",
            Self::RecordEvent(_) => "record.event",
            Self::MediaFrame(_) => "media.frame",
        }
    }
}

/// Periodic recording progress (also used inside `record.status`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Progress {
    pub rec_id: RecId,
    pub elapsed_ms: u64,
    pub frames: u64,
    pub imu_samples: u64,
    pub dropped: Dropped,
}

/// Event-form error; same `class`/`code` pairs as `index.json.errors`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordEvent {
    pub rec_id: RecId,
    pub class: ErrorClass,
    pub code: i32,
    pub message: String,
}
