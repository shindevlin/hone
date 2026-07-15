/*
 * hone.c — HONE Flipper Zero identity node
 *
 * Entry point and application lifecycle. Manages the scene/view stack.
 *
 * Shin Devlin — honemesh.network
 */

#include "hone.h"
#include "hone_ble.h"
#include "scenes/hone_scene_main.h"
#include "scenes/hone_scene_identity.h"
#include "scenes/hone_scene_ble.h"
#include "scenes/hone_scene_subghz.h"
#include "scenes/hone_scene_rotate.h"

#include <furi.h>
#include <furi_hal_random.h>
#include <storage/storage.h>
#include <string.h>
#include <stdio.h>

/* ─── Scene handlers table ─────────────────────────────────────────────── */

/* Order must match HoneScene enum in hone.h */
static const AppSceneOnEnterCallback hone_on_enter_handlers[] = {
    hone_scene_main_on_enter,
    hone_scene_identity_on_enter,
    hone_scene_ble_on_enter,
    hone_scene_subghz_on_enter,
    hone_scene_rotate_on_enter,
};

static const AppSceneOnEventCallback hone_on_event_handlers[] = {
    hone_scene_main_on_event,
    hone_scene_identity_on_event,
    hone_scene_ble_on_event,
    hone_scene_subghz_on_event,
    hone_scene_rotate_on_event,
};

static const AppSceneOnExitCallback hone_on_exit_handlers[] = {
    hone_scene_main_on_exit,
    hone_scene_identity_on_exit,
    hone_scene_ble_on_exit,
    hone_scene_subghz_on_exit,
    hone_scene_rotate_on_exit,
};

static const SceneManagerHandlers hone_scene_handlers = {
    .on_enter_handlers = hone_on_enter_handlers,
    .on_event_handlers = hone_on_event_handlers,
    .on_exit_handlers  = hone_on_exit_handlers,
    .scene_num         = HoneSceneCount,
};

/* ─── Custom event dispatcher ───────────────────────────────────────────── */

static bool hone_custom_event_callback(void* context, uint32_t event) {
    HoneApp* app = context;
    return scene_manager_handle_custom_event(app->scene_manager, event);
}

static bool hone_back_event_callback(void* context) {
    HoneApp* app = context;
    return scene_manager_handle_back_event(app->scene_manager);
}

/* ─── App alloc / free ──────────────────────────────────────────────────── */

HoneApp* hone_app_alloc(void) {
    HoneApp* app = malloc(sizeof(HoneApp));
    memset(app, 0, sizeof(HoneApp));

    app->gui = furi_record_open(RECORD_GUI);

    app->view_dispatcher = view_dispatcher_alloc();
    view_dispatcher_set_event_callback_context(app->view_dispatcher, app);
    view_dispatcher_set_custom_event_callback(app->view_dispatcher, hone_custom_event_callback);
    view_dispatcher_set_navigation_event_callback(app->view_dispatcher, hone_back_event_callback);

    app->scene_manager = scene_manager_alloc(&hone_scene_handlers, app);

    /* Submenu */
    app->submenu = submenu_alloc();
    view_dispatcher_add_view(app->view_dispatcher, HoneViewSubmenu, submenu_get_view(app->submenu));

    /* Popup */
    app->popup = popup_alloc();
    view_dispatcher_add_view(app->view_dispatcher, HoneViewPopup, popup_get_view(app->popup));

    /* TextBox */
    app->text_box = text_box_alloc();
    view_dispatcher_add_view(app->view_dispatcher, HoneViewTextBox, text_box_get_view(app->text_box));

    view_dispatcher_attach_to_gui(app->view_dispatcher, app->gui, ViewDispatcherTypeFullscreen);

    return app;
}

void hone_app_free(HoneApp* app) {
    furi_assert(app);

    view_dispatcher_remove_view(app->view_dispatcher, HoneViewTextBox);
    view_dispatcher_remove_view(app->view_dispatcher, HoneViewPopup);
    view_dispatcher_remove_view(app->view_dispatcher, HoneViewSubmenu);

    text_box_free(app->text_box);
    popup_free(app->popup);
    submenu_free(app->submenu);

    scene_manager_free(app->scene_manager);
    view_dispatcher_free(app->view_dispatcher);

    /* Close the BT record if hone_ble_start opened it. */
    if(app->bt) {
        furi_record_close(RECORD_BT);
        app->bt = NULL;
    }

    furi_record_close(RECORD_GUI);
    free(app);
}

/* ─── Identity helpers ──────────────────────────────────────────────────── */

bool hone_identity_load_or_create(HoneApp* app) {
    Storage* storage = furi_record_open(RECORD_STORAGE);

    /* Ensure data directory exists */
    storage_simply_mkdir(storage, HONE_DATA_DIR);

    bool loaded = false;

    /* Try to load existing keys */
    if(storage_file_exists(storage, HONE_KEY_PATH) &&
       storage_file_exists(storage, HONE_PUB_PATH)) {
        File* f = storage_file_alloc(storage);

        bool sk_ok = false, pk_ok = false;
        if(storage_file_open(f, HONE_KEY_PATH, FSAM_READ, FSOM_OPEN_EXISTING)) {
            sk_ok = (storage_file_read(f, app->sk, HONE_SK_LEN) == HONE_SK_LEN);
            storage_file_close(f);
        }
        if(storage_file_open(f, HONE_PUB_PATH, FSAM_READ, FSOM_OPEN_EXISTING)) {
            pk_ok = (storage_file_read(f, app->pk, HONE_PK_LEN) == HONE_PK_LEN);
            storage_file_close(f);
        }

        storage_file_free(f);
        loaded = sk_ok && pk_ok;
    }

    if(!loaded) {
        /* Generate new keypair from secure random seed */
        FURI_LOG_I(TAG, "No identity found — generating ed25519 keypair");

        /* Generate: writes sk (64 bytes) and pk (32 bytes) */
        hone_ed25519_keypair(app->pk, app->sk);

        /* Persist */
        File* f = storage_file_alloc(storage);

        if(storage_file_open(f, HONE_KEY_PATH, FSAM_WRITE, FSOM_CREATE_ALWAYS)) {
            storage_file_write(f, app->sk, HONE_SK_LEN);
            storage_file_close(f);
        } else {
            FURI_LOG_E(TAG, "Failed to write identity.key");
        }

        if(storage_file_open(f, HONE_PUB_PATH, FSAM_WRITE, FSOM_CREATE_ALWAYS)) {
            storage_file_write(f, app->pk, HONE_PK_LEN);
            storage_file_close(f);
        } else {
            FURI_LOG_E(TAG, "Failed to write identity.pub");
        }

        storage_file_free(f);
        loaded = true;
    }

    app->has_identity = loaded;
    furi_record_close(RECORD_STORAGE);
    return loaded;
}

void hone_pub_to_hex(const uint8_t pk[HONE_PK_LEN], char out[HONE_PK_LEN * 2 + 1]) {
    static const char hex[] = "0123456789abcdef";
    for(size_t i = 0; i < HONE_PK_LEN; i++) {
        out[i * 2]     = hex[(pk[i] >> 4) & 0x0F];
        out[i * 2 + 1] = hex[pk[i] & 0x0F];
    }
    out[HONE_PK_LEN * 2] = '\0';
}

/* ─── App entry point ───────────────────────────────────────────────────── */

int32_t hone_app(void* p) {
    UNUSED(p);

    HoneApp* app = hone_app_alloc();

    hone_identity_load_or_create(app);
    hone_pub_to_hex(app->pk, app->pub_hex);

    /* Start the Serial BLE profile so capture scenes can relay signed frames
     * to the paired phone. Non-fatal if it fails — the app still runs and the
     * frames still build/sign; they just won't transmit. */
    hone_ble_start(app);

    scene_manager_next_scene(app->scene_manager, HoneSceneMain);
    view_dispatcher_run(app->view_dispatcher);

    hone_ble_stop(app);
    hone_app_free(app);
    return 0;
}
