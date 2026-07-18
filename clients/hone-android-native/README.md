# HONE — Native Android App (Kotlin + Jetpack Compose)

The world-class native HONE app. Kotlin/Compose UI over the Rust chain core
(`rust/hone-android/`) via the uniffi bridge, running in a foreground service so
the phone is a true miner + clock + sensor node when backgrounded.

See [`docs/ANDROID_WORLDCLASS_PLAN.md`](../../docs/ANDROID_WORLDCLASS_PLAN.md) for
the full plan and locked decisions. This replaces the old Capacitor webview
(`clients/hone-android`), which stays buildable until this reaches parity.

## Status

**Phase 1 (in progress): native app shell against mock data.**
- Material 3 theme (dark-first, HONE orange) — `ui/theme/`
- Bottom-nav shell (Node · Wallet · Sensors · Settings) — `ui/HoneApp.kt`
- Node dashboard: live balance/epoch/peers, animated stats, haptic mine toggle
  — `ui/screens/NodeScreen.kt`
- `NodeRepository` seam with `MockNodeRepository` so the UI builds/runs with NO
  native `.so` yet — `data/NodeRepository.kt`
- Foreground `NodeService` skeleton — `service/NodeService.kt`

**Not yet wired:**
- The real Rust node (Phase 0b): drop the uniffi-generated `hone_miner.kt` +
  `libhone_miner.so` (per-ABI) into `app/src/main/`, add the JNA dep, and swap
  `MockNodeRepository` → `BridgeNodeRepository`. UI needs zero changes (the
  `NodeRepository` interface is the seam).
- Wallet / Sensors / Settings screens (Phase 2).
- Notifications, biometric unlock, LLM chat, full a11y (Phase 3).

## Build

Requires Android Studio (Ladybug+) or the command-line SDK + JDK 17.
```bash
# From this dir, once the Gradle wrapper jar is present:
./gradlew :app:assembleDebug
```
The UI runs entirely on mock data today — no NDK/Rust build needed to see it.

## Native lib (Phase 0b)

```bash
# In rust/hone-android/ — build the .so per ABI and drop into jniLibs:
cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 \
  -o ../../clients/hone-android-native/app/src/main/jniLibs build --release
# Generate Kotlin bindings (see rust/hone-android/BRIDGE_BUILD_NOTES.md):
cargo run --features=uniffi/cli --bin uniffi-bindgen -- \
  generate --library <lib> --language kotlin \
  --out-dir app/src/main/java
```
