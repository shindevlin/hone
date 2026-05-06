# Flipper Zero BTCPC Implementation Plan

This repository currently contains:
- the wallet-only Flipper binary in `website/btcpc_wallet.fap`
- the new Flipper sensor app source under `flipper/btcpc_sensor/`
- the PC relay/listener scripts
- the BTCPC chain API that accepts sensor readings

The remaining work is validation on actual Flipper firmware and making sure the generated
`.fap` package is installed on-device.

## Patch Order

### 1. Device App

Add a real Flipper FAP source tree, for example:

- `flipper/btcpc_sensor/application.fam`
- `flipper/btcpc_sensor/btcpc_sensor.c`
- `flipper/btcpc_sensor/btcpc_sensor.h`

Responsibilities:
- collect Sub-GHz RSSI scans
- collect BLE advertising hits
- collect NFC field-detect state
- read GPIO ADC
- read CPU temperature and battery
- buffer readings locally on microSD
- implement `ping`, `list`, `sensors`, `flush_readings`, and `sensor_status`

### 2. Relay Fanout

The PC side should try local BTCPC API first, then configured fallback listeners.
The reusable helper now lives in:

- `src/services/flipperRelay.js`

The updated callers are:

- `bin/btcpc-flipper-listener`
- `scripts/btcpc-flipper-bridge.js`

### 3. Validation

Add focused tests for the transport helper:

- `tests/flipperRelay.test.js`

## Current Limitation

Until the app is built and installed on a Flipper Zero, the device cannot create new readings on hardware and the relay can only forward whatever the device returns.
