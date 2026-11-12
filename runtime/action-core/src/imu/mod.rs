//! SPI IMU sampling (IMU660RB on `/dev/spidev3.0`).
//!
//! M1 polls on a timer at `imu.rate_hz` and records actual sampling
//! interval + jitter. DRDY support is M2 and needs the INT1/INT2 wiring
//! (H4); the sampling interface must not hard-code polling.

use crate::types::ImuSample;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ImuError {
    #[error("cannot open IMU device {0}: {1}")]
    OpenFailed(String, String),
    #[error("IMU read failed: {0}")]
    ReadFailed(String),
}

/// How a sample is triggered. M1 uses `Poll`; M2 may use `Drdy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleSource {
    Poll,
    Drdy,
}

/// A single IMU read. Implemented against spidev in M1 A段 step 3.
pub trait ImuReader {
    fn open(&mut self, device: &str, rate_hz: u32) -> Result<(), ImuError>;
    fn next_sample(&mut self) -> Result<ImuSample, ImuError>;
    fn close(&mut self);
}

pub struct Imu660rb {
    device: String,
    rate_hz: u32,
    source: SampleSource,
}

impl Imu660rb {
    pub fn new() -> Self {
        Self {
            device: String::new(),
            rate_hz: 0,
            source: SampleSource::Poll,
        }
    }

    pub fn source(&self) -> SampleSource {
        self.source
    }
}

impl Default for Imu660rb {
    fn default() -> Self {
        Self::new()
    }
}

impl ImuReader for Imu660rb {
    fn open(&mut self, device: &str, rate_hz: u32) -> Result<(), ImuError> {
        self.device = device.to_string();
        self.rate_hz = rate_hz;
        Err(ImuError::OpenFailed(
            device.to_string(),
            "IMU reader not implemented yet (M1 A段 step 3)".to_string(),
        ))
    }

    fn next_sample(&mut self) -> Result<ImuSample, ImuError> {
        Err(ImuError::ReadFailed(
            "IMU reader not implemented yet (M1 A段 step 3)".to_string(),
        ))
    }

    fn close(&mut self) {}
}
