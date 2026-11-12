//! Preview-plane binary frame protocol (`/run/tspi-cam/preview.sock`).
//! Little-endian header, then the payload. See architecture §9.2.

/// Magic bytes at the start of every preview frame.
pub const PREVIEW_MAGIC: [u8; 4] = *b"TSP1";
/// Protocol version.
pub const PREVIEW_VERSION: u32 = 1;
/// Maximum payload size accepted (8 MiB).
pub const PREVIEW_MAX_PAYLOAD: u32 = 8 * 1024 * 1024;

/// Pixel format codes in the preview header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PreviewFormat {
    Mjpeg = 0,
    Nv12 = 1,
    Yuyv = 2,
}

/// Fixed-size header, 32 bytes, little-endian.
#[derive(Debug, Clone, Copy)]
pub struct PreviewHeader {
    pub frame_id: u64,
    pub timestamp_ns: u64,
    pub format: PreviewFormat,
    pub width: u32,
    pub height: u32,
    pub payload_len: u32,
}

impl PreviewHeader {
    pub const SIZE: usize = 32;
}
