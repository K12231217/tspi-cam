//! MPP (VEPU2) measured throughput and supported codecs. Numbers are measured
//! on this board (`docs/architecture.md` §16 H12/H15/H16), never encoder
//! datasheet peaks.

/// H.264/H.265 are the supported recording codecs; MJPEG exists in the MPP
/// build but is only a degraded fallback (software MJPEG reached 12 fps at
/// 1080p, §10.3).
pub const CODECS: &[&str] = &["h264", "h265"];

pub const DEFAULT_CODEC: &str = "h264";

/// Pure-encode ceiling with `fakesink` (H12/H15), 300 frames each:
/// 1920x1080 = 36 fps, 1280x720 = 81 fps, 640x480 = 201 fps. H.264 and H.265
/// measured the same at 1080p.
pub const H264_FPS_1080P: u32 = 36;
pub const H265_FPS_1080P: u32 = 36;
pub const H264_FPS_720P: u32 = 81;
pub const H264_FPS_480P: u32 = 201;

/// Sustained camera -> H.264 1080p through the whole gst pipeline (H12):
/// 27.0 fps over 300 frames. This is the number that bounds a recording, not
/// the pure-encode ceiling.
pub const PIPELINE_FPS_1080P: f32 = 27.0;

/// Measured 2592x1944 H.264 -> MKV over 3600 frames (H16): 30.045 fps, frame
/// interval std 0.45 ms, zero gaps > 40 ms — only with a `queue` between
/// elements (single-threaded wiring collapsed to 20.5 fps).
pub const PIPELINE_FPS_2592X1944: f32 = 30.045;

/// Pure-encode ceiling for a supported geometry, `None` when unmeasured.
/// Only measured points are returned: guessing between them is forbidden.
pub const fn encode_fps_ceiling(width: u32, height: u32) -> Option<u32> {
    match (width, height) {
        (1920, 1080) => Some(H264_FPS_1080P),
        (1280, 720) => Some(H264_FPS_720P),
        (640, 480) => Some(H264_FPS_480P),
        _ => None,
    }
}
