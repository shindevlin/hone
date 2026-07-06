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
#include "protocol/btcpc_protocol.h"

#define TAG               "BTCPC"
#define BTCPC_VERSION     "0.1.0"

#define BTCPC_DATA_DIR    EXT_PATH("apps_data/btcpc")
#define BTCPC_KEY_PATH    EXT_PATH("apps_data/btcpc/identity.key")
#define BTCPC_PUB_PATH    EXT_PATH("apps_data/btcpc/identity.pub")

/* Secret key: 64 bytes (seed || public in TweetNaCl expanded form) */
#define BTCPC_SK_LEN      64
/* Public key: 32 bytes */
#define BTCPC_PK_LEN      32

/* Scene IDs */
typedef enum {
    BtcpcSceneMain,
    BtcpcSceneIdentity,
    BtcpcSceneBle,
    BtcpcSceneSubGhz,
    BtcpcSceneRotate,
    BtcpcSceneCount,
} BtcpcScene;

/* View IDs */
typedef enum {
    BtcpcViewSubmenu,
    BtcpcViewPopup,
    BtcpcViewTextBox,
    BtcpcViewCount,
} BtcpcViewId;

/* Submenu item IDs */
typedef enum {
    BtcpcMenuIdentity = 0,
    BtcpcMenuBle      = 1,
    BtcpcMenuSubGhz   = 2,
    BtcpcMenuRotate   = 3,
} BtcpcMenuItem;

typedef struct BtcpcApp {
    /* GUI */
    Gui*              gui;
    ViewDispatcher*   view_dispatcher;
    SceneManager*     scene_manager;
    Submenu*          submenu;
    Popup*            popup;
    TextBox*          text_box;

    /* Crypto */
    bool     has_identity;
    uint8_t  sk[BTCPC_SK_LEN];  /* ed25519 secret key (TweetNaCl expanded) */
    uint8_t  pk[BTCPC_PK_LEN];  /* ed25519 public key */

    /* BLE */
    Bt*                    bt;
    FuriHalBleProfileBase* ble_profile;
    bool                   ble_connected;

    /* Scratch buffer for hex-encoding public key (64 hex chars + NUL) */
    char     pub_hex[BTCPC_PK_LEN * 2 + 1];
} BtcpcApp;

/* Called from scene files */
BtcpcApp* btcpc_app_alloc(void);
void      btcpc_app_free(BtcpcApp* app);
bool      btcpc_identity_load_or_create(BtcpcApp* app);
void      btcpc_pub_to_hex(const uint8_t pk[BTCPC_PK_LEN], char out[BTCPC_PK_LEN * 2 + 1]);
