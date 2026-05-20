#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
WEBSITE_DIR="$PROJECT_DIR/../btcpc/website"

cd "$PROJECT_DIR"

# Check for Android SDK
if [ -z "${ANDROID_HOME:-}" ] && [ -z "${ANDROID_SDK_ROOT:-}" ]; then
  echo "ERROR: ANDROID_HOME or ANDROID_SDK_ROOT is not set."
  echo "Install Android command-line tools or Android Studio and set the env var."
  echo "See README.md for setup instructions."
  exit 1
fi

echo "Syncing web assets to Android project..."
npx cap sync android

echo "Building debug APK..."
cd android
./gradlew assembleDebug

APK_PATH="app/build/outputs/apk/debug/app-debug.apk"
if [ -f "$APK_PATH" ]; then
  cp "$APK_PATH" "$WEBSITE_DIR/btcpc.apk"
  echo ""
  echo "APK built successfully!"
  echo "  Source: $PROJECT_DIR/android/$APK_PATH"
  echo "  Copied: $WEBSITE_DIR/btcpc.apk"
  ls -lh "$WEBSITE_DIR/btcpc.apk"
else
  echo "ERROR: APK not found at $APK_PATH"
  exit 1
fi
