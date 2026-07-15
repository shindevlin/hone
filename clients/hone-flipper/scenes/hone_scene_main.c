/*
 * hone_scene_main.c — Main menu scene
 *
 * Shin Devlin — honemesh.network
 */

#include "../hone.h"
#include "hone_scene_main.h"
#include "hone_scene_identity.h"
#include "hone_scene_ble.h"
#include "hone_scene_subghz.h"
#include "hone_scene_rotate.h"

#include <gui/modules/submenu.h>

static void hone_scene_main_submenu_callback(void* context, uint32_t index) {
    HoneApp* app = context;
    scene_manager_handle_custom_event(app->scene_manager, index);
}

void hone_scene_main_on_enter(void* context) {
    HoneApp* app = context;

    submenu_reset(app->submenu);
    submenu_set_header(app->submenu, "HONE " HONE_VERSION);
    submenu_add_item(app->submenu, "Identity / Key", HoneMenuIdentity,
                     hone_scene_main_submenu_callback, app);
    submenu_add_item(app->submenu, "BLE Status", HoneMenuBle,
                     hone_scene_main_submenu_callback, app);
    submenu_add_item(app->submenu, "Sub-GHz Observe", HoneMenuSubGhz,
                     hone_scene_main_submenu_callback, app);
    submenu_add_item(app->submenu, "Auto Rotate", HoneMenuRotate,
                     hone_scene_main_submenu_callback, app);

    view_dispatcher_switch_to_view(app->view_dispatcher, HoneViewSubmenu);
}

bool hone_scene_main_on_event(void* context, SceneManagerEvent event) {
    HoneApp* app = context;
    bool consumed = false;

    if(event.type == SceneManagerEventTypeCustom) {
        switch(event.event) {
        case HoneMenuIdentity:
            scene_manager_next_scene(app->scene_manager, HoneSceneIdentity);
            consumed = true;
            break;
        case HoneMenuBle:
            scene_manager_next_scene(app->scene_manager, HoneSceneBle);
            consumed = true;
            break;
        case HoneMenuSubGhz:
            scene_manager_next_scene(app->scene_manager, HoneSceneSubGhz);
            consumed = true;
            break;
        case HoneMenuRotate:
            scene_manager_next_scene(app->scene_manager, HoneSceneRotate);
            consumed = true;
            break;
        default:
            break;
        }
    }

    return consumed;
}

void hone_scene_main_on_exit(void* context) {
    HoneApp* app = context;
    submenu_reset(app->submenu);
}
