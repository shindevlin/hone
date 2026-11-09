#!/bin/bash
# Emulator-only x86_64 native libraries for HONE mode (h3-java, hone_miner, hone_voiceprint).
# Output goes to app/src/emulator/jniLibs/x86_64 (gitignored) and is packaged ONLY by
# ./gradlew -Pemulator assembleDebug. Phone builds never include x86_64 (see build.gradle.kts).
# Needs: rustup target x86_64-linux-android (toolchain 1.90.0), cargo-ndk, NDK 27, cmake, JDK 17.
set -eu
export PATH=$HOME/.cargo/bin:$PATH
export ANDROID_NDK_HOME=${ANDROID_NDK_HOME:-$HOME/Android/Sdk/ndk/27.2.12479018}
NDKBIN=$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin
APP=$(cd "$(dirname "$0")/.." && pwd); REPO=$(cd "$APP/../.." && pwd)
OUT=$APP/app/src/emulator/jniLibs/x86_64; mkdir -p "$OUT"
W=${EMU_WORK:-$HOME/aa-emu}; mkdir -p "$W"
rustup +1.90.0 target add x86_64-linux-android >/dev/null
# h3-java JNI for x86_64 (the h3 jar only ships Android ARM builds)
[ -d "$W/h3-java" ] || git clone -q --depth 1 -b v4.5.1 https://github.com/uber/h3-java "$W/h3-java"
REF=$(grep h3.git.reference "$W/h3-java/h3version.properties" | cut -d= -f2)
[ -d "$W/h3" ] || git clone -q --depth 1 -b "$REF" https://github.com/uber/h3 "$W/h3"
mkdir -p "$W/h3/hostb" && (cd "$W/h3/hostb" && cmake -DBUILD_TESTING=OFF -DENABLE_FORMAT=OFF -DENABLE_LINTING=OFF -DENABLE_DOCS=OFF \
  -DBUILD_BENCHMARKS=OFF -DBUILD_FILTERS=OFF -DBUILD_GENERATORS=OFF -DBUILD_FUZZERS=OFF .. >/dev/null)
(cd "$W/h3-java" && [ -f src/main/c/h3-java/src/com_uber_h3core_NativeMethods.h ] || \
  javac -h src/main/c/h3-java/src -d "$W/h3jh" -sourcepath src/main/java src/main/java/com/uber/h3core/NativeMethods.java)
INC=$(dirname "$(find "$W/h3/hostb" -name h3api.h | head -1)")
"$NDKBIN/clang" --target=x86_64-linux-android26 -O2 -fPIC -shared -std=c11 -w -Wl,-z,max-page-size=16384 \
  -I"$INC" -I"$W/h3/src/h3lib/include" "$W"/h3/src/h3lib/lib/*.c "$W/h3-java/src/main/c/h3-java/src/jniapi.c" -lm -o "$OUT/libh3-java.so"
# Rust cores (gcc 15 + RocksDB needs <cstdint>)
(cd "$REPO/rust/hone-android" && CXXFLAGS="-include cstdint" nice cargo ndk -t x86_64 build --release -q)
cp "$REPO/rust/hone-android/target/x86_64-linux-android/release/libhone_miner.so" "$OUT/"
(cd "$REPO/crates/hone-voiceprint" && nice cargo ndk -t x86_64 build --release --features android -q)
cp "$REPO/crates/hone-voiceprint/target/x86_64-linux-android/release/libhone_voiceprint.so" "$OUT/"
ls -la "$OUT"
