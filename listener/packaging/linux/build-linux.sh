#!/usr/bin/env bash
# Release builds for Linux x86_64 and Raspberry Pi arm64 (cross), then the .debs.
# usage: build-linux.sh <tflite-build-root> <assets-dir> <out-dir>
set -euo pipefail
TFL=$1 ASSETS=$2 OUT=$3
HERE=$(cd "$(dirname "$0")" && pwd)
CRATE="$HERE/../.."
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/mnt/nvme-storage/hone-listener-build/target}"
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc
export CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc
cd "$CRATE"
cargo build --release --locked --target x86_64-unknown-linux-gnu
cargo build --release --locked --target aarch64-unknown-linux-gnu
"$HERE/build-deb.sh" amd64 "$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/release/hone-listener" "$TFL/tfl-x86_64/libtensorflowlite_c.so" "$ASSETS" "$OUT"
"$HERE/build-deb.sh" arm64 "$CARGO_TARGET_DIR/aarch64-unknown-linux-gnu/release/hone-listener" "$TFL/tfl-aarch64/libtensorflowlite_c.so" "$ASSETS" "$OUT"
echo LINUX_BUILD_DONE
