//! Rockchip MPP (VEPU2) H.264/H.265 encoder over the board's
//! `librockchip_mpp.so.1`, loaded at runtime with `dlopen`.
//!
//! Board facts that shaped this file (`docs/research/mpp-encoder-notes.md`):
//! - The `MppApi` table is `size = 184`; put_frame/get_packet/control are at
//!   offsets 40/48/112, verified by calling them.
//! - `MPP_ENC_SET_CFG` rejects the config with `MPP_ERR_VALUE` unless
//!   `prep:hor_stride` and `prep:ver_stride` are set alongside width/height.
//!   With both strides present a single call succeeds (no retry games).
//! - `encode_get_packet` returns `0` (not EAGAIN) with a NULL packet when the
//!   queue is empty; that must end the drain, not be dereferenced.

use std::ffi::{c_char, c_void, CString};
use std::ptr;

use super::EncodedPacket;
use crate::capture::FrameLease;

pub use super::Codec;

const RTLD_NOW: i32 = 2;

const MPP_CTX_ENC: u32 = 1;
const MPP_VIDEO_AVC: u32 = 7;
const MPP_VIDEO_HEVC: u32 = 0x0100_0004;
const MPP_FMT_YUV420SP: u32 = 0;
const MPP_BUFFER_TYPE_DRM: u32 = 3;
const MPP_BUFFER_INTERNAL: u32 = 0;
const MPP_ENC_RC_MODE_CBR: i32 = 1;

const MPP_ENC_SET_CFG: u32 = 0x0032_0001;
const MPP_ENC_SET_IDR_FRAME: u32 = 0x0032_0009;

const API_PUT_FRAME: usize = 40;
const API_GET_PACKET: usize = 48;
const API_CONTROL: usize = 112;
const API_EXPECTED_SIZE: u32 = 8 + 14 * 8 + 16 * 4;

type MppCtx = *mut c_void;
type MppApi = *mut c_void;
type MppEncCfg = *mut c_void;
type MppFrame = *mut c_void;
type MppPacket = *mut c_void;
type MppBuffer = *mut c_void;
type MppBufferGroup = *mut c_void;

#[derive(Debug, thiserror::Error)]
pub enum MppError {
    #[error("dlopen({0}) failed: {1}")]
    Load(String, String),
    #[error("symbol {0} missing")]
    Symbol(String),
    #[error("MppApi size {got} != expected {expected} (ABI mismatch)")]
    ApiAbi { got: u32, expected: u32 },
    #[error("{call} returned {code}")]
    Call { call: &'static str, code: i32 },
    #[error("config key {key} rejected with {code}")]
    ConfigKey { key: String, code: i32 },
    #[error("frame does not hold {expected} bytes")]
    FrameSize { expected: usize },
}

struct Lib {
    handle: *mut c_void,
}

type Ret = i32;
type Sym = *mut c_void;

macro_rules! sym {
    ($lib:expr, $name:ident, fn($($arg:ty),*) -> $ret:ty) => {{
        let c = CString::new(stringify!($name)).expect("static");
        // SAFETY: caller immediately uses the rk_mpi.h signature.
        let p = unsafe { libc::dlsym($lib.handle, c.as_ptr()) };
        if p.is_null() {
            return Err(MppError::Symbol(stringify!($name).into()));
        }
        // SAFETY: signature copied from rk_mpi.h / mpp_frame.h.
        unsafe { std::mem::transmute::<Sym, extern "C" fn($($arg),*) -> $ret>(p) }
    }};
}

pub struct MppEncoder {
    ctx: MppCtx,
    api: MppApi,
    cfg: MppEncCfg,
    group: MppBufferGroup,
    width: u32,
    height: u32,
    stride: u32,
    vstride: u32,
    codec: Codec,

    mpp_destroy: extern "C" fn(MppCtx) -> Ret,
    enc_cfg_deinit: extern "C" fn(MppEncCfg) -> Ret,
    frame_init: extern "C" fn(*mut MppFrame) -> Ret,
    frame_deinit: extern "C" fn(*mut MppFrame) -> Ret,
    frame_set_width: extern "C" fn(MppFrame, u32),
    frame_set_height: extern "C" fn(MppFrame, u32),
    frame_set_hor_stride: extern "C" fn(MppFrame, u32),
    frame_set_ver_stride: extern "C" fn(MppFrame, u32),
    frame_set_fmt: extern "C" fn(MppFrame, u32),
    frame_set_pts: extern "C" fn(MppFrame, i64),
    frame_set_eos: extern "C" fn(MppFrame, u32),
    frame_set_buffer: extern "C" fn(MppFrame, MppBuffer),
    buffer_group_put: extern "C" fn(MppBufferGroup) -> Ret,
    buffer_get:
        extern "C" fn(MppBufferGroup, *mut MppBuffer, usize, *const c_char, *const c_char) -> Ret,
    buffer_get_ptr: extern "C" fn(MppBuffer, *const c_char) -> *mut c_void,
    buffer_put: extern "C" fn(MppBuffer, *const c_char) -> Ret,
    packet_deinit: extern "C" fn(*mut MppPacket) -> Ret,
    packet_get_pos: extern "C" fn(MppPacket) -> *mut c_void,
    packet_get_length: extern "C" fn(MppPacket) -> usize,
    packet_get_pts: extern "C" fn(MppPacket) -> i64,
}

impl MppEncoder {
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        codec: Codec,
        width: u32,
        height: u32,
        stride: u32,
        vstride: u32,
        bitrate_kbps: u32,
        fps: u32,
    ) -> Result<Self, MppError> {
        let lib = Lib {
            handle: open_lib()?,
        };

        let mpp_create = sym!(lib, mpp_create, fn(*mut MppCtx, *mut MppApi) -> Ret);
        let mpp_init = sym!(lib, mpp_init, fn(MppCtx, u32, u32) -> Ret);
        let mpp_destroy = sym!(lib, mpp_destroy, fn(MppCtx) -> Ret);
        let mpp_enc_cfg_init = sym!(lib, mpp_enc_cfg_init, fn(*mut MppEncCfg) -> Ret);
        let enc_cfg_deinit = sym!(lib, mpp_enc_cfg_deinit, fn(MppEncCfg) -> Ret);
        let mpp_enc_cfg_set_s32 = sym!(
            lib,
            mpp_enc_cfg_set_s32,
            fn(MppEncCfg, *const c_char, i32) -> Ret
        );
        let frame_init = sym!(lib, mpp_frame_init, fn(*mut MppFrame) -> Ret);
        let frame_deinit = sym!(lib, mpp_frame_deinit, fn(*mut MppFrame) -> Ret);
        let frame_set_width = sym!(lib, mpp_frame_set_width, fn(MppFrame, u32) -> ());
        let frame_set_height = sym!(lib, mpp_frame_set_height, fn(MppFrame, u32) -> ());
        let frame_set_hor_stride = sym!(lib, mpp_frame_set_hor_stride, fn(MppFrame, u32) -> ());
        let frame_set_ver_stride = sym!(lib, mpp_frame_set_ver_stride, fn(MppFrame, u32) -> ());
        let frame_set_fmt = sym!(lib, mpp_frame_set_fmt, fn(MppFrame, u32) -> ());
        let frame_set_pts = sym!(lib, mpp_frame_set_pts, fn(MppFrame, i64) -> ());
        let frame_set_eos = sym!(lib, mpp_frame_set_eos, fn(MppFrame, u32) -> ());
        let frame_set_buffer = sym!(lib, mpp_frame_set_buffer, fn(MppFrame, MppBuffer) -> ());
        let buffer_group_get = sym!(
            lib,
            mpp_buffer_group_get,
            fn(*mut MppBufferGroup, u32, u32, *const c_char, *const c_char) -> Ret
        );
        let buffer_group_put = sym!(lib, mpp_buffer_group_put, fn(MppBufferGroup) -> Ret);
        let buffer_get = sym!(
            lib,
            mpp_buffer_get_with_tag,
            fn(MppBufferGroup, *mut MppBuffer, usize, *const c_char, *const c_char) -> Ret
        );
        let buffer_get_ptr = sym!(
            lib,
            mpp_buffer_get_ptr_with_caller,
            fn(MppBuffer, *const c_char) -> *mut c_void
        );
        let buffer_put = sym!(
            lib,
            mpp_buffer_put_with_caller,
            fn(MppBuffer, *const c_char) -> Ret
        );
        let packet_deinit = sym!(lib, mpp_packet_deinit, fn(*mut MppPacket) -> Ret);
        let packet_get_pos = sym!(lib, mpp_packet_get_pos, fn(MppPacket) -> *mut c_void);
        let packet_get_length = sym!(lib, mpp_packet_get_length, fn(MppPacket) -> usize);
        let packet_get_pts = sym!(lib, mpp_packet_get_pts, fn(MppPacket) -> i64);

        let mut ctx: MppCtx = ptr::null_mut();
        let mut api: MppApi = ptr::null_mut();
        check("mpp_create", mpp_create(&mut ctx, &mut api))?;
        // SAFETY: api is the table returned by mpp_create; first field is size.
        let api_size = unsafe { *(api as *const u32) };
        if api_size != API_EXPECTED_SIZE {
            return Err(MppError::ApiAbi {
                got: api_size,
                expected: API_EXPECTED_SIZE,
            });
        }
        let coding = match codec {
            Codec::H264 => MPP_VIDEO_AVC,
            Codec::H265 => MPP_VIDEO_HEVC,
        };
        check("mpp_init", mpp_init(ctx, MPP_CTX_ENC, coding))?;

        // SAFETY: control lives at the documented offset in the table.
        let control: extern "C" fn(MppCtx, u32, *mut c_void) -> Ret =
            unsafe { std::mem::transmute(api_slot(api, API_CONTROL)) };

        let mut cfg: MppEncCfg = ptr::null_mut();
        check("mpp_enc_cfg_init", mpp_enc_cfg_init(&mut cfg))?;
        let set = |cfg: MppEncCfg, key: &str, val: i32| -> Result<(), MppError> {
            let k = CString::new(key).expect("static key");
            let ret = mpp_enc_cfg_set_s32(cfg, k.as_ptr(), val);
            if ret != 0 {
                return Err(MppError::ConfigKey {
                    key: key.to_string(),
                    code: ret,
                });
            }
            Ok(())
        };
        let bps = bitrate_kbps as i32 * 1000;
        for (key, value) in [
            ("prep:width", width as i32),
            ("prep:height", height as i32),
            // Mandatory on this lib: without both strides SET_CFG is rejected
            // with MPP_ERR_VALUE and the encoder silently keeps 1280x720.
            ("prep:hor_stride", stride as i32),
            ("prep:ver_stride", vstride as i32),
            ("prep:format", MPP_FMT_YUV420SP as i32),
            ("codec:type", coding as i32),
            ("rc:mode", MPP_ENC_RC_MODE_CBR),
            ("rc:bps_target", bps),
            ("rc:bps_max", bps * 17 / 16),
            ("rc:bps_min", bps * 15 / 16),
            ("rc:fps_in_flex", 0),
            ("rc:fps_in_num", fps as i32),
            ("rc:fps_in_denorm", 1),
            ("rc:fps_out_flex", 0),
            ("rc:fps_out_num", fps as i32),
            ("rc:fps_out_denorm", 1),
            ("rc:gop", (fps * 2) as i32),
        ] {
            set(cfg, key, value)?;
        }
        check(
            "MPP_ENC_SET_CFG",
            control(ctx, MPP_ENC_SET_CFG, cfg as *mut c_void),
        )?;

        let mut group: MppBufferGroup = ptr::null_mut();
        let tag = c_str("tspi-cam");
        let caller = c_str("mpp_open");
        check(
            "mpp_buffer_group_get",
            buffer_group_get(
                &mut group,
                MPP_BUFFER_TYPE_DRM,
                MPP_BUFFER_INTERNAL,
                tag.as_ptr(),
                caller.as_ptr(),
            ),
        )?;

        Ok(Self {
            ctx,
            api,
            cfg,
            group,
            width,
            height,
            stride,
            vstride,
            codec,
            mpp_destroy,
            enc_cfg_deinit,
            frame_init,
            frame_deinit,
            frame_set_width,
            frame_set_height,
            frame_set_hor_stride,
            frame_set_ver_stride,
            frame_set_fmt,
            frame_set_pts,
            frame_set_eos,
            frame_set_buffer,
            buffer_group_put,
            buffer_get,
            buffer_get_ptr,
            buffer_put,
            packet_deinit,
            packet_get_pos,
            packet_get_length,
            packet_get_pts,
        })
    }

    pub fn codec(&self) -> Codec {
        self.codec
    }

    /// Encode one NV12 lease; `lease.id` becomes the packet PTS so packets can
    /// be paired with their source frame (interfaces.md §4).
    pub fn encode(&self, lease: &FrameLease) -> Result<Vec<EncodedPacket>, MppError> {
        let expected = self.stride as usize * self.vstride as usize * 3 / 2;
        let planes = lease.planes();
        if planes[0].len() + planes[1].len() < expected {
            return Err(MppError::FrameSize { expected });
        }
        let tag = c_str("tspi-cam");
        let caller = c_str("encode");
        let mut buf: MppBuffer = ptr::null_mut();
        check(
            "mpp_buffer_get",
            (self.buffer_get)(
                self.group,
                &mut buf,
                expected,
                tag.as_ptr(),
                caller.as_ptr(),
            ),
        )?;
        // SAFETY: MPP owns a buffer of `expected` bytes and we hold the ref.
        let dst = (self.buffer_get_ptr)(buf, caller.as_ptr()) as *mut u8;
        unsafe {
            ptr::copy_nonoverlapping(planes[0].as_ptr(), dst, planes[0].len());
            ptr::copy_nonoverlapping(
                planes[1].as_ptr(),
                dst.add(planes[0].len()),
                planes[1].len(),
            );
        }

        let mut frame: MppFrame = ptr::null_mut();
        check("mpp_frame_init", (self.frame_init)(&mut frame))?;
        (self.frame_set_width)(frame, self.width);
        (self.frame_set_height)(frame, self.height);
        (self.frame_set_hor_stride)(frame, self.stride);
        (self.frame_set_ver_stride)(frame, self.vstride);
        (self.frame_set_fmt)(frame, MPP_FMT_YUV420SP);
        (self.frame_set_pts)(frame, lease.id as i64);
        (self.frame_set_buffer)(frame, buf);

        // SAFETY: put_frame is a table pointer at the documented offset.
        let put: extern "C" fn(MppCtx, MppFrame) -> Ret =
            unsafe { std::mem::transmute(api_slot(self.api, API_PUT_FRAME)) };
        let ret = put(self.ctx, frame);
        (self.frame_deinit)(&mut frame);
        let _ = (self.buffer_put)(buf, caller.as_ptr());
        check("encode_put_frame", ret)?;
        self.drain_packets()
    }

    /// Send an EOS frame and drain whatever the encoder still holds.
    pub fn flush(&self) -> Result<Vec<EncodedPacket>, MppError> {
        let mut frame: MppFrame = ptr::null_mut();
        check("mpp_frame_init", (self.frame_init)(&mut frame))?;
        (self.frame_set_eos)(frame, 1);
        // SAFETY: put_frame is a table pointer at the documented offset.
        let put: extern "C" fn(MppCtx, MppFrame) -> Ret =
            unsafe { std::mem::transmute(api_slot(self.api, API_PUT_FRAME)) };
        let ret = put(self.ctx, frame);
        (self.frame_deinit)(&mut frame);
        check("encode_put_frame(eos)", ret)?;
        self.drain_packets()
    }

    pub fn request_idr(&self) -> Result<(), MppError> {
        // SAFETY: control lives at the documented offset in the table.
        let control: extern "C" fn(MppCtx, u32, *mut c_void) -> Ret =
            unsafe { std::mem::transmute(api_slot(self.api, API_CONTROL)) };
        check(
            "MPP_ENC_SET_IDR_FRAME",
            control(self.ctx, MPP_ENC_SET_IDR_FRAME, ptr::null_mut()),
        )
    }

    fn drain_packets(&self) -> Result<Vec<EncodedPacket>, MppError> {
        // SAFETY: get_packet is a table pointer at the documented offset.
        let get: extern "C" fn(MppCtx, *mut MppPacket) -> Ret =
            unsafe { std::mem::transmute(api_slot(self.api, API_GET_PACKET)) };
        let mut out = Vec::new();
        loop {
            let mut packet: MppPacket = ptr::null_mut();
            let ret = get(self.ctx, &mut packet);
            // This lib signals "empty" with ret == 0 and a null packet.
            if ret != 0 || packet.is_null() {
                break;
            }
            let pos = (self.packet_get_pos)(packet) as *const u8;
            let len = (self.packet_get_length)(packet);
            let pts = (self.packet_get_pts)(packet);

            // SAFETY: pos/len describe the packet payload owned by the packet.
            let data = unsafe { std::slice::from_raw_parts(pos, len) }.to_vec();
            (self.packet_deinit)(&mut packet);
            let keyframe = contains_idr(&data, self.codec);
            out.push(EncodedPacket {
                frame_id: pts.max(0) as u64,
                keyframe,
                data,
            });
        }
        Ok(out)
    }
}

impl Drop for MppEncoder {
    fn drop(&mut self) {
        let _ = (self.buffer_group_put)(self.group);
        (self.enc_cfg_deinit)(self.cfg);
        let _ = (self.mpp_destroy)(self.ctx);
    }
}

fn open_lib() -> Result<*mut c_void, MppError> {
    let name = c_str("librockchip_mpp.so.1");
    // SAFETY: plain dlopen; the handle stays valid for the process lifetime.
    let handle = unsafe { libc::dlopen(name.as_ptr(), RTLD_NOW) };
    if handle.is_null() {
        let err = unsafe {
            let e = libc::dlerror();
            if e.is_null() {
                "unknown".to_string()
            } else {
                std::ffi::CStr::from_ptr(e).to_string_lossy().into_owned()
            }
        };
        return Err(MppError::Load("librockchip_mpp.so.1".into(), err));
    }
    Ok(handle)
}

fn c_str(s: &str) -> CString {
    CString::new(s).expect("no NUL in static strings")
}

fn check(call: &'static str, ret: Ret) -> Result<(), MppError> {
    if ret == 0 {
        Ok(())
    } else {
        Err(MppError::Call { call, code: ret })
    }
}

/// Annex-B scan for an IDR slice. MPP's packet flag bits are not documented in
/// this BSP's headers, and the flag read returned 0 for an IDR start, so the
/// NAL type is the source of truth.
fn contains_idr(data: &[u8], codec: Codec) -> bool {
    let mut i = 0;
    while i + 4 <= data.len() {
        let start = if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            Some(i + 3)
        } else if i + 5 <= data.len()
            && data[i] == 0
            && data[i + 1] == 0
            && data[i + 2] == 0
            && data[i + 3] == 1
        {
            Some(i + 4)
        } else {
            None
        };
        if let Some(pos) = start {
            if let Some(&hdr) = data.get(pos) {
                let nal = match codec {
                    Codec::H264 => hdr & 0x1f,
                    Codec::H265 => (hdr >> 1) & 0x3f,
                };
                let is_idr = match codec {
                    Codec::H264 => nal == 5,
                    Codec::H265 => nal == 19 || nal == 20,
                };
                if is_idr {
                    return true;
                }
            }
            i = pos.max(i + 1);
        } else {
            i += 1;
        }
    }
    false
}

/// # Safety
/// `api` must be the table returned by `mpp_create` and `offset` one of the
/// pointer offsets from `rk_mpi.h`.
unsafe fn api_slot(api: MppApi, offset: usize) -> *mut c_void {
    *((api as *mut u8).add(offset) as *mut *mut c_void)
}
