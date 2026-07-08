/*
 * btcpc_scene_subghz.h — Sub-GHz spectrum observation capture scene
 *
 * Shin Devlin — btcpc.network
 */
#pragma once

#include <gui/scene_manager.h>
#include <stdint.h>
#include <stdbool.h>

void btcpc_scene_subghz_on_enter(void* context);
bool btcpc_scene_subghz_on_event(void* context, SceneManagerEvent event);
void btcpc_scene_subghz_on_exit(void* context);

/* Sample averaged RSSI (dBm) at `freq_hz`; *ok=false if frequency not allowed.
 * Reusable by the auto-rotation scene. */
int8_t btcpc_subghz_sample_rssi(uint32_t freq_hz, bool* ok);
/* Sample at the default 433.92 MHz observation frequency. */
int8_t btcpc_subghz_sample_once(bool* ok);
