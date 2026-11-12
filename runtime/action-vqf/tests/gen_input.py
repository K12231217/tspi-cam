#!/usr/bin/env python3
"""Export VQF validation inputs from the upstream test dataset.

Reads tests/imu_data.npz from https://github.com/dlaidig/vqf (MIT) and
writes float64 CSVs the C++ reference driver and the Rust test consume:
rows are gx,gy,gz,ax,ay,az (rad/s, m/s^2), matching the upstream test's
`astype(float)` cast.

Usage:
    python3 gen_input.py /path/to/imu_data.npz OUT_DIR
Writes OUT_DIR/imu_6d_input.csv (first 6000 samples: rest + motion onset)
and OUT_DIR/imu_full_input.csv (all 13500 samples, one-off full check).
"""
import sys

import numpy as np

def main() -> None:
    npz_path, out_dir = sys.argv[1], sys.argv[2]
    d = np.load(npz_path)
    gyr = d["gyr"].astype(np.float64)
    acc = d["acc"].astype(np.float64)
    rate = float(d["sampling_rate"])
    print(f"rate={rate} Hz ts={1.0 / rate:.17g} samples={len(gyr)}")
    data = np.hstack([gyr, acc])
    np.savetxt(f"{out_dir}/imu_6d_input.csv", data[:6000], delimiter=",", fmt="%.17g")
    np.savetxt(f"{out_dir}/imu_full_input.csv", data, delimiter=",", fmt="%.17g")
    print("wrote fixture (6000 rows) and full (13500 rows) inputs")

if __name__ == "__main__":
    main()
