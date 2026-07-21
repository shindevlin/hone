# Flipper Zero HONE Implementation Plan

This repository currently contains:
- the wallet-only Flipper binary in `website/hone_wallet.fap`
- the new Flipper sensor app source under `flipper/hone_sensor/`
- the PC relay/listener scripts
- the HONE chain API that accepts sensor readings

The remaining work is validation on actual Flipper firmware and making sure the generated
`.fap` package is installed on-device.

## Patch Order

### 1. Device App

Add a real Flipper FAP source tree, for example:

- `flipper/hone_sensor/application.fam`
- `flipper/hone_sensor/hone_sensor.c`
- `flipper/hone_sensor/hone_sensor.h`

Responsibilities:
- collect Sub-GHz RSSI scans
- collect BLE advertising hits
- collect NFC field-detect state
- read GPIO ADC
- read CPU temperature and battery
- buffer readings locally on microSD
- implement `ping`, `list`, `sensors`, `flush_readings`, and `sensor_status`

### 2. Relay Fanout

The PC side should try local HONE API first, then configured fallback listeners.
The reusable helper now lives in:

- `src/services/flipperRelay.js`

The updated callers are:

- `bin/hone-flipper-listener`
- `scripts/hone-flipper-bridge.js`

### 3. Validation

Add focused tests for the transport helper:

- `tests/flipperRelay.test.js`

## Current Limitation

Until the app is built and installed on a Flipper Zero, the device cannot create new readings on hardware and the relay can only forward whatever the device returns.
