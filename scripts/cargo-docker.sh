#!/usr/bin/env bash
# Run cargo inside Docker for hosts without a native Rust toolchain
# (e.g. thevault). On a machine with rustup, just use cargo directly.
# Usage: scripts/cargo-docker.sh test --workspace
set -euo pipefail
dir="$(cd "$(dirname "$0")/.." && pwd)"
docker run --rm \
  -v "$dir":/work -w /work \
  -v openporta-rustup:/usr/local/rustup \
  -v openporta-cargo:/usr/local/cargo \
  -v openporta-target:/ctarget -e CARGO_TARGET_DIR=/ctarget \
  rust:1-slim sh -c "cargo $*; s=\$?; chown -R $(id -u):$(id -g) /work; exit \$s"
