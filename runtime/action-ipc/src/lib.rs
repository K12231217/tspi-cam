//! IPC protocol definitions only: message types for the control plane
//! (NDJSON over a Unix socket) and the preview plane (binary frames over
//! a separate Unix socket). No socket implementation lives here.
//! See docs/architecture.md §9.

pub mod control;
pub mod preview;
