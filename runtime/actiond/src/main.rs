//! `actiond`: the recording daemon. Sole owner of the camera, IMU and
//! storage; owns the recording state machine. Clients (UI, CLI) only send
//! requests over the control socket. See docs/architecture.md §3, §7.
//!
//! Skeleton (M1 A段 step 1): loads config, takes the `flock()` on
//! `/run/tspi-cam/device.lock` before opening any hardware, then parks.
//! State machine, manager, IPC server and capture loop land in later steps.

use action_config::Config;
use std::fs::OpenOptions;
use std::io;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

const DEFAULT_CONFIG: &str = "/etc/tspi-cam/actiond.toml";
const DEVICE_LOCK: &str = "/run/tspi-cam/device.lock";

fn lock_device() -> io::Result<std::fs::File> {
    // Exclusive, non-blocking flock. The kernel releases it if we crash;
    // a leftover lock file means nothing by itself (architecture §11).
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o644)
        .open(DEVICE_LOCK)?;
    use std::os::unix::io::AsRawFd;
    let ret = libc_flock_ex_nb(file.as_raw_fd());
    if ret != 0 {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "device lock is held by another process (actiond or actioncal)",
        ));
    }
    Ok(file)
}

/// Minimal `flock(LOCK_EX | LOCK_NB)` without a libc dependency.
fn libc_flock_ex_nb(fd: i32) -> i32 {
    // LOCK_EX = 2, LOCK_NB = 4. Implemented via the flock syscall through
    // libc symbols linked by std on Linux.
    extern "C" {
        fn flock(fd: i32, operation: i32) -> i32;
    }
    unsafe { flock(fd, 2 | 4) }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_CONFIG));
    let config = Config::load(&config_path).unwrap_or_else(|e| {
        eprintln!("warning: cannot load {config_path:?} ({e}); using defaults");
        Config::default()
    });
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(&config.log.level))
        .init();
    log::info!(
        "actiond starting (skeleton): camera={} imu={}",
        config.camera.device,
        config.imu.device
    );

    // Lock before opening any hardware (architecture §11).
    let _lock = match lock_device() {
        Ok(f) => f,
        Err(e) => {
            log::error!("cannot acquire device lock: {e}");
            std::process::exit(1);
        }
    };
    log::info!("device lock acquired; hardware not opened yet (skeleton)");

    // TODO (M1 A段 later steps): state machine + manager + IPC server +
    // capture loop. For now the daemon parks until killed.
    log::info!("parking (skeleton build, no recording yet)");
    loop {
        std::thread::park();
    }
}
