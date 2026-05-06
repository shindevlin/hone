#!/bin/bash
# Build ZeroClaw with browser-native feature enabled
set -e

ZEROCLAW_DIR="$HOME/repos/zeroclaw"

if [ ! -d "$ZEROCLAW_DIR" ]; then
  echo "ERROR: ZeroClaw not found at $ZEROCLAW_DIR"
  exit 1
fi

echo "Building ZeroClaw with browser-native support..."
cd "$ZEROCLAW_DIR"
cargo build --release --features browser-native

echo "Build complete: $ZEROCLAW_DIR/target/release/zeroclaw"
