//! rkisp V4L2 facts measured on the board (H13, 2026-11-16). These are
//! vendor-kernel specifics, not mainline assumptions: the binding must use
//! them verbatim. Evidence and raw output: docs/research/h13-mipi-timestamps.md.

/// The capture node is multiplanar: `V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE` (9).
/// A single-planar buffer type is rejected with `EINVAL`.
pub const MULTIPLANAR: bool = true;
/// `sizeof(struct v4l2_format)` this kernel accepts for `G_FMT`/`S_FMT`.
pub const FORMAT_STRUCT_SIZE: usize = 208;
/// `sizeof(struct v4l2_buffer)`.
pub const BUFFER_STRUCT_SIZE: usize = 88;
/// `sizeof(struct v4l2_plane)`; `buf.length` is the plane count (1 here).
pub const PLANE_STRUCT_SIZE: usize = 64;
/// `plane_fmt[0].sizeimage` offset inside `v4l2_format` (vendor layout).
pub const OFFSET_SIZEIMAGE: usize = 28;
/// `plane_fmt[0].bytesperline` offset inside `v4l2_format`.
pub const OFFSET_BYTESPERLINE: usize = 32;
/// `VIDIOC_STREAMON/OFF` payload: one `int` (`_IOW`, size 4).
pub const STREAM_IOCTL_SIZE: usize = 4;

/// Driver timestamps are in `CLOCK_MONOTONIC` (`flags & 0x2000` was set on
/// every buffer), so `clock_offset_ns` between domains is 0.
pub const TIMESTAMP_IS_MONOTONIC: bool = true;
/// Timestamp source is EOF (`TSTAMP_SRC_SOE` was clear): `t_capture ≈ t_eof − 1/fps`.
pub const TIMESTAMP_SRC_EOF: bool = true;
