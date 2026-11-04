# HONE — Native Android App (Kotlin + Jetpack Compose)

The world-class native HONE app. Kotlin/Compose UI over the Rust chain core
(`rust/hone-android/`) via the uniffi bridge, running in a foreground service so
the phone is a true miner + clock + sensor node when backgrounded.

See [`docs/ANDROID_WORLDCLASS_PLAN.md`](../../docs/ANDROID_WORLDCLASS_PLAN.md) for
the full plan and locked decisions. This replaces the old Capacitor webview
(`clients/btcpc-android`), which stays buildable until this reaches parity.

## Status

**Phase 1 (in progress): native app shell against mock data.**
- Material 3 theme (dark-first, HONE orange) — `ui/theme/`
- Bottom-nav shell (Node · Wallet · Sensors · Settings) — `ui/HoneApp.kt`
- Node dashboard: live balance/epoch/peers, animated stats, haptic mine toggle
  — `ui/screens/NodeScreen.kt`
- `NodeRepository` seam with `MockNodeRepository` so the UI builds/runs with NO
  native `.so` yet — `data/NodeRepository.kt`
- Foreground `NodeService` skeleton — `service/NodeService.kt`
- Sensor wire client, AndroidKeyStore Ed25519 device identity (API 33+), and explicit
  enrollment/device-signing coordinator — `data/SensorCommitClient.kt`,
  `data/AndroidSensorKeyStore.kt`, and `data/SensorSubmissionCoordinator.kt`

The checked-in foreground service is intentionally **read-only capture**. It has no embedded
account name, node address, or owner signing key. In particular, the repository does not import,
persist, or read raw owner posting seeds. `SensorNodeSettings` requires all public runtime values
to be supplied explicitly.

**Not yet wired:**
- The real Rust node (Phase 0b): drop the uniffi-generated `hone_miner.kt` +
  `libhone_miner.so` (per-ABI) into `app/src/main/`, add the JNA dep, and swap
  `MockNodeRepository` → `BridgeNodeRepository`. UI needs zero changes (the
  `NodeRepository` interface is the seam).
- Wallet / Settings screens and the sensor enrollment/configuration UI (Phase 2).
- Notifications, biometric unlock, LLM chat, full a11y (Phase 3).

## Build

Requires Android Studio (Ladybug+) or the command-line SDK + JDK 17.
```bash
# From this dir, once the Gradle wrapper jar is present:
./gradlew :app:assembleDebug
```
The UI runs entirely on mock data today — no NDK/Rust build needed to see it.

## Remaining sensor submission hooks

Before submission can be enabled, the app state layer must:

1. collect and visibly confirm `chainId`, owner account, node URL, and a unique sensor prefix;
2. create the per-sensor device identity with `AndroidSensorKeyStore.getOrCreate`;
3. request the owner wallet to authorize one enrollment operation and expose only a
   one-operation `SensorSigner` callback—never owner key bytes;
4. call `SensorSubmissionCoordinator.enrollDevice` once, then submit readings through
   `submitWithEnrolledDevice`; and
5. display enrollment/revocation and HTTP acceptance state in the UI.

The owner-posting fallback remains at the protocol seam because both authorization paths are
approved, but it likewise accepts only a per-operation callback. The foreground service must not
retain that callback. Test this only against a disposable local node and fresh test accounts.

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
