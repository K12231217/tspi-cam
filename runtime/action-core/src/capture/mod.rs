//! Camera capture: one owned `FrameLease` per dequeued buffer, plus the
//! `FrameSource` trait. The V4L2 binding lands with the board integration; the
//! pool state machine and its accounting are host-testable here.

use std::time::Duration;

use thiserror::Error;

use crate::types::{Geometry, PixelFormat};

pub mod pool;
pub use pool::FrameLease;
#[cfg(target_os = "linux")]
pub mod v4l2;

#[derive(Debug, Error)]
pub enum CaptureError {
    #[error("cannot open camera device {0}: {1}")]
    OpenFailed(String, String),
    #[error("capture failed: {0}")]
    ReadFailed(String),
    #[error("device stopped")]
    Stopped,
    #[error("bad stride: stride={stride} vstride={vstride}")]
    BadStride { stride: u32, vstride: u32 },
    #[error("invalid pool configuration: {0}")]
    Invalid(String),
    #[error("unsupported pixel format: {0:?}")]
    UnsupportedFormat(PixelFormat),
}

/// What a started source delivers. `stride`/`vstride` come from the driver's
/// `plane_fmt` (see `action_contract::nv12`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceInfo {
    pub geom: Geometry,
    pub fps: u32,
    pub stride: u32,
    pub vstride: u32,
    pub format: PixelFormat,
}

/// One camera source. `stop()` only stops streaming: leases already handed out
/// stay readable, and `next_frame` afterwards fails with
/// [`CaptureError::Stopped`] (not `Ok(None)`).
pub trait FrameSource: Send {
    fn start(&mut self, geom: Geometry, fps: u32) -> Result<SourceInfo, CaptureError>;
    fn next_frame(&mut self, timeout: Duration) -> Result<Option<pool::FrameLease>, CaptureError>;
    fn stop(&mut self);
    fn info(&self) -> SourceInfo;
}
