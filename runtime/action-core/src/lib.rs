//! Hardware operations, acquisition, buffering, distribution and file
//! writing. Owns no recording state and decides nothing about when to
//! start or stop; the daemon (`actiond`) decides, this crate executes.
//! See docs/architecture.md §3.

pub mod capture;
pub mod clock;
pub mod imu;
pub mod orientation;
pub mod record;
pub mod types;
