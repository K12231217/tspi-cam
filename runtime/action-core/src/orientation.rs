//! Orientation estimation from IMU samples (VQF, 6D mode).
//!
//! Thin wrapper over the vendored `action-vqf` crate. Feeds each
//! [`ImuSample`] (rad/s, m/s²) into the filter and exposes the 6D
//! orientation quaternion, the gyro bias estimate, and rest detection.
//! No magnetometer: the filter runs with `mag = None`, so only the
//! inclination + gyro integration path is active (architecture §6.4
//! already standardizes on rad/s and m/s², which is exactly what VQF
//! expects).

use crate::types::ImuSample;
use vqf_rs::VQF;

/// Orientation estimator running VQF in 6D (magnetometer-free) mode.
pub struct OrientationEstimator {
    vqf: VQF,
}

impl OrientationEstimator {
    /// Create a new estimator for the given IMU sample period in seconds
    /// (0.005 s at the 200 Hz target rate).
    pub fn new(sample_period_s: f64) -> Self {
        Self {
            vqf: VQF::new(sample_period_s, None, None, None),
        }
    }

    /// Feed one IMU sample. Returns the 6D orientation quaternion
    /// `[w, x, y, z]` after the update.
    pub fn update(&mut self, sample: &ImuSample) -> [f64; 4] {
        self.vqf.update(
            [sample.gx as f64, sample.gy as f64, sample.gz as f64],
            [sample.ax as f64, sample.ay as f64, sample.az as f64],
            None,
        );
        self.quat_6d()
    }

    /// Current 6D orientation quaternion `[w, x, y, z]`.
    pub fn quat_6d(&self) -> [f64; 4] {
        let q = self.vqf.quat_6d();
        [q.0, q.1, q.2, q.3]
    }

    /// Current gyro bias estimate (rad/s) and its uncertainty (rad/s).
    pub fn bias_estimate(&self) -> ([f64; 3], f64) {
        self.vqf.bias_estimate()
    }

    /// Whether the device is currently detected as at rest.
    pub fn rest_detected(&self) -> bool {
        self.vqf.rest_detected()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ImuSample;

    #[test]
    fn static_sample_converges_to_identity_and_rest() {
        let mut est = OrientationEstimator::new(0.005);
        let sample = ImuSample {
            timestamp_ns: 0,
            gx: 0.0,
            gy: 0.0,
            gz: 0.0,
            ax: 0.0,
            ay: 0.0,
            az: 9.81,
        };
        let mut q = [1.0, 0.0, 0.0, 0.0];
        for _ in 0..600 {
            // 3 s at 200 Hz, level and static.
            q = est.update(&sample);
        }
        assert!((q[0] - 1.0).abs() < 1e-3, "w = {}", q[0]);
        assert!(
            q[1].abs() < 1e-3 && q[2].abs() < 1e-3 && q[3].abs() < 1e-3,
            "q = {q:?}"
        );
        assert!(
            est.rest_detected(),
            "rest should be detected after 3 s static"
        );
    }
}
