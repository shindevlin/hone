#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
WEBSITE_DIR="$PROJECT_DIR/../btcpc/website"

cd "$PROJECT_DIR"

# Check for Android SDK
if [ -z "${ANDROID_HOME:-}" ] && [ -z "${ANDROID_SDK_ROOT:-}" ]; then
  echo "ERROR: ANDROID_HOME or ANDROID_SDK_ROOT is not set."
  exit 1
fi

# Check for keystore
if [ -z "${BTCPC_KEYSTORE:-}" ]; then
  echo "ERROR: BTCPC_KEYSTORE env var not set (path to release keystore)."
  echo "Generate one with:"
  echo "  keytool -genkey -v -keystore btcpc-release.jks -keyalg RSA -keysize 2048 -validity 10000 -alias btcpc"
  exit 1
fi

echo "Syncing web assets to Android project..."
npx cap sync android

echo "Building release APK..."
cd android
./gradlew assembleRelease

APK_PATH="app/build/outputs/apk/release/app-release.apk"
if [ -f "$APK_PATH" ]; then
  cp "$APK_PATH" "$WEBSITE_DIR/btcpc.apk"
  echo ""
  echo "Release APK built successfully!"
  echo "  Copied: $WEBSITE_DIR/btcpc.apk"
  ls -lh "$WEBSITE_DIR/btcpc.apk"
else
  echo "ERROR: Release APK not found at $APK_PATH"
  exit 1
fi
