//! HIL probe: camera -> MPP H.264/H.265 -> Annex-B file, then check with ffprobe.
//! Build: `deploy/container/build.sh --example encode_probe`
//! Run:   `./encode_probe /dev/video0 1920 1080 30 10000 60 /tmp/out.h264`

#[cfg(target_os = "linux")]
fn main() {
    use action_core::capture::{v4l2::V4l2Capture, FrameSource};
    use action_core::encode::mpp::MppEncoder;
    use action_core::encode::{Codec, EncoderConfig, VideoEncoder};
    use action_core::types::Geometry;
    use std::io::Write;
    use std::time::{Duration, Instant};

    let mut args = std::env::args().skip(1);
    let dev = args.next().unwrap_or_else(|| "/dev/video0".into());
    let w: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(1920);
    let h: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(1080);
    let fps: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(30);
    let kbps: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(10_000);
    let n: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(60);
    let out = args.next().unwrap_or_else(|| "/tmp/out.h264".into());

    let mut cam = V4l2Capture::open(&dev).unwrap_or_else(|e| {
        eprintln!("capture open: {e}");
        std::process::exit(1);
    });
    let info = cam.start(Geometry::new(w, h), fps).unwrap_or_else(|e| {
        eprintln!("capture start: {e}");
        std::process::exit(1);
    });
    println!(
        "capture {}x{} stride={} vstride={}",
        info.geom.w, info.geom.h, info.stride, info.vstride
    );

    // Strides come from the driver, not from the requested geometry: that is
    // the fact that made `feat/encode-blocked` fail (`interfaces.md` §4).
    let mut enc = MppEncoder::open(&EncoderConfig {
        codec: Codec::H264,
        geom: info.geom,
        stride: info.stride,
        vstride: info.vstride,
        bitrate_kbps: kbps,
        fps,
        gop: fps * 2,
        bframes: 0,
    })
    .unwrap_or_else(|e| {
        eprintln!("encoder open: {e}");
        std::process::exit(1);
    });

    let mut file = std::fs::File::create(&out).unwrap();
    let t0 = Instant::now();
    let mut frames = 0usize;
    let mut packets = 0usize;
    let mut bytes = 0usize;
    let mut keyframes = 0usize;

    let mut write = |p: &action_core::encode::EncodedPacket| {
        if p.keyframe {
            keyframes += 1;
        }
        bytes += p.data.len();
        packets += 1;
        file.write_all(&p.data).unwrap();
    };

    while frames < n {
        let lease = match cam.next_frame(Duration::from_millis(200)) {
            Ok(Some(l)) => l,
            Ok(None) => continue,
            Err(e) => {
                eprintln!("next_frame: {e}");
                std::process::exit(1);
            }
        };
        if let Err(e) = enc.submit(lease) {
            eprintln!("submit: {e}");
            std::process::exit(1);
        }
        frames += 1;
        // Drain whatever is ready; the encoder holds the lease until then.
        while let Ok(Some(p)) = enc.poll(Duration::ZERO) {
            write(&p);
        }
    }

    // Drain the tail: `flush` only reports a lifecycle error if inputs are
    // still held after the deadline; the packets stay readable via `poll`.
    match enc.flush(Duration::from_secs(2)) {
        Ok(()) => {}
        Err(e) => eprintln!("flush: {e}"),
    }
    while let Ok(Some(p)) = enc.poll(Duration::ZERO) {
        write(&p);
    }

    file.flush().ok();
    cam.stop();
    let elapsed = t0.elapsed().as_secs_f64();
    println!(
        "done: {frames} frames -> {packets} packets ({keyframes} key) {bytes} B in {elapsed:.3}s = {:.2} fps; enc={:?}; cam={:?}",
        frames as f64 / elapsed,
        enc.stats(),
        cam.stats()
    );
    println!("wrote {out}");
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("encode_probe runs on the board only (linux)");
}
