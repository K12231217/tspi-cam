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
//!
//! Input ownership: `submit` takes the lease and keeps it in `pending` until a
//! packet carrying that frame's token appears, or until the encoder demonstrably
//! skipped it. That is `interfaces.md` §4's pairing rule, and it is what makes
//! `in_flight` meaningful: the pool cannot hand the same slot to the driver
//! while the encoder still believes it owns the frame.

use std::collections::{HashMap, VecDeque};
use std::ffi::{c_char, c_void, CString};
use std::ptr;
use std::time::{Duration, Instant};

use super::{EncodeError, EncodedPacket, EncoderConfig, EncoderStats, VideoEncoder};
use crate::capture::FrameLease;
use crate::types::MonoTime;

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

/// How long `poll` sleeps between `get_packet` attempts. MPP has no blocking
/// wait, so this is the cost of not busy-spinning a core during a 33 ms frame.
const POLL_SLEEP: Duration = Duration::from_millis(1);

type MppCtx = *mut c_void;
type MppApi = *mut c_void;
type MppEncCfg = *mut c_void;
type MppFrame = *mut c_void;
type MppPacket = *mut c_void;
type MppBuffer = *mut c_void;
type MppBufferGroup = *mut c_void;

#[derive(Debug, thiserror::Error)]
enum MppError {
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
}

impl From<MppError> for EncodeError {
    fn from(e: MppError) -> Self {
        Self::Backend(e.to_string())
    }
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

/// One submitted frame whose packet has not appeared yet.
///
/// The lease is held here on purpose: the pool must not recycle the slot while
/// the encoder still owns the input (`interfaces.md` §4).
struct Pending {
    frame_id: u64,
    pts: MonoTime,
    _lease: FrameLease,
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

    /// Inputs submitted and not yet released, keyed by the token MPP echoes
    /// back. Tokens increase with submission order, so a packet whose token is
    /// `t` proves every pending token `< t` was skipped by the encoder.
    pending: HashMap<i64, Pending>,
    next_token: i64,
    /// Packets drained by `flush`, which returns `()` by contract but must not
    /// lose them; `poll` serves these first.
    deferred: VecDeque<EncodedPacket>,
    stats: EncoderStats,
    eos_sent: bool,
}

// SAFETY: every mutating method takes `&mut self`, so the MPP context is only
// ever driven from one thread at a time. Moving the encoder to another thread
// transfers that exclusive access; no `&self` method touches the context (only
// `stats`, which reads plain fields). The raw pointers are owned exclusively by
// this struct and outlive it only through `Drop`.
unsafe impl Send for MppEncoder {}

impl MppEncoder {
    /// Request an IDR on the next submitted frame. Not part of the trait: the
    /// session decides when a keyframe is wanted (start of recording, recovery
    /// after an error), the encoder only provides the hook.
    pub fn request_idr(&mut self) -> Result<(), EncodeError> {
        // SAFETY: control lives at the documented offset in the table.
        let control: extern "C" fn(MppCtx, u32, *mut c_void) -> Ret =
            unsafe { std::mem::transmute(api_slot(self.api, API_CONTROL)) };
        check(
            "MPP_ENC_SET_IDR_FRAME",
            control(self.ctx, MPP_ENC_SET_IDR_FRAME, ptr::null_mut()),
        )?;
        Ok(())
    }

    fn open_inner(cfg: &EncoderConfig) -> Result<Self, MppError> {
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
        let coding = match cfg.codec {
            Codec::H264 => MPP_VIDEO_AVC,
            Codec::H265 => MPP_VIDEO_HEVC,
        };
        check("mpp_init", mpp_init(ctx, MPP_CTX_ENC, coding))?;

        // SAFETY: control lives at the documented offset in the table.
        let control: extern "C" fn(MppCtx, u32, *mut c_void) -> Ret =
            unsafe { std::mem::transmute(api_slot(api, API_CONTROL)) };

        let mut enc_cfg: MppEncCfg = ptr::null_mut();
        check("mpp_enc_cfg_init", mpp_enc_cfg_init(&mut enc_cfg))?;
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
        let bps = cfg.bitrate_kbps as i32 * 1000;
        for (key, value) in [
            ("prep:width", cfg.geom.w as i32),
            ("prep:height", cfg.geom.h as i32),
            // Mandatory on this lib: without both strides SET_CFG is rejected
            // with MPP_ERR_VALUE and the encoder silently keeps 1280x720.
            ("prep:hor_stride", cfg.stride as i32),
            ("prep:ver_stride", cfg.vstride as i32),
            ("prep:format", MPP_FMT_YUV420SP as i32),
            ("codec:type", coding as i32),
            ("rc:mode", MPP_ENC_RC_MODE_CBR),
            ("rc:bps_target", bps),
            ("rc:bps_max", bps * 17 / 16),
            ("rc:bps_min", bps * 15 / 16),
            ("rc:fps_in_flex", 0),
            ("rc:fps_in_num", cfg.fps as i32),
            ("rc:fps_in_denorm", 1),
            ("rc:fps_out_flex", 0),
            ("rc:fps_out_num", cfg.fps as i32),
            ("rc:fps_out_denorm", 1),
            ("rc:gop", cfg.gop as i32),
        ] {
            set(enc_cfg, key, value)?;
        }
        check("MPP_ENC_SET_CFG", control(ctx, MPP_ENC_SET_CFG, enc_cfg))?;

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
            cfg: enc_cfg,
            group,
            width: cfg.geom.w,
            height: cfg.geom.h,
            stride: cfg.stride,
            vstride: cfg.vstride,
            codec: cfg.codec,
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
            pending: HashMap::new(),
            next_token: 1,
            deferred: VecDeque::new(),
            stats: EncoderStats::default(),
            eos_sent: false,
        })
    }

    /// Copy one lease into an MPP buffer and hand it to the encoder.
    fn put_frame(&mut self, lease: &FrameLease) -> Result<(), EncodeError> {
        let expected = self.stride as usize * self.vstride as usize * 3 / 2;
        let planes = lease.planes();
        if planes[0].len() + planes[1].len() < expected {
            return Err(EncodeError::FrameSize { expected });
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

        let token = self.next_token;
        let mut frame: MppFrame = ptr::null_mut();
        check("mpp_frame_init", (self.frame_init)(&mut frame))?;
        (self.frame_set_width)(frame, self.width);
        (self.frame_set_height)(frame, self.height);
        (self.frame_set_hor_stride)(frame, self.stride);
        (self.frame_set_ver_stride)(frame, self.vstride);
        (self.frame_set_fmt)(frame, MPP_FMT_YUV420SP);
        (self.frame_set_pts)(frame, token);
        (self.frame_set_buffer)(frame, buf);

        // SAFETY: put_frame is a table pointer at the documented offset.
        let put: extern "C" fn(MppCtx, MppFrame) -> Ret =
            unsafe { std::mem::transmute(api_slot(self.api, API_PUT_FRAME)) };
        let ret = put(self.ctx, frame);
        (self.frame_deinit)(&mut frame);
        let _ = (self.buffer_put)(buf, caller.as_ptr());
        check("encode_put_frame", ret)?;

        // Only advance the token once the frame is really accepted, so a failed
        // submit never leaves a hole that later looks like a skipped frame.
        self.next_token += 1;
        Ok(())
    }

    /// Pull at most one packet. `None` means the encoder has nothing ready.
    fn drain_one(&mut self) -> Result<Option<EncodedPacket>, EncodeError> {
        // SAFETY: get_packet is a table pointer at the documented offset.
        let get: extern "C" fn(MppCtx, *mut MppPacket) -> Ret =
            unsafe { std::mem::transmute(api_slot(self.api, API_GET_PACKET)) };
        let mut packet: MppPacket = ptr::null_mut();
        let ret = get(self.ctx, &mut packet);
        // This lib signals "empty" with ret == 0 and a null packet.
        if ret != 0 || packet.is_null() {
            return Ok(None);
        }
        let pos = (self.packet_get_pos)(packet) as *const u8;
        let len = (self.packet_get_length)(packet);
        let token = (self.packet_get_pts)(packet);
        // SAFETY: pos/len describe the packet payload owned by the packet.
        let data = unsafe { std::slice::from_raw_parts(pos, len) }.to_vec();
        (self.packet_deinit)(&mut packet);
        let keyframe = contains_idr(&data, self.codec);

        let Some(p) = self.pending.remove(&token) else {
            // MPP invented a token: report it rather than attributing the
            // packet to frame 0 and silently mislabelling a recording.
            return Err(EncodeError::Backend(format!(
                "packet with unknown token {token}"
            )));
        };
        // Every still-pending token below this one was skipped by rate control:
        // release those inputs now, or they would pin the pool forever.
        self.release_skipped_below(token);
        self.stats.packets += 1;
        Ok(Some(EncodedPacket {
            frame_id: p.frame_id,
            pts: p.pts,
            keyframe,
            data,
        }))
    }

    /// Drop every pending input with a token lower than `token`, counting them
    /// as skipped. Their leases return to the pool here.
    fn release_skipped_below(&mut self, token: i64) {
        let skipped: Vec<i64> = self
            .pending
            .keys()
            .copied()
            .filter(|t| *t < token)
            .collect();
        for t in skipped {
            self.pending.remove(&t);
            self.stats.skipped += 1;
        }
    }

    fn send_eos(&mut self) -> Result<(), EncodeError> {
        let mut frame: MppFrame = ptr::null_mut();
        check("mpp_frame_init", (self.frame_init)(&mut frame))?;
        (self.frame_set_eos)(frame, 1);
        // SAFETY: put_frame is a table pointer at the documented offset.
        let put: extern "C" fn(MppCtx, MppFrame) -> Ret =
            unsafe { std::mem::transmute(api_slot(self.api, API_PUT_FRAME)) };
        let ret = put(self.ctx, frame);
        (self.frame_deinit)(&mut frame);
        check("encode_put_frame(eos)", ret)?;
        Ok(())
    }
}

impl VideoEncoder for MppEncoder {
    fn open(cfg: &EncoderConfig) -> Result<Self, EncodeError> {
        // Contract: no B-frames, because output order must match input order for
        // the `frame_id` pairing to work without a reorder buffer.
        if cfg.bframes != 0 {
            return Err(EncodeError::Config(format!(
                "bframes must be 0 (got {})",
                cfg.bframes
            )));
        }
        if cfg.stride < cfg.geom.w || cfg.vstride < cfg.geom.h {
            return Err(EncodeError::Config(format!(
                "stride {}x{} smaller than geometry {}x{}",
                cfg.stride, cfg.vstride, cfg.geom.w, cfg.geom.h
            )));
        }
        Ok(Self::open_inner(cfg)?)
    }

    fn submit(&mut self, frame: FrameLease) -> Result<(), EncodeError> {
        if self.eos_sent {
            return Err(EncodeError::Config("submit after submit_eos".into()));
        }
        self.put_frame(&frame)?;
        let token = self.next_token - 1;
        self.stats.submits += 1;
        self.pending.insert(
            token,
            Pending {
                frame_id: frame.id,
                pts: frame.pts,
                _lease: frame,
            },
        );
        Ok(())
    }

    fn poll(&mut self, timeout: Duration) -> Result<Option<EncodedPacket>, EncodeError> {
        if let Some(p) = self.deferred.pop_front() {
            return Ok(Some(p));
        }
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(p) = self.drain_one()? {
                return Ok(Some(p));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            std::thread::sleep(POLL_SLEEP);
        }
    }

    fn submit_eos(&mut self) -> Result<(), EncodeError> {
        if !self.eos_sent {
            self.send_eos()?;
            self.eos_sent = true;
        }
        Ok(())
    }

    fn flush(&mut self, deadline: Duration) -> Result<(), EncodeError> {
        self.submit_eos()?;
        let until = Instant::now() + deadline;
        while !self.pending.is_empty() {
            match self.drain_one()? {
                Some(p) => self.deferred.push_back(p),
                None => {
                    if Instant::now() >= until {
                        // Inputs still held after the deadline: this is the
                        // only condition the contract calls a lifecycle error.
                        let stuck = self.pending.keys().copied().min().unwrap_or_default();
                        let frame_id = self
                            .pending
                            .get(&stuck)
                            .map(|p| p.frame_id)
                            .unwrap_or_default();
                        let held = self.pending.len() as u64;
                        self.pending.clear();
                        self.stats.skipped += held;
                        self.stats.lifecycle_errors += 1;
                        return Err(EncodeError::LifecycleMismatch { frame_id });
                    }
                    std::thread::sleep(POLL_SLEEP);
                }
            }
        }
        Ok(())
    }

    fn stats(&self) -> EncoderStats {
        EncoderStats {
            in_flight: self.pending.len(),
            ..self.stats
        }
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
