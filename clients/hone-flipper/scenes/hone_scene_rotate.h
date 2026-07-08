/*
 * btcpc_scene_rotate.h — adaptive auto-rotation capture scene
 *
 * Shin Devlin — btcpc.network
 */
#pragma once

#include <gui/scene_manager.h>

void btcpc_scene_rotate_on_enter(void* context);
bool btcpc_scene_rotate_on_event(void* context, SceneManagerEvent event);
void btcpc_scene_rotate_on_exit(void* context);
