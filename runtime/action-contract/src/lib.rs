//! Device facts, zero dependencies: what the hardware *is*, measured.
//!
//! Everything here must trace to a measurement or a driver/sensor datasheet
//! fact recorded in the repo; numbers that were not measured stay out. Semantic
//! types live in `action-core::types`, wire types in `action-ipc`; this crate
//! only states constants and pure arithmetic about them.
//!
//! Module boundaries: [`sensor`] (OV5695 modes), [`nv12`] (layout and the
//! 16-byte stride rule), [`encoder`] (MPP/VEPU2 measured throughput), [`imu`]
//! (LSM6DSR identity and read constraints).

#![forbid(unsafe_code)]

pub mod encoder;
pub mod imu;
pub mod nv12;
pub mod sensor;
pub mod v4l2;

#[cfg(test)]
mod tests;
