//! Independent video and IMU writers. Each writer owns its queue and its
//! file handles; a stall in one never stalls the other's sampling thread.
//! See architecture §5.

use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WriterError {
    #[error("cannot create file {0}: {1}")]
    CreateFailed(PathBuf, String),
    #[error("write failed: {0}")]
    WriteFailed(String),
    #[error("finalize failed: {0}")]
    FinalizeFailed(String),
}

/// Writes `video.avi` + `frames.csv` for one recording.
pub struct VideoWriter;

impl VideoWriter {
    pub fn new() -> Self {
        Self
    }

    /// Drain the video queue, flush and fsync the files.
    /// TODO (M1 A段 step 5).
    pub fn drain_and_flush(&mut self) -> Result<(), WriterError> {
        Ok(())
    }
}

impl Default for VideoWriter {
    fn default() -> Self {
        Self::new()
    }
}

/// Writes `imu.csv` for one recording.
pub struct ImuWriter;

impl ImuWriter {
    pub fn new() -> Self {
        Self
    }

    /// Drain the IMU queue, flush and fsync the file.
    /// TODO (M1 A段 step 5).
    pub fn drain_and_flush(&mut self) -> Result<(), WriterError> {
        Ok(())
    }
}

impl Default for ImuWriter {
    fn default() -> Self {
        Self::new()
    }
}
