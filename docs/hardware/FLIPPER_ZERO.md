# Flipper Zero — BTCPC Hardware Reference

## Device Info
- **Model**: Flipper Zero (flip_Ar8atsu1)
- **Firmware**: 1.4.3
- **CPU**: STM32WB55 (ARM Cortex-M4 @ 64MHz + Cortex-M0+ for radio)
- **RAM**: 256KB (apps get ~60-70KB)
- **Flash**: 1MB for FAPs
- **MicroSD**: up to 256GB
- **Battery**: 2000mAh Li-Po (~7-8 hours with radio scanning)
- **BTCPC Account**: josh
- **Key stored**: memo key only (sensor data signing)

## Radios Available

### Sub-GHz (CC1101)
- **Frequencies**: 300-348 MHz, 387-464 MHz, 779-928 MHz
- **BTCPC scans**: 433.92, 315, 868.35, 915 MHz
- **API**: furi_hal_subghz_* (see flipper_subghz_api.h)
- **Init sequence**: reset() → 5ms settle → idle() → set_frequency_and_path() → rx() → get_rssi() → idle() → sleep()
- **IMPORTANT**: furi_hal_subghz_init() is NOT available to FAP apps. Use reset()+idle() instead.

### BLE (STM32WB integrated)
- **Channels**: 0-39 (advertising: 37, 38, 39)
- **API**: furi_hal_bt_* (see flipper_bt_api.h)
- **Init**: check furi_hal_bt_is_active() first — if phone is connected, skip scan
- **Sequence**: start_rx(channel) → delay 200ms → get_rssi() → stop_rx()
- **IMPORTANT**: BLE and Sub-GHz share the radio. Time-multiplex, never simultaneous.

### NFC (ST25R3916)
- **Frequency**: 13.56 MHz
- **API**: furi_hal_nfc_* (see flipper_nfc_api.h)
- **Field detect**: field_detect_start() → 100ms → field_is_present() → field_detect_stop()
- **IMPORTANT**: Do NOT use nfc_alloc/nfc_scanner — crashes on 1.4.3. Use HAL field detect only.

### IR (Infrared)
- **API**: infrared_* (see flipper_ir_api.h)
- **Can detect**: IR signals from remotes, sensors
- **BTCPC use**: future — IR presence detection

### LF RFID (125 kHz)
- **API**: lfrfid_* (see flipper_rfid_api.h)
- **Can read**: EM4100, HID, Indala tags
- **BTCPC use**: future — asset tracking, access control logging

### GPIO
- **Pins**: PA4 (ADC), PA6, PA7, PB2 (RX), PB3 (TX), PC0 (SDA), PC1 (SCL)
- **ADC**: furi_hal_adc_* on PA4 (see flipper_adc_api.h)
- **I2C**: PC0/PC1 for BME280, ADXL345, etc.
- **UART**: PB2/PB3 at 9600-115200 baud for GPS modules

### Internal Sensors
- **CPU temperature**: via ADC FuriHalAdcChannelTEMPSENSOR
- **Battery**: furi_hal_power_get_battery_voltage(), get_pct()

## BTCPC App Architecture

```
btcpc_wallet.c (single file FAP)
├── Wallet (import, list, delete, sign via USB JSON)
├── Sensor Thread (background, 4KB stack)
│   ├── Sub-GHz scan (433/315/868/915 MHz RSSI)
│   ├── BLE scan (advertising channel hits)
│   ├── NFC field detect
│   ├── GPIO ADC read (PA4)
│   └── CPU temperature
├── USB CDC Thread (JSON command protocol)
└── Reading buffer (readings.jsonl on microSD)
```

## USB Serial Protocol
- Baud: 115200
- Format: newline-delimited JSON
- Commands: ping, list, import, delete, sign, sensors, flush_readings, sensor_status

## Known Issues (1.4.3)
- furi_hal_subghz_init() not exported to FAP apps — use reset()+idle()
- furi_hal_crypto_gcm_* crashes — use XOR encryption for Phase 1
- NFC scanner (nfc_alloc) crashes — use HAL field_detect instead
- ufbt launch hangs intermittently — use qFlipper for file upload
- USB CDC claimed by app blocks ufbt RPC — exit app before deploying
