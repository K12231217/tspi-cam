#!/usr/bin/env bash
# Dev checks: format + clippy, all workspace members.
# NOTE: action-vqf is vendored upstream code and is intentionally NOT
# formatted (its file must stay byte-identical to upstream except the
# attribution header); its clippy lints are allowed in its own Cargo.toml.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo fmt -p actiond -p actionctl -p tspi-cam-ui -p action-core -p action-ipc -p action-config -p action-contract -p actioncal -p xtask -- --check
cargo clippy --workspace --all-targets -- -D warnings
echo "fmt + clippy OK"
