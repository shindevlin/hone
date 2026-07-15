/*
 * hone_scene_rotate.h — adaptive auto-rotation capture scene
 *
 * Shin Devlin — honemesh.network
 */
#pragma once

#include <gui/scene_manager.h>

void hone_scene_rotate_on_enter(void* context);
bool hone_scene_rotate_on_event(void* context, SceneManagerEvent event);
void hone_scene_rotate_on_exit(void* context);
