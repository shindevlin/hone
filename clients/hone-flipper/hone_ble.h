/*
 * btcpc_ble.h — BLE serial transport for sending signed frames to the phone
 *
 * The BTCPC app runs the Flipper's Serial BLE profile so the paired phone can
 * receive framed, device-signed sensor observations over the BLE serial (UART)
 * characteristic. Capture scenes call btcpc_ble_send() with a complete
 * BtcpcFrame; the phone verifies the device signature, then re-signs the
 * reading as a chain SensorReading (see docs/SIGNING_INTEGRATION.md).
 *
 * Shin Devlin — btcpc.network
 */
#pragma once

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>

/* Forward-declared to avoid pulling the full app header into the BLE unit. */
typedef struct BtcpcApp BtcpcApp;

/*
 * Start the Serial BLE profile and begin advertising. Idempotent — safe to
 * call more than once. Sets app->ble_connected as the link state changes.
 * Returns true if the profile started (advertising), false on failure.
 */
bool btcpc_ble_start(BtcpcApp* app);

/* Stop advertising and restore the default BLE profile. Safe if not started. */
void btcpc_ble_stop(BtcpcApp* app);

/*
 * Send `len` bytes over the BLE serial TX characteristic. Returns true if the
 * bytes were queued for transmission. Returns false if BLE is not connected or
 * the profile is not running. Large frames are chunked to the negotiated MTU
 * by the HAL; callers pass the whole frame.
 */
bool btcpc_ble_send(BtcpcApp* app, const uint8_t* data, size_t len);
