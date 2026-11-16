//! Device facts only, zero dependencies: OV5695 modes, NV12/stride formulas,
//! encoder limits, IMU660RB registers.
//!
//! Semantic types live in `action-core::types`; the wire side keeps plain
//! `u64`/`String`. Nothing in this crate may depend on another crate.
//! See docs/design/interfaces.md §2.
