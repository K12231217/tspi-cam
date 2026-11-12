//! Frame distributor: one captured frame goes to the video queue and to
//! the preview slot (latest frame only). Never blocks capture.
//!
//! TODO (M1 A段 step 4): bounded video queue (8 frames AND 64 MiB,
//! first limit wins) + latest-frame preview slot.

use crate::types::Frame;

/// Distributes captured frames to the recorder and to preview.
pub struct FrameDistributor;

impl FrameDistributor {
    pub fn new() -> Self {
        Self
    }

    /// Hand a frame to the video queue and overwrite the preview slot.
    /// Returns `true` when the frame was accepted into the video queue,
    /// `false` when it was dropped because the queue is full (the caller
    /// counts it in `frames_dropped`).
    pub fn distribute(&self, _frame: Frame) -> bool {
        // TODO: implement in M1 A段 step 4.
        false
    }
}

impl Default for FrameDistributor {
    fn default() -> Self {
        Self::new()
    }
}
