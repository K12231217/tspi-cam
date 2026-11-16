#!/usr/bin/env bash
# Build board binaries in the Ubuntu 20.04 container.
# Usage: deploy/container/build.sh [cargo args...]
#   deploy/container/build.sh --example capture_probe
#   deploy/container/build.sh -p actiond
# Extra env: CONTAINER_RUNTIME=podman
set -euo pipefail
cd "$(dirname "$0")/../.."

RUNTIME=${CONTAINER_RUNTIME:-docker}
IMAGE=tspi-cam-build:20.04

if ! command -v "$RUNTIME" >/dev/null 2>&1; then
    echo "error: $RUNTIME not found (install docker.io or podman)" >&2
    exit 1
fi

"$RUNTIME" build -q -t "$IMAGE" deploy/container

exec "$RUNTIME" run --rm \
    -v "$PWD":/src \
    -v tspi-cam-cargo:/usr/local/cargo \
    -v tspi-cam-rustup:/usr/local/rustup \
    -w /src \
    "$IMAGE" \
    cargo build --release --target aarch64-unknown-linux-gnu "$@"
