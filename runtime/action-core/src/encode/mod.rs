//! Hardware encoding contract (`interfaces.md` §4).
//!
//! The encoder takes ownership of one NV12 lease per `submit` and hands it back
//! when the packet carrying that `frame_id` appears. That pairing is the whole
//! point: an encoder that drops frames under rate control must be able to skip
//! an id without pinning every input behind it, and a recording that ends with
//! inputs still held must report `LifecycleMismatch` instead of silently losing
//! frames.
//!
//! The board implementation (`mpp`) talks to `librockchip_mpp` via `dlopen`;
//! host builds keep the types so tests and the pipeline compile everywhere.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::capture::FrameLease;
use crate::types::{Geometry, MonoTime};

/// Recording codecs. Values match the IPC contract strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Codec {
    #[serde(rename = "h264")]
    H264,
    #[serde(rename = "h265")]
    H265,
}

impl Codec {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::H264 => "h264",
            Self::H265 => "h265",
        }
    }
}

/// One compressed packet, paired with the frame it came from.
///
/// `frame_id` is the source frame's `v4l2_buffer.sequence` (driver numbering,
/// gaps visible) and `pts` is that frame's monotonic timestamp. Both are
/// needed: `frame_id` matches input to output, `pts` is what a muxer needs for
/// A/V alignment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedPacket {
    pub frame_id: u64,
    pub pts: MonoTime,
    pub keyframe: bool,
    pub data: Vec<u8>,
}

/// Everything the encoder needs before the first frame.
///
/// `stride`/`vstride` describe the **input** frame, not `geom`. They are not
/// derivable from `geom` on this board: the driver hands out 1920-wide rows for
/// 1080p, and MPP rejects `MPP_ENC_SET_CFG` with `MPP_ERR_VALUE` unless both
/// strides arrive in the same cfg as width/height (then it silently keeps
/// 1280x720). That failure is why `feat/encode-blocked` existed.
///
/// `fps` is required for rate control: CBR needs a frame-rate denominator, and
/// `gop` is expressed in frames, so neither can be defaulted safely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncoderConfig {
    pub codec: Codec,
    /// Encoded output geometry (whitelist in `interfaces.md` §5).
    pub geom: Geometry,
    /// Input frame horizontal stride, from `SourceInfo::stride`.
    pub stride: u32,
    /// Input frame vertical stride, from `SourceInfo::vstride`.
    pub vstride: u32,
    pub bitrate_kbps: u32,
    pub fps: u32,
    pub gop: u32,
    /// B-frames. The contract requires 0; `open` rejects anything else.
    pub bframes: u8,
}

#[derive(Debug, thiserror::Error)]
pub enum EncodeError {
    #[error("encoder backend: {0}")]
    Backend(String),
    #[error("frame does not hold {expected} bytes")]
    FrameSize { expected: usize },
    #[error("input frame {frame_id} was still held when the flush deadline expired")]
    LifecycleMismatch { frame_id: u64 },
    #[error("config rejected: {0}")]
    Config(String),
}

/// Encoder accounting. `in_flight` is live (inputs submitted, packet not yet
/// produced); the rest are monotonic counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EncoderStats {
    pub submits: u64,
    pub packets: u64,
    /// Inputs the encoder dropped on its own (rate control); released without
    /// a packet and counted here, never left to pin the queue.
    pub skipped: u64,
    pub in_flight: usize,
    pub lifecycle_errors: u64,
}

/// The encoder slot (`interfaces.md` §4).
///
pub trait VideoEncoder: Send {
    fn open(cfg: &EncoderConfig) -> Result<Self, EncodeError>
    where
        Self: Sized;

    /// Take ownership of one input frame. The lease returns to the pool only
    /// when the packet for its `frame_id` is produced (or the id is skipped).
    fn submit(&mut self, frame: FrameLease) -> Result<(), EncodeError>;

    /// Block at most `timeout` for one packet. Producing a packet releases the
    /// input whose `frame_id` it carries.
    fn poll(&mut self, timeout: Duration) -> Result<Option<EncodedPacket>, EncodeError>;

    /// Signal end of stream. Inputs stay held until `flush` drains them.
    fn submit_eos(&mut self) -> Result<(), EncodeError>;

    /// Drain remaining inputs into packets by `deadline`. Only inputs still
    /// held when the deadline expires are a `LifecycleMismatch`; drained packets
    /// stay readable through `poll`.
    fn flush(&mut self, deadline: Duration) -> Result<(), EncodeError>;

    fn stats(&self) -> EncoderStats;
}

#[cfg(target_os = "linux")]
pub mod mpp;
