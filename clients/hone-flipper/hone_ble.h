/*
 * hone_ble.h — BLE serial transport for sending signed frames to the phone
 *
 * The HONE app runs the Flipper's Serial BLE profile so the paired phone can
 * receive framed, device-signed sensor observations over the BLE serial (UART)
 * characteristic. Capture scenes call hone_ble_send() with a complete
 * HoneFrame; the phone verifies the device signature, then re-signs the
 * reading as a chain SensorReading (see docs/SIGNING_INTEGRATION.md).
 *
 * Shin Devlin — honemesh.network
 */
#pragma once

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>

/* Forward-declared to avoid pulling the full app header into the BLE unit. */
typedef struct HoneApp HoneApp;

/*
 * Start the Serial BLE profile and begin advertising. Idempotent — safe to
 * call more than once. Sets app->ble_connected as the link state changes.
 * Returns true if the profile started (advertising), false on failure.
 */
bool hone_ble_start(HoneApp* app);

/* Stop advertising and restore the default BLE profile. Safe if not started. */
void hone_ble_stop(HoneApp* app);

/*
 * Send `len` bytes over the BLE serial TX characteristic. Returns true if the
 * bytes were queued for transmission. Returns false if BLE is not connected or
 * the profile is not running. Large frames are chunked to the negotiated MTU
 * by the HAL; callers pass the whole frame.
 */
bool hone_ble_send(HoneApp* app, const uint8_t* data, size_t len);
