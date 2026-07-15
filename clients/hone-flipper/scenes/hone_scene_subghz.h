/*
 * hone_scene_subghz.h — Sub-GHz spectrum observation capture scene
 *
 * Shin Devlin — honemesh.network
 */
#pragma once

#include <gui/scene_manager.h>
#include <stdint.h>
#include <stdbool.h>

void hone_scene_subghz_on_enter(void* context);
bool hone_scene_subghz_on_event(void* context, SceneManagerEvent event);
void hone_scene_subghz_on_exit(void* context);

/* Sample averaged RSSI (dBm) at `freq_hz`; *ok=false if frequency not allowed.
 * Reusable by the auto-rotation scene. */
int8_t hone_subghz_sample_rssi(uint32_t freq_hz, bool* ok);
/* Sample at the default 433.92 MHz observation frequency. */
int8_t hone_subghz_sample_once(bool* ok);
