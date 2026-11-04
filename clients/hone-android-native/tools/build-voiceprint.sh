#!/usr/bin/env bash
# Build crates/hone-voiceprint for Android (JNI) and put libhone_voiceprint.so in jniLibs.
# Needs: rust target aarch64-linux-android, cargo-ndk, NDK 27 ($ANDROID_NDK_HOME or $ANDROID_HOME/ndk/27.2.12479018).
set -euo pipefail
cd "$(dirname "$0")/.."
app=$PWD
export ANDROID_NDK_HOME=${ANDROID_NDK_HOME:-${ANDROID_HOME:-$HOME/Android/Sdk}/ndk/27.2.12479018}
cd ../../crates/hone-voiceprint
cargo ndk -t arm64-v8a -o "$app/app/src/main/jniLibs" build --release --features android
ls -la "$app/app/src/main/jniLibs/arm64-v8a/libhone_voiceprint.so"
