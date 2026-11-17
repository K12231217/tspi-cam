//! IMU660RB (LSM6DSR family) on SPI3.
//!
//! **Board constraint (H6, re-confirmed in `taishanpi-tools`): multi-byte
//! auto-increment reads return frozen/garbage data on this board** while single
//! byte reads are correct (temp ~21.5 °C, gyro ~0, |accel| ~1 g). Every register
//! is therefore read with its own 2-byte transfer: 12 transfers per sample,
//! ~12 ioctls. That is a correctness requirement, not a tuning choice.
//!
//! The node also disappears on reboot and is `0600 root:root`; the image needs
//! `spidev-bind.service` plus a udev rule (`0660 root:video`, deploy.md).

#[cfg(unix)]
use action_contract::imu as facts;
use thiserror::Error;

#[cfg(unix)]
use crate::clock::SystemTimebase;
use crate::types::{ImuSample, MonoTime};

pub const CTRL1_XL: u8 = 0x10;
pub const CTRL2_G: u8 = 0x11;
pub const CTRL3_C: u8 = 0x12;
pub const STATUS_REG: u8 = 0x1E;
pub const GYRO_OUT: u8 = 0x22;
pub const ACCEL_OUT: u8 = 0x28;

/// ODR 208 Hz, ±2 g (CTRL1_XL) / ±250 dps (CTRL2_G). 208 is the closest step to
/// the 200 Hz target; the target itself must never be written as hardware ODR.
pub const CTRL_ODR_208HZ: u8 = 0x50;
/// BDU + IF_INC: block-data update avoids tearing even though reads are single.
pub const CTRL3_BDU_IF_INC: u8 = 0x44;

/// Sensitivities at the configured full scale.
pub const GYRO_MDPS_PER_LSB: f32 = 8.75;
pub const ACCEL_MG_PER_LSB: f32 = 0.061;

const GRAVITY: f32 = 9.806_65;
const DEG_TO_RAD: f32 = std::f32::consts::PI / 180.0;

#[derive(Debug, Error)]
pub enum ImuError {
    #[error("cannot open IMU {0}: {1}")]
    OpenFailed(String, String),
    #[error("IMU read failed: {0}")]
    ReadFailed(String),
    #[error("IMU config did not stick: wrote {reg:#04x}={want:#04x}, read {got:#04x}")]
    ConfigStuck { reg: u8, want: u8, got: u8 },
    #[error("unexpected WHO_AM_I: got {got:#04x}, expected {expected:#04x}")]
    WhoAmIMismatch { got: u8, expected: u8 },
    #[error("unsupported ODR {0} Hz (this part has 208 Hz)")]
    UnsupportedOdr(u32),
    #[error("IMU is not open")]
    NotOpen,
}

/// Identity of an opened sensor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImuInfo {
    pub who_am_i: u8,
    pub odr_hz: u32,
}

/// Raw counts exactly as read, before any scaling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RawSample {
    pub gyro: [i16; 3],
    pub accel: [i16; 3],
}

/// 12 little-endian bytes: gyro XYZ then accel XYZ (registers 0x22..0x2D).
pub fn decode(bytes: &[u8; 12]) -> RawSample {
    let s16 = |lo: u8, hi: u8| i16::from_le_bytes([lo, hi]);
    RawSample {
        gyro: [
            s16(bytes[0], bytes[1]),
            s16(bytes[2], bytes[3]),
            s16(bytes[4], bytes[5]),
        ],
        accel: [
            s16(bytes[6], bytes[7]),
            s16(bytes[8], bytes[9]),
            s16(bytes[10], bytes[11]),
        ],
    }
}

/// SI units: rad/s and m/s², per the calibrated sensitivities.
pub fn to_si(raw: &RawSample) -> ([f32; 3], [f32; 3]) {
    let g = DEG_TO_RAD * GYRO_MDPS_PER_LSB / 1000.0;
    let a = ACCEL_MG_PER_LSB / 1000.0 * GRAVITY;
    (
        [
            raw.gyro[0] as f32 * g,
            raw.gyro[1] as f32 * g,
            raw.gyro[2] as f32 * g,
        ],
        [
            raw.accel[0] as f32 * a,
            raw.accel[1] as f32 * a,
            raw.accel[2] as f32 * a,
        ],
    )
}

pub fn sample_at(raw: RawSample, t: MonoTime) -> ImuSample {
    let (gyro, accel) = to_si(&raw);
    ImuSample {
        t,
        gyro,
        accel,
        raw: [
            raw.gyro[0],
            raw.gyro[1],
            raw.gyro[2],
            raw.accel[0],
            raw.accel[1],
            raw.accel[2],
        ],
    }
}

/// One IMU source. `read()` is the only sampling entry point; DRDY (M2) must
/// stay expressible without changing callers.
pub trait ImuSource: Send {
    fn open(&mut self, spi: &str, odr_hz: u32) -> Result<ImuInfo, ImuError>;
    fn read(&mut self) -> Result<ImuSample, ImuError>;
    /// (previous sample time, last sample time); both zero before the first read.
    fn last_timing(&self) -> (MonoTime, MonoTime);
}

#[cfg(unix)]
pub use imp::Lsm6dsr;

#[cfg(unix)]
mod imp {
    use std::os::fd::RawFd;

    use super::*;

    const SPI_IOC_WR_MODE: u64 = 0x4001_6B01;
    const SPI_IOC_WR_BITS_PER_WORD: u64 = 0x4001_6B03;
    const SPI_IOC_WR_MAX_SPEED_HZ: u64 = 0x4004_6B04;
    /// `SPI_IOC_MESSAGE(1)`.
    const SPI_IOC_MESSAGE_1: u64 = 0x4020_6B00;
    const SPI_MODE: u8 = 0;
    const SPI_BITS: u8 = 8;
    const SPI_SPEED_HZ: u32 = 1_000_000;

    #[repr(C)]
    struct Transfer {
        tx_buf: u64,
        rx_buf: u64,
        len: u32,
        speed_hz: u32,
        delay_usecs: u16,
        bits_per_word: u8,
        cs_change: u8,
        tx_nbits: u8,
        rx_nbits: u8,
        word_delay_usecs: u8,
        pad: u8,
    }

    const _: () = assert!(std::mem::size_of::<Transfer>() == 32);

    struct Spidev {
        fd: RawFd,
    }

    impl Spidev {
        fn open(path: &str) -> Result<Self, ImuError> {
            let c_path = std::ffi::CString::new(path)
                .map_err(|_| ImuError::OpenFailed(path.into(), "path contains NUL".into()))?;
            // SAFETY: valid C string, flags are constants.
            let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDWR) };
            if fd < 0 {
                return Err(ImuError::OpenFailed(
                    path.into(),
                    std::io::Error::last_os_error().to_string(),
                ));
            }
            let dev = Self { fd };
            dev.set_ioctl(SPI_IOC_WR_MODE, &SPI_MODE)?;
            dev.set_ioctl(SPI_IOC_WR_BITS_PER_WORD, &SPI_BITS)?;
            dev.set_ioctl(SPI_IOC_WR_MAX_SPEED_HZ, &SPI_SPEED_HZ)?;
            Ok(dev)
        }

        fn set_ioctl<T>(&self, request: u64, value: &T) -> Result<(), ImuError> {
            // SAFETY: `value` outlives the call; the kernel only reads it.
            let ptr = (value as *const T).cast::<libc::c_void>() as *mut libc::c_void;
            let rc = unsafe { libc::ioctl(self.fd, request as _, ptr) };
            if rc < 0 {
                return Err(ImuError::OpenFailed(
                    "spidev".into(),
                    std::io::Error::last_os_error().to_string(),
                ));
            }
            Ok(())
        }

        fn xfer(&self, tx: &[u8]) -> Result<Vec<u8>, ImuError> {
            let mut rx = vec![0u8; tx.len()];
            let mut tr = Transfer {
                tx_buf: tx.as_ptr() as u64,
                rx_buf: rx.as_mut_ptr() as u64,
                len: tx.len() as u32,
                speed_hz: SPI_SPEED_HZ,
                delay_usecs: 0,
                bits_per_word: SPI_BITS,
                cs_change: 0,
                tx_nbits: 0,
                rx_nbits: 0,
                word_delay_usecs: 0,
                pad: 0,
            };
            // SAFETY: both buffers live until ioctl returns; `tr` is 32 bytes.
            let rc = unsafe {
                libc::ioctl(
                    self.fd,
                    SPI_IOC_MESSAGE_1 as _,
                    &mut tr as *mut Transfer as *mut _,
                )
            };
            if rc < 0 {
                return Err(ImuError::ReadFailed(
                    std::io::Error::last_os_error().to_string(),
                ));
            }
            Ok(rx)
        }
    }

    impl Drop for Spidev {
        fn drop(&mut self) {
            // SAFETY: fd is owned by this struct and closed exactly once.
            unsafe { libc::close(self.fd) };
        }
    }

    /// Single-byte register access only (H6).
    pub struct Lsm6dsr {
        dev: Option<Spidev>,
        info: Option<ImuInfo>,
        prev_t: MonoTime,
        last_t: MonoTime,
    }

    impl Lsm6dsr {
        pub fn new() -> Self {
            Self {
                dev: None,
                info: None,
                prev_t: MonoTime(0),
                last_t: MonoTime(0),
            }
        }

        fn dev(&self) -> Result<&Spidev, ImuError> {
            self.dev.as_ref().ok_or(ImuError::NotOpen)
        }

        fn read_reg(&self, reg: u8) -> Result<u8, ImuError> {
            let rx = self.dev()?.xfer(&[reg | 0x80, 0x00])?;
            Ok(rx[1])
        }

        fn write_reg(&self, reg: u8, value: u8) -> Result<(), ImuError> {
            self.dev()?.xfer(&[reg & 0x7F, value])?;
            Ok(())
        }

        fn check(&self, reg: u8, want: u8) -> Result<(), ImuError> {
            let got = self.read_reg(reg)?;
            if got != want {
                return Err(ImuError::ConfigStuck { reg, want, got });
            }
            Ok(())
        }
    }

    impl Default for Lsm6dsr {
        fn default() -> Self {
            Self::new()
        }
    }

    impl ImuSource for Lsm6dsr {
        fn open(&mut self, spi: &str, odr_hz: u32) -> Result<ImuInfo, ImuError> {
            if odr_hz != facts::ODR_HZ {
                return Err(ImuError::UnsupportedOdr(odr_hz));
            }
            self.dev = Some(Spidev::open(spi)?);
            let who = self.read_reg(facts::WHO_AM_I_REG)?;
            if who != facts::WHO_AM_I_LSM6DSR {
                return Err(ImuError::WhoAmIMismatch {
                    got: who,
                    expected: facts::WHO_AM_I_LSM6DSR,
                });
            }
            self.write_reg(CTRL3_C, CTRL3_BDU_IF_INC)?;
            self.write_reg(CTRL1_XL, CTRL_ODR_208HZ)?;
            self.write_reg(CTRL2_G, CTRL_ODR_208HZ)?;
            self.check(CTRL3_C, CTRL3_BDU_IF_INC)?;
            self.check(CTRL1_XL, CTRL_ODR_208HZ)?;
            self.check(CTRL2_G, CTRL_ODR_208HZ)?;
            let info = ImuInfo {
                who_am_i: who,
                odr_hz,
            };
            self.info = Some(info);
            Ok(info)
        }

        fn read(&mut self) -> Result<ImuSample, ImuError> {
            // Twelve single-byte transfers; a block read is wrong on this board.
            let mut bytes = [0u8; 12];
            for (i, byte) in bytes.iter_mut().enumerate() {
                let reg = GYRO_OUT + i as u8;
                *byte = self.read_reg(reg)?;
            }
            let t = SystemTimebase::system_now();
            self.prev_t = self.last_t;
            self.last_t = t;
            Ok(sample_at(decode(&bytes), t))
        }

        fn last_timing(&self) -> (MonoTime, MonoTime) {
            (self.prev_t, self.last_t)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_signed_little_endian() {
        let bytes = [
            0x00, 0x80, // gyro X = -32768
            0xFF, 0x7F, // gyro Y = +32767
            0x00, 0x00, // gyro Z = 0
            0x00, 0x40, // accel X = +16384 (1 g at ±2 g)
            0x00, 0xC0, // accel Y = -16384
            0x00, 0x00, // accel Z = 0
        ];
        let raw = decode(&bytes);
        assert_eq!(raw.gyro, [-32768, 32767, 0]);
        assert_eq!(raw.accel, [16384, -16384, 0]);
    }

    #[test]
    fn one_g_is_gravity() {
        let raw = RawSample {
            accel: [16384, 0, 0],
            ..Default::default()
        };
        let (_, accel) = to_si(&raw);
        assert!((accel[0] - 9.80665).abs() < 0.02, "got {}", accel[0]);
        assert_eq!(accel[1], 0.0);
    }

    #[test]
    fn dps_scales_to_rad_per_second() {
        // 250 dps full scale -> raw 28571 is ~250 dps ~ 4.363 rad/s.
        let raw = RawSample {
            gyro: [28571, 0, 0],
            ..Default::default()
        };
        let (gyro, _) = to_si(&raw);
        let expected = 250.0 * std::f32::consts::PI / 180.0;
        assert!((gyro[0] - expected).abs() < 0.05, "got {}", gyro[0]);
    }

    #[test]
    fn sample_keeps_raw_counts() {
        let bytes = [1, 0, 2, 0, 3, 0, 4, 0, 5, 0, 6, 0];
        let sample = sample_at(decode(&bytes), MonoTime(42));
        assert_eq!(sample.t, MonoTime(42));
        assert_eq!(sample.raw, [1, 2, 3, 4, 5, 6]);
    }
}
