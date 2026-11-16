//! Hardware encoding contract. v1 submits one NV12 lease per frame and returns
//! packets paired with `frame_id` (interfaces.md §4); the encoder may emit one
//! or more packets per frame and the caller releases the input only when the
//! packet for that id appears.
//!
//! The board implementation (`mpp`) talks to `librockchip_mpp` via `dlopen`;
//! host builds keep the types so tests and the pipeline compile everywhere.

use serde::{Deserialize, Serialize};

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

/// One compressed packet. `frame_id` is the source frame's
/// `v4l2_buffer.sequence`; the input lease may be released only after the
/// packet carrying that id has been produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedPacket {
    pub frame_id: u64,
    pub keyframe: bool,
    pub data: Vec<u8>,
}

#[cfg(target_os = "linux")]
pub mod mpp;
