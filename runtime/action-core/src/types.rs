//! Shared data types: one video frame and one IMU sample, both stamped on
//! the same clock (see `clock`).

/// Pixel format of a captured frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Mjpeg,
    Nv12,
    Yuyv,
}

/// One captured video frame. `payload` is owned: for NV12 it holds the
/// plane data as delivered by the driver (MIPI OV5695).
#[derive(Debug)]
pub struct Frame {
    /// Monotonically increasing frame number.
    pub frame_id: u64,
    /// Capture time, `CLOCK_MONOTONIC`, nanoseconds. See architecture §6.2:
    /// the driver timestamp's clock domain and semantics (SOF / EOF /
    /// dequeue) must be verified on the board (H2).
    pub timestamp_ns: u64,
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
    pub payload: Vec<u8>,
}

/// One IMU sample, already converted to SI units (rad/s, m/s²).
/// See architecture §6.4.
#[derive(Debug, Clone, Copy)]
pub struct ImuSample {
    /// Read time, `CLOCK_MONOTONIC`, nanoseconds.
    pub timestamp_ns: u64,
    /// Angular velocity, rad/s.
    pub gx: f32,
    pub gy: f32,
    pub gz: f32,
    /// Acceleration, m/s².
    pub ax: f32,
    pub ay: f32,
    pub az: f32,
}
