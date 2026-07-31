# Local-only sensor earning round trip

This is a gate recipe, not permission to point the Android client at a live node.
Use fresh test accounts and a disposable local node state directory only.

1. Start a sensor-accepting node bound to loopback with a fresh temporary state directory.
2. Configure the debug Android client for that loopback endpoint (emulator `10.0.2.2`, or an
   explicitly isolated test LAN address for a physical phone).
3. Create a fresh test owner. Supply its posting signer through the wallet's secure signing callback;
   do not export or log its private key.
4. On Android API 33+, call `AndroidSensorKeyStore.getOrCreate(sensorId)`. Enroll its public key once
   through `SensorSubmissionCoordinator.enrollDevice`.
5. Submit a reading with `submitWithEnrolledDevice`. Assert HTTP acceptance, a ledger
   `SensorDataCommit`, and reward attribution to the owner (never to `sensorId`).
6. Attempt the same sensor ID under a different owner and assert rejection without side-state change.
7. Revoke with the owner signer, then submit with `submitWithOwnerFallback`; assert acceptance and
   reward attribution to the owner. Assert the formerly enrolled device signature is rejected.

The current Compose screen still displays mock readings. Production wiring requires the node URL,
owner identity, wallet signing callback, permission flow, sensor sampling policy, and foreground
service lifecycle to be supplied by the app state layer. It must not store posting-key material in
Compose state, intents, preferences, logs, or service extras.
