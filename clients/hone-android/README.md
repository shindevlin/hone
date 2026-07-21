# HONE Android

Android APK wrapper for the HONE PWA (`website/app.html`) using Capacitor.
Provides native sensor access (GPS, motion, device info, network state) to the
web app running inside a native WebView.

## Prerequisites

### Android SDK

Install **one** of:

- **Android Studio** (recommended): https://developer.android.com/studio
- **Command-line tools only**: https://developer.android.com/studio#command-line-tools-only

After installing, set the environment variable:

```bash
export ANDROID_HOME=$HOME/Android/Sdk
export PATH=$PATH:$ANDROID_HOME/platform-tools:$ANDROID_HOME/cmdline-tools/latest/bin
```

Install required SDK components:

```bash
sdkmanager "platforms;android-34" "build-tools;34.0.0" "platform-tools"
```

Accept licenses:

```bash
sdkmanager --licenses
```

### Java

Android Gradle requires JDK 17+:

```bash
sudo apt install openjdk-17-jdk
```

## Build

### Debug APK

```bash
npm run build
# or directly:
./scripts/build-apk.sh
```

The APK is copied to `../hone/website/hone.apk` for download from the site.

### Release APK

Generate a keystore first:

```bash
keytool -genkey -v -keystore hone-release.jks -keyalg RSA -keysize 2048 -validity 10000 -alias hone
```

Then build:

```bash
HONE_KEYSTORE=./hone-release.jks npm run build:release
```

## Development

Open in Android Studio:

```bash
npm run open
```

Run on connected device/emulator:

```bash
npm run run
```

Sync web assets after changing app.html:

```bash
npm run sync
```

## Native Plugins

| Plugin | Purpose |
|--------|---------|
| @capacitor/geolocation | GPS coordinates for sensor mining |
| @capacitor/motion | Accelerometer/gyroscope data |
| @capacitor/device | Device ID, model, OS info |
| @capacitor/network | Connection type, online/offline |
| @capacitor/status-bar | Dark status bar matching app theme |

## Project Structure

```
hone-android/
  capacitor.config.ts    — Capacitor configuration
  scripts/
    build-apk.sh         — Debug build script
    build-release.sh     — Signed release build script
  android/               — Native Android project (Gradle)
    app/src/main/
      AndroidManifest.xml — Permissions (location, sensors, network)
      assets/public/      — Copied web assets from website/
```
