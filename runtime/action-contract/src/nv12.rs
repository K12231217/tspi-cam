//! NV12 layout. The three quantities `w`, `stride` and `vstride` are distinct:
//! luma rows are `stride` bytes apart (only `w` of them meaningful), the chroma
//! plane starts at `stride * vstride`, and the last row may be padded in
//! `vstride`. Consumers jump with `stride`/`vstride`, never with `w`/`h`.
//!
//! Measured anchors (`docs/architecture.md` §16 H9, and §10.3):
//! 1080p frame = 3_110_400 B, 2592x1944 frame = 7_558_272 B, both exactly
//! `stride * vstride * 3/2` with `stride == w`, `vstride == h`.

/// MPP's stride alignment: pass frames with 16-aligned stride to avoid the
/// per-frame RGA realign attempt (`RGA_BLIT fail`, §10.3).
pub const STRIDE_ALIGN: u32 = 16;

/// Round a width up to the stride alignment.
pub const fn align_stride(width: u32) -> u32 {
    width.div_ceil(STRIDE_ALIGN) * STRIDE_ALIGN
}

/// Chroma plane height for a given visible geometry; NV12 has 4:2:0 chroma.
pub const fn chroma_height(height: u32) -> u32 {
    height / 2
}

/// Bytes the encoder must receive for `stride x vstride`, luma plus chroma.
/// `None` when the geometry cannot be NV12 (zero or odd dimensions).
pub const fn plane_size(stride: u32, vstride: u32) -> Option<u64> {
    if stride == 0 || vstride == 0 || !vstride.is_multiple_of(2) || !stride.is_multiple_of(2) {
        return None;
    }
    Some(stride as u64 * vstride as u64 * 3 / 2)
}

/// `(offset, len)` of the luma plane inside a frame buffer.
pub const fn luma_span(stride: u32, vstride: u32) -> (usize, usize) {
    (0, stride as usize * vstride as usize)
}

/// `(offset, len)` of the interleaved chroma plane.
pub const fn chroma_span(stride: u32, vstride: u32) -> (usize, usize) {
    let offset = stride as usize * vstride as usize;
    (offset, stride as usize * chroma_height(vstride) as usize)
}

/// Driver `sizeimage` measured for 1920x1080 NV12 (H9).
pub const SIZEIMAGE_1080P: u64 = 3_110_400;
/// Driver `sizeimage` measured for 2592x1944 NV12 (§10.3 / §16).
pub const SIZEIMAGE_2592X1944: u64 = 7_558_272;
