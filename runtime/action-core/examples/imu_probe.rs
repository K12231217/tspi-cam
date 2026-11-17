//! HIL probe: open the IMU, print identity and a few samples (SI + raw).
//! Build for the board with
//! `cargo build --release --target aarch64-unknown-linux-gnu --example imu_probe`,
//! then run: `sudo ./imu_probe /dev/spidev3.0 5`.

#[cfg(unix)]
fn main() {
    use action_contract::imu;
    use action_core::imu::{ImuSource, Lsm6dsr};

    let mut args = std::env::args().skip(1);
    let dev = args.next().unwrap_or_else(|| imu::DEVICE.to_string());
    let n: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(5);

    let mut sensor = Lsm6dsr::new();
    match sensor.open(&dev, imu::ODR_HZ) {
        Ok(info) => println!(
            "open {dev}: WHO_AM_I=0x{:02X} odr={}Hz",
            info.who_am_i, info.odr_hz
        ),
        Err(e) => {
            eprintln!("open failed: {e}");
            std::process::exit(1);
        }
    }
    for i in 0..n {
        match sensor.read() {
            Ok(s) => println!(
                "#{i} t={} gyro(rad/s)=[{:.4},{:.4},{:.4}] accel(m/s2)=[{:.3},{:.3},{:.3}] raw={:?}",
                s.t, s.gyro[0], s.gyro[1], s.gyro[2], s.accel[0], s.accel[1], s.accel[2], s.raw
            ),
            Err(e) => {
                eprintln!("read failed: {e}");
                std::process::exit(1);
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[cfg(not(unix))]
fn main() {
    eprintln!("imu_probe runs on the board only (linux)");
}
