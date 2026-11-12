//! Validate the vendored VQF port against the upstream C++ reference.
//!
//! Fixtures: first 6000 samples of the upstream test dataset
//! (`tests/imu_data.npz` from https://github.com/dlaidig/vqf, MIT;
//! rest 500-2000, motion from 3000), 285.71 Hz, 6D mode (no magnetometer).
//! Reference outputs were produced by the C++ reference implementation
//! (see `tests/ref_driver.cpp`); input CSVs by `tests/gen_input.py`.
//! Tolerance is 1e-6 per quaternion component, matching the upstream
//! implementation-comparison test (`assert_almost_equal`, 6 decimals).
//!
//! Set `VQF_FIXTURE_DIR` to point at a directory with
//! `imu_6d_input.csv` / `quat6d_ref.csv` / `bias_ref.csv` to run the same
//! check on other data (e.g. the full 13500-sample dataset).

use std::path::PathBuf;
use vqf_rs::VQF;

const TS: f64 = 1.0 / 285.7142857142857;
const TOL: f64 = 1e-6;

fn fixture_dir() -> PathBuf {
    std::env::var("VQF_FIXTURE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures"))
}

fn read_csv(path: &PathBuf) -> Vec<Vec<f64>> {
    let text =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {path:?}: {e}"));
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            l.split(',')
                .map(|v| {
                    v.trim()
                        .parse::<f64>()
                        .unwrap_or_else(|e| panic!("bad number {v:?}: {e}"))
                })
                .collect()
        })
        .collect()
}

fn run_filter(input: &[Vec<f64>]) -> (Vec<[f64; 4]>, ([f64; 3], f64)) {
    let mut vqf = VQF::new(TS, None, None, None);
    let mut quats = Vec::with_capacity(input.len());
    for row in input {
        vqf.update([row[0], row[1], row[2]], [row[3], row[4], row[5]], None);
        let q = vqf.quat_6d();
        quats.push([q.0, q.1, q.2, q.3]);
    }
    (quats, vqf.bias_estimate())
}

#[test]
fn quat_6d_matches_reference() {
    let dir = fixture_dir();
    let input = read_csv(&dir.join("imu_6d_input.csv"));
    let reference = read_csv(&dir.join("quat6d_ref.csv"));
    assert_eq!(
        input.len(),
        reference.len(),
        "input/reference row count differs"
    );
    let (quats, _) = run_filter(&input);
    let mut max_err = 0.0f64;
    for (i, (q, r)) in quats.iter().zip(reference.iter()).enumerate() {
        // Sign-insensitive: q and -q describe the same orientation.
        let err_pos = (0..4).map(|k| (q[k] - r[k]).abs()).fold(0.0f64, f64::max);
        let err_neg = (0..4).map(|k| (q[k] + r[k]).abs()).fold(0.0f64, f64::max);
        let err = err_pos.min(err_neg);
        max_err = max_err.max(err);
        assert!(err < TOL, "sample {i}: quat error {err:e} exceeds {TOL:e}");
    }
    eprintln!(
        "max quat component error over {} samples: {max_err:e}",
        input.len()
    );
}

#[test]
fn final_bias_matches_reference() {
    let dir = fixture_dir();
    let input = read_csv(&dir.join("imu_6d_input.csv"));
    let bias_ref = read_csv(&dir.join("bias_ref.csv"));
    assert_eq!(bias_ref.len(), 1, "bias_ref.csv must hold exactly one row");
    let (_, (bias, sigma)) = run_filter(&input);
    let r = &bias_ref[0];
    for k in 0..3 {
        let err = (bias[k] - r[k]).abs();
        assert!(err < TOL, "bias[{k}] error {err:e} exceeds {TOL:e}");
    }
    let sigma_err = (sigma - r[3]).abs();
    assert!(
        sigma_err < TOL,
        "bias sigma error {sigma_err:e} exceeds {TOL:e}"
    );
    eprintln!("final bias {bias:?} sigma {sigma:e} (ref {r:?})");
}
