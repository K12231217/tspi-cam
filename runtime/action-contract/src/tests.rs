//! Contract tests: the constants must reproduce the measured anchors, and the
//! helpers must reject geometry that cannot be NV12.

use crate::encoder;
use crate::imu;
use crate::nv12;
use crate::sensor;

#[test]
fn plane_sizes_match_driver_sizeimage() {
    assert_eq!(nv12::plane_size(1920, 1080), Some(nv12::SIZEIMAGE_1080P));
    assert_eq!(
        nv12::plane_size(2592, 1944),
        Some(nv12::SIZEIMAGE_2592X1944)
    );
    assert_eq!(nv12::SIZEIMAGE_1080P, 3_110_400);
    assert_eq!(nv12::SIZEIMAGE_2592X1944, 7_558_272);
}

#[test]
fn planes_are_contiguous_and_cover_the_frame() {
    let (luma_off, luma_len) = nv12::luma_span(1920, 1080);
    let (chroma_off, chroma_len) = nv12::chroma_span(1920, 1080);
    assert_eq!(luma_off, 0);
    assert_eq!(chroma_off, luma_len);
    assert_eq!(
        (luma_len + chroma_len) as u64,
        nv12::plane_size(1920, 1080).unwrap()
    );
    assert_eq!(chroma_len, 1920 * 540);
}

#[test]
fn odd_geometry_is_rejected_not_rounded() {
    assert_eq!(nv12::plane_size(1920, 1081), None);
    assert_eq!(nv12::plane_size(0, 1080), None);
    assert_eq!(nv12::plane_size(1920, 0), None);
}

#[test]
fn stride_alignment_rounds_up_to_16() {
    assert_eq!(nv12::align_stride(1920), 1920);
    assert_eq!(nv12::align_stride(2592), 2592);
    assert_eq!(nv12::align_stride(1930), 1936);
    assert_eq!(nv12::align_stride(1), 16);
}

#[test]
fn sensor_resolutions_are_the_configured_set() {
    assert!(sensor::is_supported_resolution(1920, 1080));
    assert!(sensor::is_supported_resolution(2592, 1944));
    assert!(!sensor::is_supported_resolution(1234, 567));
    assert!(sensor::SUPPORTED_RESOLUTIONS.contains(&sensor::DEFAULT_RESOLUTION));
    assert_eq!((sensor::SOURCE_WIDTH, sensor::SOURCE_HEIGHT), (2592, 1944));
}

#[test]
fn encode_ceiling_only_reports_measured_points() {
    assert_eq!(encoder::encode_fps_ceiling(1920, 1080), Some(36));
    assert_eq!(encoder::encode_fps_ceiling(1280, 720), Some(81));
    assert_eq!(encoder::encode_fps_ceiling(640, 480), Some(201));
    assert_eq!(encoder::encode_fps_ceiling(960, 540), None);
    assert_eq!(encoder::encode_fps_ceiling(2592, 1944), None);
    assert!(encoder::CODECS.contains(&encoder::DEFAULT_CODEC));
}

#[test]
fn imu_identity_and_odr() {
    assert!(imu::who_am_i_matches(0x6B));
    assert!(!imu::who_am_i_matches(0x00));
    assert_eq!(imu::WHO_AM_I_REG, 0x0F);
    assert_eq!(imu::ODR_HZ, 208, "no 200 Hz step on this part");
}
