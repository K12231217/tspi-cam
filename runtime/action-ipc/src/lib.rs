//! The one implementation of the tspi-cam IPC contract: JSON-RPC 2.0 over
//! newline-delimited JSON, one Unix socket per service. Message types only;
//! no sockets, no runtime, no tokio.
//!
//! **Wire path**: never `serde_json::from_str::<Request>`. An
//! adjacently-tagged `Call` turns an unknown method into a decode error, which
//! makes `METHOD_NOT_FOUND` indistinguishable from `INVALID_PARAMS`. Use
//! [`Request::decode`], which reports the method name / offending field in
//! `error.data`. `Call`'s derived `Deserialize` exists for tests and docs.
//!
//! Contract owner: `docs/design/ipc.md`. Any wire change bumps
//! [`API_VERSION`] and the `CHANGELOG` `ipc:` section.

pub mod error;
pub mod event;
pub mod method;
pub mod params;
pub mod results;
pub mod wire;

pub use error::{ErrorClass, RpcError, StateReason};
pub use event::{Event, Progress, RecordEvent};
pub use method::{Call, Lane};
pub use params::ConfigValue;
pub use results::{Dropped, FrameHeader, IndexSummary, State, Status};
pub use wire::{Id, Notification, Request, Response};

/// Contract version. Any incompatible change increments this.
pub const API_VERSION: u32 = 1;

/// Control/status socket (systemd `actiond.socket`, `0660 root:video`).
pub const RECORD_SOCK: &str = "/run/tspi-cam/record.sock";
/// Preview socket (`JSON-RPC header line + payload_len bytes`, `0660 root:video`).
pub const MEDIA_SOCK: &str = "/run/tspi-cam/media.sock";
