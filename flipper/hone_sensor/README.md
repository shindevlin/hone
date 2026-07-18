# HONE Sensor

External Flipper Zero app for HONE sensor collection.

## Responsibilities

- collect RF / proximity-style readings from the device
- buffer readings locally on microSD
- expose newline-delimited JSON over USB CDC
- respond to `ping`, `sensors`, `sensor_status`, and `flush_readings`

## Build

This app is intended to be built with `ufbt` in a Flipper firmware SDK checkout.
The HONE repository now contains the source tree, but not the Flipper SDK itself.

## Notes

- BLE and Sub-GHz are time-multiplexed, not concurrent
- the app skips BLE scans when the BT stack is already active
- NFC is used in field-detect mode only
- GPIO ADC uses PA4 when available
