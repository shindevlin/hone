/*
 * btcpc_scene_identity.c — Identity / public key display scene
 *
 * Shows the Flipper's ed25519 public key as a 64-character hex string.
 * The user reads this key and registers it on-chain via the phone app
 * (DeviceRegister entry), or by scanning the phone's QR code during pairing.
 *
 * Shin Devlin — btcpc.network
 */

#include "../btcpc.h"
#include "btcpc_scene_identity.h"

#include <gui/modules/text_box.h>
#include <string.h>
#include <stdio.h>

/*
 * Format: 64-hex public key split across two lines (32 chars each),
 * with a label on the first line so the screen is readable on 128x64.
 *
 * Buffer: "Public Key:\n" (12) + 32 hex (32) + "\n" (1) + 32 hex (32) + NUL = 78
 */
#define IDENTITY_TEXT_LEN 80

void btcpc_scene_identity_on_enter(void* context) {
    BtcpcApp* app = context;

    static char identity_text[IDENTITY_TEXT_LEN];

    if(app->has_identity) {
        snprintf(identity_text, sizeof(identity_text),
                 "Public Key:\n%.32s\n%.32s",
                 app->pub_hex,
                 app->pub_hex + 32);
    } else {
        snprintf(identity_text, sizeof(identity_text),
                 "No identity\nkey found.\nCheck storage.");
    }

    text_box_reset(app->text_box);
    text_box_set_font(app->text_box, TextBoxFontText);
    text_box_set_text(app->text_box, identity_text);

    view_dispatcher_switch_to_view(app->view_dispatcher, BtcpcViewTextBox);
}

bool btcpc_scene_identity_on_event(void* context, SceneManagerEvent event) {
    UNUSED(context);
    UNUSED(event);
    return false;
}

void btcpc_scene_identity_on_exit(void* context) {
    BtcpcApp* app = context;
    text_box_reset(app->text_box);
}
