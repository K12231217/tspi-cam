//! Camera capture abstraction. Upper layers (distributor, writer,
//! preview) depend only on [`Frame`] and [`CaptureAdapter`], never on a
//! concrete V4L2 device. M1 implements the MIPI adapter only; USB was
//! dropped entirely (architecture v1.2).

use crate::types::{Frame, PixelFormat};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CaptureError {
    #[error("cannot open camera device {0}: {1}")]
    OpenFailed(String, String),
    #[error("capture failed: {0}")]
    ReadFailed(String),
    #[error("unsupported pixel format: {0:?}")]
    UnsupportedFormat(PixelFormat),
}

/// Low-level camera adapter. One implementation per camera source.
pub trait CaptureAdapter {
    fn open(&mut self, device: &str, width: u32, height: u32, fps: u32)
        -> Result<(), CaptureError>;
    fn next_frame(&mut self) -> Result<Frame, CaptureError>;
    fn close(&mut self);
}

pub mod mipi_nv12;
