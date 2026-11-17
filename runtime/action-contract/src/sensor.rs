//! OV5695 over MIPI (rkisp). Facts from `docs/architecture.md` §16 H14 and the
//! `media-ctl` topology recorded there.

/// Sensor: OmniVision OV5695 behind the Rockchip ISP.
pub const SENSOR: &str = "OV5695";

/// The only mode the sensor exposes. `media-ctl` reports
/// `SBGGR10_1X10/2592x1944@10000/300000`, i.e. 30 fps at the source.
pub const SOURCE_WIDTH: u32 = 2592;
pub const SOURCE_HEIGHT: u32 = 1944;
pub const SOURCE_FPS: u32 = 30;

/// The sensor's single media-bus mode string, verbatim from `media-ctl -p`.
pub const MBUS_MODE: &str = "SBGGR10_1X10/2592x1944@10000/300000";

/// Resolutions `camera.width/height` accepts (config §12). The first is the
/// default for recording.
pub const SUPPORTED_RESOLUTIONS: &[(u32, u32)] = &[
    (1920, 1080),
    (1280, 720),
    (960, 540),
    (480, 800),
    (2592, 1944),
];

/// Default encode geometry (1080p30 H.264).
pub const DEFAULT_RESOLUTION: (u32, u32) = (1920, 1080);

/// `VIDIOC_S_PARM` is not supported by this driver (`Inappropriate ioctl`), so
/// changing resolution does not change the frame rate. Measured: 27-28 fps at
/// 640x480, 1280x720, 1920x1080 and 2592x1944 (H14).
pub const S_PARM_UNSUPPORTED: bool = true;

/// Frame-rate ceiling observed across all four tested modes (H14). Use this,
/// not [`SOURCE_FPS`], when reasoning about achieved frames.
pub const MEASURED_FPS_MIN: f32 = 27.0;
pub const MEASURED_FPS_MAX: f32 = 28.0;

/// True when the pair is one of [`SUPPORTED_RESOLUTIONS`].
pub fn is_supported_resolution(width: u32, height: u32) -> bool {
    SUPPORTED_RESOLUTIONS
        .iter()
        .any(|&(w, h)| w == width && h == height)
}
