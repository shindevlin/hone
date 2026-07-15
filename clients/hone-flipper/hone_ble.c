/*
 * hone_ble.c — BLE serial transport (see hone_ble.h)
 *
 * Uses the Flipper Serial BLE profile so the phone can receive framed,
 * device-signed sensor observations over the BLE serial characteristic.
 *
 * NOTE (on-device verification needed): BLE profile management on the Flipper
 * is firmware-version-sensitive. This is written against the current
 * ble_profile_serial API (furi_hal_bt_start_app / ble_profile_serial /
 * furi_hal_bt_serial_tx). If a `ufbt` build reports a mismatch, the fix is
 * localised to this one file — the capture scenes only depend on the three
 * functions in the header.
 *
 * Shin Devlin — honemesh.network
 */

#include "hone.h"
#include "hone_ble.h"

#include <furi_hal_bt.h>
#include <targets/f7/ble_glue/profiles/serial_profile.h>

/* Serial TX buffer size the HAL exposes per notification. Frames larger than
 * this are sent in chunks. 244 = typical BLE 5 ATT payload after the 3-byte
 * ATT header on a 247-byte MTU. */
#define HONE_BLE_TX_CHUNK 240

static void hone_ble_connection_status(BtStatus status, void* context) {
    HoneApp* app = context;
    if(!app) return;
    app->ble_connected = (status == BtStatusConnected);
}

bool hone_ble_start(HoneApp* app) {
    if(!app) return false;

    if(!app->bt) {
        app->bt = furi_record_open(RECORD_BT);
    }

    /* Don't restart if we already have a running profile. */
    if(app->ble_profile) {
        return true;
    }

    bt_disconnect(app->bt);
    /* Give the stack a moment to drop the prior profile before switching. */
    furi_delay_ms(100);
    bt_keys_storage_set_default_path(app->bt);

    app->ble_profile = bt_profile_start(app->bt, ble_profile_serial, NULL);
    if(!app->ble_profile) {
        return false;
    }

    furi_hal_bt_start_advertising();
    bt_set_status_changed_callback(app->bt, hone_ble_connection_status, app);
    return true;
}

void hone_ble_stop(HoneApp* app) {
    if(!app || !app->bt) return;

    bt_set_status_changed_callback(app->bt, NULL, NULL);

    if(app->ble_profile) {
        /* Restore the default (keyboard/serial) profile so the rest of the
         * Flipper's BLE behaves normally after we exit. */
        bt_profile_restore_default(app->bt);
        app->ble_profile = NULL;
    }
    app->ble_connected = false;
}

bool hone_ble_send(HoneApp* app, const uint8_t* data, size_t len) {
    if(!app || !app->ble_profile || !app->ble_connected) return false;
    if(!data || len == 0) return false;

    size_t offset = 0;
    while(offset < len) {
        size_t chunk = len - offset;
        if(chunk > HONE_BLE_TX_CHUNK) chunk = HONE_BLE_TX_CHUNK;

        if(!ble_profile_serial_tx(app->ble_profile, (uint8_t*)(data + offset), (uint16_t)chunk)) {
            return false;
        }
        offset += chunk;
    }
    return true;
}
