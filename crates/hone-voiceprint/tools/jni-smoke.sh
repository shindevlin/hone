#!/usr/bin/env bash
# Host check of the JNI binding: builds the host cdylib with the android feature and calls every
# entry point from a JVM class with the same name as the Kotlin wrapper.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release --features android
out=$(mktemp -d)
javac -d "$out" jni-test/network/hone/app/sound/Voiceprints.java
java -cp "$out" network.hone.app.sound.Voiceprints "$PWD/target/release/libhone_voiceprint.so"
