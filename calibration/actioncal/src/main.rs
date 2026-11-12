//! `actioncal`: calibration tool. Takes exclusive hardware access via the
//! same `flock()` as `actiond` (`/run/tspi-cam/device.lock`); refused
//! while the daemon holds it. Writes `calibration.json`.
//! Skeleton: lock attempt only; calibration routines land in M2/M3.

fn main() {
    // TODO (M2): try flock, refuse if held; IMU bias + camera intrinsics.
    eprintln!("actioncal: calibration tool is M2, not implemented in this skeleton");
    std::process::exit(0);
}
