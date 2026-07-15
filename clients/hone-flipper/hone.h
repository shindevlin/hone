#pragma once

#include <furi.h>
#include <gui/gui.h>
#include <gui/view_dispatcher.h>
#include <gui/scene_manager.h>
#include <gui/modules/submenu.h>
#include <gui/modules/popup.h>
#include <gui/modules/text_box.h>
#include <storage/storage.h>
#include <applications/services/bt/bt_service/bt.h>

#include "crypto/ed25519.h"
#include "protocol/hone_protocol.h"

#define TAG               "HONE"
#define HONE_VERSION     "0.1.0"

#define HONE_DATA_DIR    EXT_PATH("apps_data/hone")
#define HONE_KEY_PATH    EXT_PATH("apps_data/hone/identity.key")
#define HONE_PUB_PATH    EXT_PATH("apps_data/hone/identity.pub")

/* Secret key: 64 bytes (seed || public in TweetNaCl expanded form) */
#define HONE_SK_LEN      64
/* Public key: 32 bytes */
#define HONE_PK_LEN      32

/* Scene IDs */
typedef enum {
    HoneSceneMain,
    HoneSceneIdentity,
    HoneSceneBle,
    HoneSceneSubGhz,
    HoneSceneRotate,
    HoneSceneCount,
} HoneScene;

/* View IDs */
typedef enum {
    HoneViewSubmenu,
    HoneViewPopup,
    HoneViewTextBox,
    HoneViewCount,
} HoneViewId;

/* Submenu item IDs */
typedef enum {
    HoneMenuIdentity = 0,
    HoneMenuBle      = 1,
    HoneMenuSubGhz   = 2,
    HoneMenuRotate   = 3,
} HoneMenuItem;

typedef struct HoneApp {
    /* GUI */
    Gui*              gui;
    ViewDispatcher*   view_dispatcher;
    SceneManager*     scene_manager;
    Submenu*          submenu;
    Popup*            popup;
    TextBox*          text_box;

    /* Crypto */
    bool     has_identity;
    uint8_t  sk[HONE_SK_LEN];  /* ed25519 secret key (TweetNaCl expanded) */
    uint8_t  pk[HONE_PK_LEN];  /* ed25519 public key */

    /* BLE */
    Bt*                    bt;
    FuriHalBleProfileBase* ble_profile;
    bool                   ble_connected;

    /* Scratch buffer for hex-encoding public key (64 hex chars + NUL) */
    char     pub_hex[HONE_PK_LEN * 2 + 1];
} HoneApp;

/* Called from scene files */
HoneApp* hone_app_alloc(void);
void      hone_app_free(HoneApp* app);
bool      hone_identity_load_or_create(HoneApp* app);
void      hone_pub_to_hex(const uint8_t pk[HONE_PK_LEN], char out[HONE_PK_LEN * 2 + 1]);
