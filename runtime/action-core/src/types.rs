//! Cross-module semantic types. Wire types live in `action-ipc`; device facts
//! in `action-contract`. Nothing here serialises: the IPC layer converts.

use std::fmt;

/// `CLOCK_MONOTONIC` nanoseconds. The only clock in the system; wall time is
/// recorded once per session for display.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub struct MonoTime(pub u64);

impl MonoTime {
    pub const fn from_nanos(ns: u64) -> Self {
        Self(ns)
    }

    pub const fn as_nanos(self) -> u64 {
        self.0
    }

    /// Saturating arithmetic: a timestamp must never wrap into the past.
    pub const fn saturating_add(self, ns: u64) -> Self {
        Self(self.0.saturating_add(ns))
    }

    pub const fn saturating_sub(self, ns: u64) -> Self {
        Self(self.0.saturating_sub(ns))
    }
}

impl fmt::Display for MonoTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}ns", self.0)
    }
}

/// What the driver's timestamp means. Semantics were not verified for MIPI yet
/// (H13); `Unknown` must be reported rather than guessed.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum TsSource {
    Sof,
    Eof,
    Dequeue,
    #[default]
    Unknown,
}

/// Visible frame geometry in pixels.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Geometry {
    pub w: u32,
    pub h: u32,
}

impl Geometry {
    pub const fn new(w: u32, h: u32) -> Self {
        Self { w, h }
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum PixelFormat {
    Nv12,
}

/// Spans `(offset, len)` into a leased buffer. Lengths use `stride`/`vstride`,
/// never `w`/`h`: the row padding is part of the plane.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct FramePlanes {
    pub luma: (usize, usize),
    pub chroma: (usize, usize),
}

pub const EMPTY_PLANES: FramePlanes = FramePlanes {
    luma: (0, 0),
    chroma: (0, 0),
};

/// One IMU sample in SI units, raw counts kept for offline recomputation.
#[derive(Copy, Clone, Debug, Default)]
pub struct ImuSample {
    /// `t_spi_read` on the monotonic clock; M2 may switch to `t_drdy`.
    pub t: MonoTime,
    /// Angular velocity, rad/s.
    pub gyro: [f32; 3],
    /// Acceleration, m/s².
    pub accel: [f32; 3],
    /// Raw LSB values, same order as the register read.
    pub raw: [i16; 6],
}
