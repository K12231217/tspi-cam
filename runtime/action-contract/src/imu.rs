//! IMU660RB: an LSM6DSR-family device on SPI3 (`docs/architecture.md` §6.3,
//! §16 H3/H6).

/// SPI device node; bound by `spidev-bind.service` at boot (H5), so opening it
/// can fail before that unit runs.
pub const DEVICE: &str = "/dev/spidev3.0";

/// `WHO_AM_I` register address.
pub const WHO_AM_I_REG: u8 = 0x0F;
/// Expected `WHO_AM_I` value (LSM6DSR family), verified on the board.
pub const WHO_AM_I_LSM6DSR: u8 = 0x6B;

/// Hardware ODR actually configured: the LSM6DSR has no 200 Hz step, the
/// nearest is 208 Hz. Never write the 200 Hz target as hardware ODR into
/// metadata (`index.json` records both).
pub const ODR_HZ: u32 = 208;
/// M1 polling target rate; actual rate and jitter are measured and recorded.
pub const POLL_RATE_HZ: u32 = 200;

/// This board returns wrong data for multi-byte auto-increment reads (H6):
/// sample reads must be byte-at-a-time. This is a correctness constraint, not
/// a performance choice.
pub const MULTI_BYTE_READ_BROKEN: bool = true;

/// True when the chip identity read matches the expected part.
pub const fn who_am_i_matches(value: u8) -> bool {
    value == WHO_AM_I_LSM6DSR
}
