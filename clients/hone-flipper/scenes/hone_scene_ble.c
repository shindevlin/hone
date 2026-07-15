/*
 * hone_scene_ble.c — BLE pairing status scene
 *
 * Shows the current BLE connection state and instructs the user
 * how to pair. The actual BLE NUS peripheral is managed in hone.c;
 * this scene is display-only.
 *
 * TODO (Phase 9 wiring):
 *   - Start BLE NUS peripheral via furi_hal_bt_start_advertising()
 *     or profiles/serial_profile.h (matches hone_ble_relay pattern)
 *   - Register RX callback to receive HONE_MSG_ENTRY_HASH /
 *     HONE_MSG_CLOCK_SYNC / HONE_MSG_GPS frames from the phone app
 *   - On each incoming Sub-GHz / RFID / NFC / iButton event, call
 *     hone_build_*() and send over BLE TX characteristic
 *   - Auto-reconnect: store paired device address in
 *     /ext/apps_data/hone/paired.addr (6 bytes) and reconnect on boot
 *
 * Shin Devlin — honemesh.network
 */

#include "../hone.h"
#include "hone_scene_ble.h"

#include <gui/modules/text_box.h>
#include <string.h>
#include <stdio.h>

#define BLE_TEXT_LEN 128

void hone_scene_ble_on_enter(void* context) {
    HoneApp* app = context;

    static char ble_text[BLE_TEXT_LEN];

    const char* status = app->ble_connected ? "Connected" : "Not connected";

    snprintf(ble_text, sizeof(ble_text),
             "BLE Status:\n"
             "%s\n\n"
             "Open HONE app\n"
             "on phone to pair.",
             status);

    text_box_reset(app->text_box);
    text_box_set_font(app->text_box, TextBoxFontText);
    text_box_set_text(app->text_box, ble_text);

    view_dispatcher_switch_to_view(app->view_dispatcher, HoneViewTextBox);
}

bool hone_scene_ble_on_event(void* context, SceneManagerEvent event) {
    UNUSED(context);
    UNUSED(event);
    /*
     * TODO: handle HoneEventBlePaired / HoneEventBleDisconnected custom
     * events posted from the BLE RX callback, and refresh the text_box.
     */
    return false;
}

void hone_scene_ble_on_exit(void* context) {
    HoneApp* app = context;
    text_box_reset(app->text_box);
    /*
     * TODO: stop advertising / disconnect if user navigates away
     * while not yet paired (optional — auto-reconnect is preferred).
     */
}
