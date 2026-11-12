//! Control-plane messages (NDJSON, `/run/tspi-cam/control.sock`).

use serde::{Deserialize, Serialize};

/// Client -> daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ControlRequest {
    StartRecording,
    StopRecording,
    GetStatus,
}

/// Daemon -> client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ControlResponse {
    Status {
        state: String,
        recording_secs: f64,
        frames_captured: u64,
        imu_sampled: u64,
        disk_free_mb: u64,
        last_error: Option<String>,
    },
    Error {
        code: String,
        message: String,
    },
}
