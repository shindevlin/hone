#!/usr/bin/env bash
# macOS universal build. Run ON A MAC (Xcode command line tools, rustup, cmake, git).
#   ./build-macos.sh <tensorflow-src> <assets-dir> <ffmpeg-universal-binary> <out-dir>
# Produces:
#   hone-listener-<ver>-macos-universal          bare binary (auto-update artifact)
#   hone-listener-<ver>-macos-universal.tar.gz   used by install.sh (curl one-liner)
#   HONE Birds Listener.app + HONE-Birds-Listener-<ver>.dmg
# Notarized .dmg: set DEVELOPER_ID ("Developer ID Application: NAME (TEAMID)") and
# NOTARY_PROFILE (xcrun notarytool store-credentials name). Without them the .dmg is unsigned
# (blocked: needs Josh's Apple Developer account).
set -euo pipefail
TF=$1 ASSETS=$2 FFMPEG=$3 OUT=$4
HERE=$(cd "$(dirname "$0")" && pwd); CRATE="$HERE/../.."
VER=$(grep -m1 '^version' "$CRATE/Cargo.toml" | cut -d'"' -f2)
mkdir -p "$OUT"; W=$(mktemp -d)
rustup target add aarch64-apple-darwin x86_64-apple-darwin
export MACOSX_DEPLOYMENT_TARGET=12.0
for a in aarch64 x86_64; do
  (cd "$CRATE" && cargo build --release --locked --features tray --target $a-apple-darwin)
  ca=$([ $a = aarch64 ] && echo arm64 || echo x86_64)
  cmake -S "$TF/tensorflow/lite/c" -B "$W/tfl-$a" -DCMAKE_BUILD_TYPE=Release -DCMAKE_OSX_ARCHITECTURES=$ca \
    -DTFLITE_ENABLE_XNNPACK=ON -DTFLITE_ENABLE_GPU=OFF -DCMAKE_POLICY_VERSION_MINIMUM=3.5
  cmake --build "$W/tfl-$a" -j 8
done
T="${CARGO_TARGET_DIR:-$CRATE/target}"
lipo -create "$T/aarch64-apple-darwin/release/hone-listener" "$T/x86_64-apple-darwin/release/hone-listener" -output "$W/hone-listener"
lipo -create "$W/tfl-aarch64/libtensorflowlite_c.dylib" "$W/tfl-x86_64/libtensorflowlite_c.dylib" -output "$W/libtensorflowlite_c.dylib"
lipo -info "$W/hone-listener" "$W/libtensorflowlite_c.dylib"
cp "$W/hone-listener" "$OUT/hone-listener-$VER-macos-universal"
# flat bundle for install.sh (~/Library/Application Support/HONE Listener/app)
F="$W/flat"; mkdir -p "$F"
cp "$W/hone-listener" "$W/libtensorflowlite_c.dylib" "$FFMPEG" "$F/"; chmod +x "$F/ffmpeg" "$F/hone-listener"
for f in perch_v2_8bit.tflite class_names.json names_en.json NOTICE.txt; do cp "$ASSETS/$f" "$F/"; done
tar -czf "$OUT/hone-listener-$VER-macos-universal.tar.gz" -C "$F" .
# .app (menu-bar only) + .dmg
APP="$W/HONE Birds Listener.app"; mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$W/hone-listener" "$APP/Contents/MacOS/"
cp "$W/libtensorflowlite_c.dylib" "$FFMPEG" "$ASSETS"/{perch_v2_8bit.tflite,class_names.json,names_en.json,NOTICE.txt} "$APP/Contents/Resources/"
sed "s/@VERSION@/$VER/g" "$HERE/Info.plist" > "$APP/Contents/Info.plist"
if [ -n "${DEVELOPER_ID:-}" ]; then
  codesign --force --options runtime --timestamp --sign "$DEVELOPER_ID" "$APP/Contents/Resources/libtensorflowlite_c.dylib" "$APP/Contents/Resources/ffmpeg"
  codesign --force --options runtime --timestamp --entitlements "$HERE/entitlements.plist" --sign "$DEVELOPER_ID" "$APP"
fi
DMG="$OUT/HONE-Birds-Listener-$VER.dmg"
hdiutil create -volname "HONE Birds Listener" -srcfolder "$APP" -ov -format UDZO "$DMG"
if [ -n "${DEVELOPER_ID:-}" ] && [ -n "${NOTARY_PROFILE:-}" ]; then
  codesign --force --timestamp --sign "$DEVELOPER_ID" "$DMG"
  xcrun notarytool submit "$DMG" --keychain-profile "$NOTARY_PROFILE" --wait
  xcrun stapler staple "$DMG"
else
  echo "NOTE: $DMG is NOT signed or notarized (needs the Apple Developer account)."
fi
ls -la "$OUT"
