//! Buffer pool: the only place a [`FrameLease`] is constructed, so "same buffer
//! leased twice" and "two QBUF for one drop" are unreachable by construction.
//!
//! # Ownership and accounting
//!
//! - A lease is owned, `Send`, and returns its slot when dropped; the drop path
//!   calls [`Requeue::requeue`] **before** freeing the slot. A failed requeue
//!   poisons the slot instead of handing the same buffer out again.
//! - Slot state is `AtomicU8` (`Free -> Leased -> Free`). Counters are
//!   `AtomicU64`; a `Cell`-style counter loses 5-8% of concurrent updates
//!   (review experiment E2), which would make `available()` and the stats
//!   disagree forever.
//! - `gap_frames` counts *driver-side* sequence gaps only. The application
//!   queue has its own `queue_dropped` counter; the two are never merged.
//!
//! On the board the slot payload is the `mmap`ed V4L2 buffer; on the host it is
//! a heap allocation with the same layout, which makes the accounting testable
//! without a camera.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use action_contract::nv12;

use super::CaptureError;
use crate::types::{FramePlanes, Geometry, MonoTime, PixelFormat};

const FREE: u8 = 0;
const LEASED: u8 = 1;
const POISONED: u8 = 2;

/// Hands a buffer back to the driver (V4L2 `QBUF`). Implemented by the V4L2
/// binding on board and by a recorder in tests.
pub trait Requeue: Send + Sync {
    fn requeue(&self, index: usize) -> Result<(), CaptureError>;
}

struct Slot {
    state: AtomicU8,
    buf: UnsafeCell<Box<[u8]>>,
}

struct SeqTracker {
    seen: bool,
    last: u64,
}

struct Inner {
    slots: Vec<Slot>,
    dequeued: AtomicU64,
    returned: AtomicU64,
    return_errors: AtomicU64,
    gap_frames: AtomicU64,
    stopped: AtomicBool,
    frame_bytes: usize,
    requeue: Box<dyn Requeue>,
    seq: Mutex<SeqTracker>,
}

// SAFETY: `Slot::buf` is only accessed while the slot is `LEASED`, and the
// `compare_exchange` in `Pool::try_lease` guarantees a single owner. The
// counters are atomics. `Pool` hands out `&[u8]` tied to `&FrameLease`, so no
// reference survives the lease.
unsafe impl Send for Inner {}
unsafe impl Sync for Inner {}

/// Snapshot of pool accounting. `available()` is separate because it is read
/// often and must not contend on the stat counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PoolStats {
    pub dequeued: u64,
    pub returned: u64,
    pub return_errors: u64,
    pub gap_frames: u64,
}

/// Shared pool handle; cheap to clone across the capture and encode threads.
#[derive(Clone)]
pub struct Pool {
    inner: Arc<Inner>,
}

impl Pool {
    /// `count` slots, each `frame_bytes` long. Capacity must satisfy
    /// `count >= queue_depth + encoder_in_flight_max + 2` (interfaces.md §3.1);
    /// on this board that is 8 slots and a queue depth of at most 4.
    pub fn new(
        count: u32,
        frame_bytes: usize,
        requeue: Box<dyn Requeue>,
    ) -> Result<Self, CaptureError> {
        if count == 0 || frame_bytes == 0 {
            return Err(CaptureError::Invalid(format!(
                "pool needs count > 0 and frame_bytes > 0 (got {count}, {frame_bytes})"
            )));
        }
        let slots = (0..count)
            .map(|_| Slot {
                state: AtomicU8::new(FREE),
                buf: UnsafeCell::new(vec![0u8; frame_bytes].into_boxed_slice()),
            })
            .collect();
        Ok(Self {
            inner: Arc::new(Inner {
                slots,
                dequeued: AtomicU64::new(0),
                returned: AtomicU64::new(0),
                return_errors: AtomicU64::new(0),
                gap_frames: AtomicU64::new(0),
                stopped: AtomicBool::new(false),
                frame_bytes,
                requeue,
                seq: Mutex::new(SeqTracker {
                    seen: false,
                    last: 0,
                }),
            }),
        })
    }

    /// Take a free slot and build the lease for one dequeued driver frame.
    /// `None` means every slot is busy (the consumer is behind); `Stopped`
    /// means the source was stopped on purpose.
    pub fn try_lease(
        &self,
        id: u64,
        pts: MonoTime,
        geom: Geometry,
        stride: u32,
        vstride: u32,
    ) -> Result<Option<FrameLease>, CaptureError> {
        if self.inner.stopped.load(Ordering::Acquire) {
            return Err(CaptureError::Stopped);
        }
        if stride < geom.w || stride == 0 {
            return Err(CaptureError::BadStride { stride, vstride });
        }
        let luma = nv12::luma_span(stride, vstride);
        let chroma = nv12::chroma_span(stride, vstride);
        let total = luma.1 + chroma.1;
        if total > self.inner.frame_bytes {
            return Err(CaptureError::BadStride { stride, vstride });
        }

        let mut leased = None;
        for (index, slot) in self.inner.slots.iter().enumerate() {
            if slot
                .state
                .compare_exchange(FREE, LEASED, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                leased = Some(index);
                break;
            }
        }
        let Some(slot) = leased else {
            return Ok(None);
        };

        self.record_sequence(id);
        self.inner.dequeued.fetch_add(1, Ordering::Relaxed);
        Ok(Some(FrameLease {
            id,
            pts,
            geom,
            format: PixelFormat::Nv12,
            stride,
            vstride,
            planes: FramePlanes { luma, chroma },
            slot,
            pool: Arc::clone(&self.inner),
        }))
    }

    pub fn available(&self) -> usize {
        self.inner
            .slots
            .iter()
            .filter(|s| s.state.load(Ordering::Acquire) == FREE)
            .count()
    }

    pub fn stats(&self) -> PoolStats {
        PoolStats {
            dequeued: self.inner.dequeued.load(Ordering::Relaxed),
            returned: self.inner.returned.load(Ordering::Relaxed),
            return_errors: self.inner.return_errors.load(Ordering::Relaxed),
            gap_frames: self.inner.gap_frames.load(Ordering::Relaxed),
        }
    }

    /// Only stops new leases. Already handed-out leases stay readable and are
    /// requeued normally; poisoning the pool happens only on requeue failure.
    pub fn stop(&self) {
        self.inner.stopped.store(true, Ordering::Release);
    }

    fn record_sequence(&self, id: u64) {
        let mut seq = self.inner.seq.lock().expect("sequence tracker poisoned");
        if seq.seen {
            if id > seq.last + 1 {
                self.inner
                    .gap_frames
                    .fetch_add(id - seq.last - 1, Ordering::Relaxed);
            }
            seq.last = seq.last.max(id);
        } else {
            seq.seen = true;
            seq.last = id;
        }
    }
}

impl Inner {
    /// SAFETY: caller must hold the slot's lease (state `LEASED`).
    unsafe fn slot_buf(&self, index: usize) -> &[u8] {
        (*self.slots[index].buf.get()).as_ref()
    }

    fn release(&self, index: usize) {
        match self.requeue.requeue(index) {
            Ok(()) => {
                let prior = self.slots[index].state.compare_exchange(
                    LEASED,
                    FREE,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
                if prior.is_ok() {
                    self.returned.fetch_add(1, Ordering::Relaxed);
                } else {
                    self.return_errors.fetch_add(1, Ordering::Relaxed);
                }
            }
            Err(_) => {
                // The driver may still own this buffer: never reuse it.
                self.slots[index].state.store(POISONED, Ordering::Release);
                self.return_errors.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

/// Owned view of one captured frame. Dropping it returns the buffer to the
/// driver exactly once.
pub struct FrameLease {
    /// `v4l2_buffer.sequence`; driver-side gaps are computed from this.
    pub id: u64,
    pub pts: MonoTime,
    pub geom: Geometry,
    pub format: PixelFormat,
    pub stride: u32,
    pub vstride: u32,
    planes: FramePlanes,
    slot: usize,
    pool: Arc<Inner>,
}

impl FrameLease {
    /// `[luma, chroma]`, lengths `stride * vstride` and `stride * vstride / 2`.
    /// The slices borrow `&self`, so they cannot outlive the lease.
    pub fn planes(&self) -> [&[u8]; 2] {
        // SAFETY: this lease is the only owner of `slot`, so the slot is
        // LEASED until `Drop`; the returned slices borrow `self`.
        let base = unsafe { self.pool.slot_buf(self.slot) };
        let (lo, ll) = self.planes.luma;
        let (co, cl) = self.planes.chroma;
        [&base[lo..lo + ll], &base[co..co + cl]]
    }

    /// Plane spans `(offset, len)` for consumers that copy with an explicit
    /// geometry (encoder stride/vstride arguments).
    pub fn plane_spans(&self) -> FramePlanes {
        self.planes
    }

    /// v2 zero-copy export; v1 is mmap-only, so always `None`. Linux-only
    /// because a borrowed fd has no host-wide equivalent type.
    #[cfg(unix)]
    pub fn dmabuf(&self) -> Option<std::os::fd::BorrowedFd<'_>> {
        None
    }
}

impl std::fmt::Debug for FrameLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameLease")
            .field("id", &self.id)
            .field("pts", &self.pts)
            .field("geom", &self.geom)
            .field("stride", &self.stride)
            .field("vstride", &self.vstride)
            .field("slot", &self.slot)
            .finish()
    }
}

impl Drop for FrameLease {
    fn drop(&mut self) {
        self.pool.release(self.slot);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Recorder {
        calls: Mutex<Vec<usize>>,
        fail_on: Option<usize>,
    }

    impl Requeue for Arc<Recorder> {
        fn requeue(&self, index: usize) -> Result<(), CaptureError> {
            self.calls.lock().unwrap().push(index);
            match self.fail_on {
                Some(bad) if bad == index => Err(CaptureError::ReadFailed("EBUSY".into())),
                _ => Ok(()),
            }
        }
    }

    fn recorder() -> (Arc<Recorder>, Box<dyn Requeue>) {
        let rec = Arc::new(Recorder::default());
        (Arc::clone(&rec), Box::new(Arc::clone(&rec)))
    }

    const GEOM: Geometry = Geometry { w: 32, h: 32 };
    const FRAME_BYTES: usize = 32 * 32 * 3 / 2;

    fn pool(count: u32) -> (Pool, Arc<Recorder>) {
        let (rec, requeue) = recorder();
        (Pool::new(count, FRAME_BYTES, requeue).unwrap(), rec)
    }

    fn lease(pool: &Pool, id: u64) -> FrameLease {
        pool.try_lease(id, MonoTime(id), GEOM, 32, 32)
            .unwrap()
            .expect("a slot must be free")
    }

    #[test]
    fn lease_drop_requeues_exactly_once() {
        let (pool, rec) = pool(2);
        assert_eq!(pool.available(), 2);
        let l = lease(&pool, 1);
        assert_eq!(pool.available(), 1);
        let planes = l.planes();
        assert_eq!(planes[0].len(), 32 * 32);
        assert_eq!(planes[1].len(), 32 * 16);
        drop(l);
        assert_eq!(pool.available(), 2);
        assert_eq!(rec.calls.lock().unwrap().len(), 1);
        assert_eq!(pool.stats().dequeued, 1);
        assert_eq!(pool.stats().returned, 1);
        assert_eq!(pool.stats().return_errors, 0);
    }

    #[test]
    fn exhaustion_returns_none_not_an_error() {
        let (pool, _rec) = pool(2);
        let a = lease(&pool, 1);
        let b = lease(&pool, 2);
        assert!(pool
            .try_lease(3, MonoTime(3), GEOM, 32, 32)
            .unwrap()
            .is_none());
        drop(a);
        assert!(pool
            .try_lease(3, MonoTime(3), GEOM, 32, 32)
            .unwrap()
            .is_some());
        drop(b);
        assert_eq!(pool.available(), 2);
        assert_eq!(pool.stats().returned, 3, "a, the retry lease, and b");
    }

    #[test]
    fn driver_gaps_are_counted_separately() {
        let (pool, _rec) = pool(4);
        for id in [1u64, 2, 5] {
            drop(lease(&pool, id));
        }
        assert_eq!(pool.stats().gap_frames, 2, "ids 3 and 4 were dropped");
        // Reordering must not produce negative or duplicate gaps.
        drop(lease(&pool, 5));
        drop(lease(&pool, 6));
        assert_eq!(pool.stats().gap_frames, 2);
    }

    #[test]
    fn failed_requeue_poisons_the_slot() {
        let rec = Arc::new(Recorder {
            fail_on: Some(0),
            ..Recorder::default()
        });
        let pool = Pool::new(2, FRAME_BYTES, Box::new(Arc::clone(&rec))).unwrap();
        drop(lease(&pool, 1));
        assert_eq!(pool.stats().return_errors, 1);
        assert_eq!(pool.stats().returned, 0);
        assert_eq!(pool.available(), 1, "poisoned slot must not come back");
    }

    #[test]
    fn stop_only_blocks_new_leases() {
        let (pool, _rec) = pool(2);
        let held = lease(&pool, 7);
        pool.stop();
        assert!(matches!(
            pool.try_lease(8, MonoTime(8), GEOM, 32, 32),
            Err(CaptureError::Stopped)
        ));
        // The held lease stays readable and returns normally after stop.
        assert_eq!(held.planes()[0].len(), 32 * 32);
        drop(held);
        assert_eq!(pool.stats().returned, 1);
    }

    #[test]
    fn geometry_must_fit_the_slot() {
        let (pool, _rec) = pool(2);
        // stride < width
        assert!(matches!(
            pool.try_lease(1, MonoTime(1), GEOM, 16, 32),
            Err(CaptureError::BadStride { .. })
        ));
        // 64x32 needs more bytes than the 32x32 slot holds
        assert!(matches!(
            pool.try_lease(1, MonoTime(1), Geometry { w: 64, h: 32 }, 64, 32),
            Err(CaptureError::BadStride { .. })
        ));
        assert_eq!(pool.stats().dequeued, 0);
    }

    #[test]
    fn concurrent_lease_drop_accounts_every_cycle() {
        let (pool, rec) = pool(8);
        let threads = 8;
        let per_thread = 1_000;
        let mut handles = Vec::new();
        for _ in 0..threads {
            let pool = pool.clone();
            handles.push(std::thread::spawn(move || {
                let mut done = 0;
                while done < per_thread {
                    match pool.try_lease(0, MonoTime(0), GEOM, 32, 32) {
                        Ok(Some(l)) => {
                            assert_eq!(l.planes()[0].len(), 32 * 32);
                            drop(l);
                            done += 1;
                        }
                        Ok(None) => std::hint::spin_loop(),
                        Err(e) => panic!("unexpected error: {e}"),
                    }
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        let stats = pool.stats();
        assert_eq!(stats.dequeued, (threads * per_thread) as u64);
        assert_eq!(stats.returned, (threads * per_thread) as u64);
        assert_eq!(stats.return_errors, 0);
        assert_eq!(pool.available(), 8);
        assert_eq!(
            rec.calls.lock().unwrap().len(),
            (threads * per_thread) as usize
        );
    }

    #[test]
    fn lease_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<FrameLease>();
        assert_send::<Pool>();
    }
}
