# action-vqf — vendored VQF orientation estimation

This crate is a **vendored copy** of [`vqf-rs` 0.3.0](https://crates.io/crates/vqf-rs)
([source](https://github.com/vgskye/vqf-rs)), a Rust port of the VQF
(Versatile Quaternion-based Filter) algorithm by Daniel Laidig and Thomas
Seel ([original C++](https://github.com/dlaidig/vqf),
"VQF: Highly Accurate IMU Orientation Estimation with Bias Estimation and
Magnetic Disturbance Rejection", *Information Fusion* 2023, 91, 187–204).

## Why vendored

Decided 2026-11-12 (user choice): vendor the source instead of depending on
the crate. `vqf-rs` is a single-maintainer crate with no tests and no CI;
vendoring keeps the algorithm readable and modifiable in our own tree, and
matches the project's "no black box" rule. Both the original algorithm and
the port are MIT licensed (see `LICENSE-MIT`).

## What is vendored

- `src/lib.rs` — verbatim upstream `src/lib.rs`, except for the attribution
  header prepended at the top.
- `katex.html` — needed by a `doc = include_str!` attribute in `lib.rs`.
- `LICENSE-MIT` — upstream MIT license text.

The lib target keeps the upstream name `vqf_rs`, so the code (including its
doctest) works unchanged: `use vqf_rs::VQF;`.

## What tspi-cam uses

**6D only.** Our IMU (IMU660RB) has no magnetometer, so we call
`update(gyr, acc, None)` and read `quat_6d()`. The magnetometer / 9D code
is still in the file (it is part of the upstream port) but is never called
by our code. If a magnetometer is ever added, the 9D path is already there.

The thin integration wrapper lives in `action-core` (`orientation.rs`):
it feeds `ImuSample`s (rad/s, m/s²) into the filter at the IMU rate and
exposes the 6D quaternion, the gyro bias estimate, and rest detection.

## Updating

To update to a newer upstream version:

1. Download the new crate file from crates.io and extract `src/lib.rs`
   (and `katex.html` / `LICENSE` if they changed).
2. Diff it against the current `src/lib.rs` (ignoring the header) and
   review the changes.
3. Re-run the validation test (`cargo test -p action-vqf`): it compares
   the filter output against the upstream reference dataset
   (`tests/fixtures/`, generated from
   `https://github.com/dlaidig/vqf/blob/main/tests/imu_data.npz`).

## Validation

`cargo test -p action-vqf` runs `tests/validation_6d.rs`, which feeds the
first 6000 samples of the upstream test dataset (rest + motion onset,
285.71 Hz) through the filter in 6D mode and compares `quat_6d` against
reference outputs produced by the official Python package (`pip install
vqf`). Tolerance is 1e-6 per component, matching the upstream
implementation-comparison test.
