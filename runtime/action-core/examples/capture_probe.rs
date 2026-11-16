//! HIL probe: stream NV12 frames from /dev/video0 and print per-frame facts.
//! Build: `cargo build --release --target aarch64-unknown-linux-musl --example capture_probe`
//! Run:   `./capture_probe /dev/video0 1920 1080 30 20`

#[cfg(target_os = "linux")]
fn main() {
    use action_core::capture::{v4l2::V4l2Capture, FrameSource};
    use action_core::types::Geometry;
    use std::time::{Duration, Instant};

    let mut args = std::env::args().skip(1);
    let dev = args.next().unwrap_or_else(|| "/dev/video0".to_string());
    let w: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(1920);
    let h: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(1080);
    let fps: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(30);
    let n: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(20);

    let mut cam = match V4l2Capture::open(&dev) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("open {dev}: {e}");
            std::process::exit(1);
        }
    };
    let info = match cam.start(Geometry::new(w, h), fps) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("start: {e}");
            std::process::exit(1);
        }
    };
    println!(
        "started {}x{} fps={} format={:?} stride={} vstride={}",
        info.geom.w, info.geom.h, info.fps, info.format, info.stride, info.vstride
    );

    let t0 = Instant::now();
    let mut got = 0usize;
    while got < n {
        match cam.next_frame(Duration::from_millis(200)) {
            Ok(Some(lease)) => {
                let planes = lease.planes();
                if got < 3 || got + 1 == n {
                    println!(
                        "frame id={} pts={} luma={}B chroma={}B age_ms={:.2}",
                        lease.id,
                        lease.pts,
                        planes[0].len(),
                        planes[1].len(),
                        t0.elapsed().as_secs_f64() * 1000.0
                    );
                }
                got += 1;
            }
            Ok(None) => {}
            Err(e) => {
                eprintln!("next_frame: {e}");
                std::process::exit(1);
            }
        }
    }
    let elapsed = t0.elapsed().as_secs_f64();
    cam.stop();
    if let Some(stats) = cam.stats() {
        println!(
            "done: {got} frames in {elapsed:.3}s = {:.2} fps; stats={:?}",
            got as f64 / elapsed,
            stats
        );
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("capture_probe runs on the board only (linux)");
}
