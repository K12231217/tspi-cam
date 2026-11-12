//! MIPI OV5695 capture adapter (NV12).
//!
//! TODO (M1 B段, needs the module on the board, H8): open the V4L2 node,
//! request NV12 buffers, verify the driver timestamp's clock domain and
//! semantics (SOF / EOF / dequeue, H2), and return owned [`Frame`]s.

use super::{CaptureAdapter, CaptureError};
use crate::types::{Frame, PixelFormat};

pub struct MipiNv12Adapter {
    device: String,
    width: u32,
    height: u32,
    fps: u32,
    next_frame_id: u64,
}

impl MipiNv12Adapter {
    pub fn new() -> Self {
        Self {
            device: String::new(),
            width: 0,
            height: 0,
            fps: 0,
            next_frame_id: 0,
        }
    }
}

impl Default for MipiNv12Adapter {
    fn default() -> Self {
        Self::new()
    }
}

impl CaptureAdapter for MipiNv12Adapter {
    fn open(
        &mut self,
        device: &str,
        width: u32,
        height: u32,
        fps: u32,
    ) -> Result<(), CaptureError> {
        self.device = device.to_string();
        self.width = width;
        self.height = height;
        self.fps = fps;
        Err(CaptureError::OpenFailed(
            device.to_string(),
            "MIPI adapter not implemented yet (M1 B段, H8)".to_string(),
        ))
    }

    fn next_frame(&mut self) -> Result<Frame, CaptureError> {
        let _ = self.next_frame_id;
        Err(CaptureError::ReadFailed(
            "MIPI adapter not implemented yet (M1 B段, H8)".to_string(),
        ))
    }

    fn close(&mut self) {}
}

// Referenced so the import is not dead while the adapter is a stub.
#[allow(dead_code)]
fn _format_is_nv12() -> PixelFormat {
    PixelFormat::Nv12
}
