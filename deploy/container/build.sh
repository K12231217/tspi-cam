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

BASE_IMAGE=${BASE_IMAGE:-ubuntu:20.04}

# Proxy support (e.g. HTTPS_PROXY=http://127.0.0.1:7890 on a mirrored-network WSL):
# podman pulls through the host proxy, and --network=host lets the build steps
# (apt/rustup) reach the same endpoint.
PROXY_ARGS=()
if [ -n "${HTTPS_PROXY:-}${https_proxy:-}" ]; then
    P=${HTTPS_PROXY:-$https_proxy}
    PROXY_ARGS+=(--build-arg "HTTP_PROXY=$P" --build-arg "HTTPS_PROXY=$P"
                 --build-arg "http_proxy=$P" --build-arg "https_proxy=$P"
                 --build-arg "NO_PROXY=${NO_PROXY:-localhost,127.0.0.1}")
fi

"$RUNTIME" build ${PROXY_ARGS[@]+"${PROXY_ARGS[@]}"} --network=host \
    -q --build-arg BASE_IMAGE="$BASE_IMAGE" -t "$IMAGE" deploy/container

exec "$RUNTIME" run --rm --network=host \
    -e HTTP_PROXY -e HTTPS_PROXY -e NO_PROXY \
    -v "$PWD":/src \
    -v tspi-cam-cargo:/usr/local/cargo \
    -v tspi-cam-rustup:/usr/local/rustup \
    -w /src \
    "$IMAGE" \
    cargo build --release --target aarch64-unknown-linux-gnu "$@"
