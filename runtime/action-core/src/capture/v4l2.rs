//! rkisp V4L2 binding over the measured vendor facts
//! (`action_contract::v4l2`, `docs/research/h13-mipi-timestamps.md`).
//!
//! Streaming path: `S_FMT(NV12) -> REQBUFS(8) -> mmap -> QBUF -> STREAMON`,
//! then `poll + DQBUF` per frame. The dequeued `v4l2_buffer.index` is the pool
//! slot index, and dropping the lease runs `QBUF` for that same index. Nothing
//! in this file allocates or copies frame data.
//!
//! `stop()` only issues `STREAMOFF` (leases stay readable); `munmap` happens
//! when the pool and this struct are dropped.

use std::ffi::CString;
use std::io;
use std::os::fd::RawFd;
use std::sync::Arc;
use std::time::Duration;

use action_contract::v4l2 as facts;

use super::pool::{FrameLease, Pool, Requeue};
use super::{CaptureError, FrameSource, SourceInfo};
use crate::types::{Geometry, MonoTime, PixelFormat};

const CAPTURE_MPLANE: u32 = 9;
const MEMORY_MMAP: u32 = 1;
const FIELD_NONE: u32 = 1;
const NV12_FOURCC: u32 = 0x3231_564E;
/// Capacity rule: `count >= queue_depth + encoder_in_flight_max + 2` -> 8.
const BUFFER_COUNT: u32 = 8;

const fn ioc(direction: u64, nr: u64, size: u64) -> u64 {
    (direction << 30) | (size << 16) | (0x56 << 8) | nr
}

const QUERYCAP: u64 = ioc(2, 0, 104);
const G_FMT: u64 = ioc(3, 4, 208);
const S_FMT: u64 = ioc(3, 5, 208);
const REQBUFS: u64 = ioc(3, 8, 20);
const QUERYBUF: u64 = ioc(3, 9, 88);
const QBUF: u64 = ioc(3, 15, 88);
const DQBUF: u64 = ioc(3, 17, 88);
const STREAMON: u64 = ioc(1, 18, 4);
const STREAMOFF: u64 = ioc(1, 19, 4);

const CAP_VIDEO_CAPTURE_MPLANE: u32 = 0x0000_1000;
const FLAG_TIMESTAMP_MASK: u32 = 0x0000_E000;
const FLAG_TIMESTAMP_MONOTONIC: u32 = 0x0000_2000;

#[repr(C)]
struct Plane {
    bytesused: u32,
    length: u32,
    m: u64,
    data_offset: u32,
    reserved: [u32; 11],
}

#[repr(C)]
struct Buffer {
    index: u32,
    type_: u32,
    bytesused: u32,
    flags: u32,
    field: u32,
    timestamp_sec: i64,
    timestamp_usec: i64,
    timecode: [u8; 16],
    sequence: u32,
    memory: u32,
    m: u64,
    length: u32,
    reserved2: u32,
    request_fd: i32,
}

const _: () = assert!(std::mem::size_of::<Plane>() == facts::PLANE_STRUCT_SIZE);
const _: () = assert!(std::mem::size_of::<Buffer>() == facts::BUFFER_STRUCT_SIZE);

fn check_fd(fd: RawFd, request: u64, ptr: *mut libc::c_void) -> io::Result<()> {
    // SAFETY: `ptr` points at a live struct of the size encoded in `request`.
    let rc = unsafe { libc::ioctl(fd, request as _, ptr) };
    if rc < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn qbuf(fd: RawFd, index: u32) -> io::Result<()> {
    let mut plane = Plane {
        bytesused: 0,
        length: 0,
        m: 0,
        data_offset: 0,
        reserved: [0; 11],
    };
    let mut buf = Buffer {
        index,
        type_: CAPTURE_MPLANE,
        bytesused: 0,
        flags: 0,
        field: 0,
        timestamp_sec: 0,
        timestamp_usec: 0,
        timecode: [0; 16],
        sequence: 0,
        memory: MEMORY_MMAP,
        m: &mut plane as *mut Plane as u64,
        length: 1,
        reserved2: 0,
        request_fd: 0,
    };
    check_fd(fd, QBUF, &mut buf as *mut Buffer as *mut libc::c_void)
}

struct FdHolder(RawFd);

impl Drop for FdHolder {
    fn drop(&mut self) {
        // SAFETY: this holder is the only owner of the descriptor.
        unsafe { libc::close(self.0) };
    }
}

struct V4l2Requeue {
    fd: Arc<FdHolder>,
}

impl Requeue for V4l2Requeue {
    fn requeue(&self, index: usize) -> Result<(), CaptureError> {
        qbuf(self.fd.0, index as u32)
            .map_err(|e| CaptureError::ReadFailed(format!("QBUF({index}): {e}")))
    }
}

pub struct V4l2Capture {
    fd: Arc<FdHolder>,
    /// Declared before `maps`: the pool (and its leases) must be gone before the
    /// regions are unmapped.
    pool: Option<Pool>,
    info: SourceInfo,
    maps: Vec<(usize, usize)>,
    streaming: bool,
}

impl V4l2Capture {
    pub fn open(device: &str) -> Result<Self, CaptureError> {
        let c_path = CString::new(device)
            .map_err(|_| CaptureError::OpenFailed(device.into(), "path contains NUL".into()))?;
        // SAFETY: valid C string; flags are constants.
        let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDWR | libc::O_NONBLOCK) };
        if fd < 0 {
            return Err(CaptureError::OpenFailed(
                device.into(),
                std::io::Error::last_os_error().to_string(),
            ));
        }
        let fd = Arc::new(FdHolder(fd));

        let mut caps = [0u8; 104];
        check_fd(fd.0, QUERYCAP, caps.as_mut_ptr() as *mut libc::c_void)
            .map_err(|e| CaptureError::OpenFailed(device.into(), format!("QUERYCAP: {e}")))?;
        let device_caps = u32::from_ne_bytes([caps[88], caps[89], caps[90], caps[91]]);
        if device_caps & CAP_VIDEO_CAPTURE_MPLANE == 0 {
            return Err(CaptureError::OpenFailed(
                device.into(),
                format!("not a multiplanar capture node (caps {device_caps:#010x})"),
            ));
        }

        Ok(Self {
            fd,
            pool: None,
            info: SourceInfo {
                geom: Geometry::new(0, 0),
                fps: 0,
                stride: 0,
                vstride: 0,
                format: PixelFormat::Nv12,
            },
            maps: Vec::new(),
            streaming: false,
        })
    }

    fn negotiate(&self, geom: Geometry) -> Result<(u32, u32), CaptureError> {
        let mut fmt = [0u8; facts::FORMAT_STRUCT_SIZE];
        fmt[0..4].copy_from_slice(&CAPTURE_MPLANE.to_ne_bytes());
        check_fd(self.fd.0, G_FMT, fmt.as_mut_ptr() as *mut libc::c_void)
            .map_err(|e| CaptureError::ReadFailed(format!("G_FMT: {e}")))?;
        // Only geometry/format/field change; the driver's num_planes and the
        // vendor-specific field offsets stay untouched (H13 probe proof).
        fmt[8..12].copy_from_slice(&geom.w.to_ne_bytes());
        fmt[12..16].copy_from_slice(&geom.h.to_ne_bytes());
        fmt[16..20].copy_from_slice(&NV12_FOURCC.to_ne_bytes());
        fmt[20..24].copy_from_slice(&FIELD_NONE.to_ne_bytes());
        check_fd(self.fd.0, S_FMT, fmt.as_mut_ptr() as *mut libc::c_void)
            .map_err(|e| CaptureError::ReadFailed(format!("S_FMT {}x{}: {e}", geom.w, geom.h)))?;
        check_fd(self.fd.0, G_FMT, fmt.as_mut_ptr() as *mut libc::c_void)
            .map_err(|e| CaptureError::ReadFailed(format!("G_FMT after S_FMT: {e}")))?;
        let stride = u32::from_ne_bytes([fmt[32], fmt[33], fmt[34], fmt[35]]);
        let sizeimage = u32::from_ne_bytes([fmt[28], fmt[29], fmt[30], fmt[31]]);
        let row = stride as u64 * 3 / 2;
        if stride == 0 || row == 0 || sizeimage as u64 % row != 0 {
            return Err(CaptureError::BadStride { stride, vstride: 0 });
        }
        let vstride = (sizeimage as u64 / row) as u32;
        Ok((stride, vstride))
    }
}

impl FrameSource for V4l2Capture {
    fn start(&mut self, geom: Geometry, fps: u32) -> Result<SourceInfo, CaptureError> {
        let (stride, vstride) = self.negotiate(geom)?;

        let mut rb = [0u8; 20];
        rb[0..4].copy_from_slice(&BUFFER_COUNT.to_ne_bytes());
        rb[4..8].copy_from_slice(&CAPTURE_MPLANE.to_ne_bytes());
        rb[8..12].copy_from_slice(&MEMORY_MMAP.to_ne_bytes());
        check_fd(self.fd.0, REQBUFS, rb.as_mut_ptr() as *mut libc::c_void)
            .map_err(|e| CaptureError::ReadFailed(format!("REQBUFS: {e}")))?;
        let count = u32::from_ne_bytes([rb[0], rb[1], rb[2], rb[3]]);
        if count == 0 {
            return Err(CaptureError::Invalid("driver allocated no buffers".into()));
        }

        let mut maps = Vec::with_capacity(count as usize);
        for index in 0..count {
            let mut plane = Plane {
                bytesused: 0,
                length: 0,
                m: 0,
                data_offset: 0,
                reserved: [0; 11],
            };
            let mut buf = Buffer {
                index,
                type_: CAPTURE_MPLANE,
                bytesused: 0,
                flags: 0,
                field: 0,
                timestamp_sec: 0,
                timestamp_usec: 0,
                timecode: [0; 16],
                sequence: 0,
                memory: MEMORY_MMAP,
                m: &mut plane as *mut Plane as u64,
                length: 1,
                reserved2: 0,
                request_fd: 0,
            };
            check_fd(
                self.fd.0,
                QUERYBUF,
                &mut buf as *mut Buffer as *mut libc::c_void,
            )
            .map_err(|e| CaptureError::ReadFailed(format!("QUERYBUF({index}): {e}")))?;
            let length = plane.length as usize;
            let offset = (plane.m & 0xFFFF_FFFF) as i64;
            // SAFETY: length/offset come from QUERYBUF for this fd.
            let ptr = unsafe {
                libc::mmap(
                    std::ptr::null_mut(),
                    length,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_SHARED,
                    self.fd.0,
                    offset,
                )
            };
            if ptr == libc::MAP_FAILED {
                return Err(CaptureError::ReadFailed(format!(
                    "mmap({index}): {}",
                    std::io::Error::last_os_error()
                )));
            }
            maps.push((ptr as usize, length));
        }

        let requeue = Box::new(V4l2Requeue {
            fd: Arc::clone(&self.fd),
        });
        // SAFETY: the regions were just mapped and live until `maps` is dropped
        // after the pool (field order guarantees it).
        let regions = maps
            .iter()
            .map(|(ptr, len)| (*ptr as *mut u8, *len))
            .collect();
        // SAFETY: regions were mapped above and are unmapped only in `Drop`
        // after the pool is gone.
        let pool = unsafe { Pool::from_mmaps(regions, requeue)? };
        for index in 0..count {
            qbuf(self.fd.0, index)
                .map_err(|e| CaptureError::ReadFailed(format!("initial QBUF({index}): {e}")))?;
        }
        check_fd(
            self.fd.0,
            STREAMON,
            &CAPTURE_MPLANE as *const u32 as *mut libc::c_void,
        )
        .map_err(|e| CaptureError::ReadFailed(format!("STREAMON: {e}")))?;

        self.pool = Some(pool);
        self.maps = maps;
        self.streaming = true;
        self.info = SourceInfo {
            geom,
            fps,
            stride,
            vstride,
            format: PixelFormat::Nv12,
        };
        Ok(self.info)
    }

    fn next_frame(&mut self, timeout: Duration) -> Result<Option<FrameLease>, CaptureError> {
        let Some(pool) = self.pool.as_ref() else {
            return Err(CaptureError::Stopped);
        };
        let mut pfd = libc::pollfd {
            fd: self.fd.0,
            events: libc::POLLIN,
            revents: 0,
        };
        let timeout_ms = timeout.as_millis().min(i32::MAX as u128) as i32;
        // SAFETY: one valid pollfd.
        let rc = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
        if rc < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                return Ok(None);
            }
            return Err(CaptureError::ReadFailed(format!("poll: {err}")));
        }
        if rc == 0 {
            return Ok(None);
        }

        let mut plane: Plane = Plane {
            bytesused: 0,
            length: 0,
            m: 0,
            data_offset: 0,
            reserved: [0; 11],
        };
        let mut buf = Buffer {
            index: 0,
            type_: CAPTURE_MPLANE,
            bytesused: 0,
            flags: 0,
            field: 0,
            timestamp_sec: 0,
            timestamp_usec: 0,
            timecode: [0; 16],
            sequence: 0,
            memory: MEMORY_MMAP,
            m: &mut plane as *mut Plane as u64,
            length: 1,
            reserved2: 0,
            request_fd: 0,
        };
        if let Err(e) = check_fd(
            self.fd.0,
            DQBUF,
            &mut buf as *mut Buffer as *mut libc::c_void,
        ) {
            if e.raw_os_error() == Some(libc::EAGAIN) {
                return Ok(None);
            }
            return Err(CaptureError::ReadFailed(format!("DQBUF: {e}")));
        }
        if buf.flags & FLAG_TIMESTAMP_MASK != FLAG_TIMESTAMP_MONOTONIC {
            // H13 measured MONOTONIC; anything else must not be silently mixed
            // into the monotonic timeline.
            qbuf(self.fd.0, buf.index).ok();
            return Err(CaptureError::ReadFailed(format!(
                "driver timestamp is not CLOCK_MONOTONIC (flags {:#010x})",
                buf.flags
            )));
        }
        let ts_ns = buf.timestamp_sec.max(0) as u64 * 1_000_000_000
            + buf.timestamp_usec.max(0) as u64 * 1_000;
        let info = self.info;
        match pool.try_lease_slot(
            buf.index as usize,
            buf.sequence as u64,
            MonoTime(ts_ns),
            info.geom,
            info.stride,
            info.vstride,
        ) {
            Ok(Some(lease)) => Ok(Some(lease)),
            Ok(None) => {
                // Slot already held: hand the buffer back and report the bug.
                qbuf(self.fd.0, buf.index)
                    .map_err(|e| CaptureError::ReadFailed(format!("QBUF after clash: {e}")))?;
                Err(CaptureError::ReadFailed(format!(
                    "buffer {} dequeued while still leased",
                    buf.index
                )))
            }
            Err(e) => {
                qbuf(self.fd.0, buf.index).ok();
                Err(e)
            }
        }
    }

    fn stop(&mut self) {
        if self.streaming {
            // SAFETY: STREAMOFF takes a pointer to the buffer type.
            let rc = unsafe {
                libc::ioctl(
                    self.fd.0,
                    STREAMOFF as _,
                    &CAPTURE_MPLANE as *const u32 as *mut libc::c_void,
                )
            };
            let _ = rc;
            self.streaming = false;
        }
        if let Some(pool) = self.pool.as_ref() {
            pool.stop();
        }
    }

    fn info(&self) -> SourceInfo {
        self.info
    }
}

impl Drop for V4l2Capture {
    fn drop(&mut self) {
        self.stop();
        // Drop the pool (leases, QBUF machinery) before unmapping.
        self.pool = None;
        for (ptr, len) in self.maps.drain(..) {
            // SAFETY: mapped by this struct and not used after this point.
            unsafe { libc::munmap(ptr as *mut libc::c_void, len) };
        }
    }
}

impl V4l2Capture {
    /// Pool statistics (not part of the `FrameSource` contract).
    pub fn stats(&self) -> Option<super::pool::PoolStats> {
        self.pool.as_ref().map(Pool::stats)
    }
}
