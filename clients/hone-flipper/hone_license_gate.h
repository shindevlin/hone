/*
 * UFL 3.8 Decentralized acceptance gate (UFL Section 9) for HONE Flipper apps.
 *
 * Identical copies live in every HONE Flipper app directory
 * (clients/hone-flipper, flipper/hone_sensor, flipper/hone_relay,
 * flipper/hone_ble_relay). Keep them in sync. See docs/LICENSE_ACCEPTANCE.md.
 */
#pragma once

#include <stdbool.h>

/*
 * Call first in the app's entry point. Returns true if HONE's license (UFL 3.8,
 * Operational Scope: Decentralized) was accepted, now or earlier, on this
 * Flipper. Returns false if the user declined: the app must exit without doing
 * anything else.
 *
 * One record, /ext/apps_data/hone/license-acceptance.json, covers every HONE
 * app on the device. It holds only license, version, scope, the device-clock
 * time, and the app name, and stays on the SD card.
 */
bool hone_license_gate(const char* app_name);
