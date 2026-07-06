/*
 * btcpc_scene_main.c — Main menu scene
 *
 * Shin Devlin — btcpc.network
 */

#include "../btcpc.h"
#include "btcpc_scene_main.h"
#include "btcpc_scene_identity.h"
#include "btcpc_scene_ble.h"
#include "btcpc_scene_subghz.h"
#include "btcpc_scene_rotate.h"

#include <gui/modules/submenu.h>

static void btcpc_scene_main_submenu_callback(void* context, uint32_t index) {
    BtcpcApp* app = context;
    scene_manager_handle_custom_event(app->scene_manager, index);
}

void btcpc_scene_main_on_enter(void* context) {
    BtcpcApp* app = context;

    submenu_reset(app->submenu);
    submenu_set_header(app->submenu, "BTCPC " BTCPC_VERSION);
    submenu_add_item(app->submenu, "Identity / Key", BtcpcMenuIdentity,
                     btcpc_scene_main_submenu_callback, app);
    submenu_add_item(app->submenu, "BLE Status", BtcpcMenuBle,
                     btcpc_scene_main_submenu_callback, app);
    submenu_add_item(app->submenu, "Sub-GHz Observe", BtcpcMenuSubGhz,
                     btcpc_scene_main_submenu_callback, app);
    submenu_add_item(app->submenu, "Auto Rotate", BtcpcMenuRotate,
                     btcpc_scene_main_submenu_callback, app);

    view_dispatcher_switch_to_view(app->view_dispatcher, BtcpcViewSubmenu);
}

bool btcpc_scene_main_on_event(void* context, SceneManagerEvent event) {
    BtcpcApp* app = context;
    bool consumed = false;

    if(event.type == SceneManagerEventTypeCustom) {
        switch(event.event) {
        case BtcpcMenuIdentity:
            scene_manager_next_scene(app->scene_manager, BtcpcSceneIdentity);
            consumed = true;
            break;
        case BtcpcMenuBle:
            scene_manager_next_scene(app->scene_manager, BtcpcSceneBle);
            consumed = true;
            break;
        case BtcpcMenuSubGhz:
            scene_manager_next_scene(app->scene_manager, BtcpcSceneSubGhz);
            consumed = true;
            break;
        case BtcpcMenuRotate:
            scene_manager_next_scene(app->scene_manager, BtcpcSceneRotate);
            consumed = true;
            break;
        default:
            break;
        }
    }

    return consumed;
}

void btcpc_scene_main_on_exit(void* context) {
    BtcpcApp* app = context;
    submenu_reset(app->submenu);
}
