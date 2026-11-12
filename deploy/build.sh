#!/usr/bin/env bash
# Cross-compile tspi-cam for the Taishan Pi (aarch64) on a Windows PC
# (or any Linux PC). Primary route: cargo-zigbuild.
#   cargo install cargo-zigbuild
#   rustup target add aarch64-unknown-linux-gnu
#   ./deploy/build.sh
# Fallback: WSL2 / Linux PC with gcc-aarch64-linux-gnu, then
#   cargo build --release --target aarch64-unknown-linux-gnu
# Binaries land in build/bin/. The board never compiles (2 GB RAM).
set -euo pipefail
cd "$(dirname "$0")/.."

if ! command -v cargo-zigbuild >/dev/null 2>&1; then
    echo "cargo-zigbuild not found. Install it first:" >&2
    echo "  cargo install cargo-zigbuild" >&2
    echo "  rustup target add aarch64-unknown-linux-gnu" >&2
    exit 1
fi

cargo zigbuild --release --target aarch64-unknown-linux-gnu
mkdir -p build/bin
for bin in actiond actionctl tspi-cam-ui actioncal; do
    cp "target/aarch64-unknown-linux-gnu/release/$bin" build/bin/
done
echo "built: build/bin/{actiond,actionctl,tspi-cam-ui,actioncal}"
