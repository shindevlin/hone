/*
 * btcpc_scene_subghz.c — Sub-GHz spectrum observation capture
 *
 * Samples RSSI at a fixed frequency using the Flipper's CC1101 radio, packs
 * the reading into a BtcpcSubGhzObs, signs it with the device key, and sends
 * the framed observation to the paired phone over BLE. The phone verifies the
 * device signature, then re-signs it as a chain SensorReading with the owner's
 * posting key (see clients/btcpc-flipper/docs/SIGNING_INTEGRATION.md, Option B).
 *
 * Shin Devlin — btcpc.network
 */

#include "../btcpc.h"
#include "../btcpc_ble.h"
#include "btcpc_scene_subghz.h"

#include <furi_hal_subghz.h>
#include <furi_hal_region.h>
#include <gui/modules/text_box.h>

/* Default observation frequency: 433.92 MHz (ISM, most common remote band). */
#define BTCPC_SUBGHZ_FREQ_HZ   433920000UL
/* Settle time after tuning before the RSSI read is meaningful (datasheet ~1ms). */
#define BTCPC_SUBGHZ_SETTLE_US 3000
/* Number of RSSI samples to average, spaced a few ms apart. */
#define BTCPC_SUBGHZ_SAMPLES   16
#define BTCPC_SUBGHZ_SAMPLE_GAP_MS 2

#define SUBGHZ_TEXT_LEN 256

/*
 * Read an averaged RSSI (dBm) at `freq_hz`. Returns the rounded dBm value,
 * or 0 and sets *ok = false if the frequency is not permitted in this region.
 * Leaves the radio in idle/sleep on exit.
 */
int8_t btcpc_subghz_sample_rssi(uint32_t freq_hz, bool* ok) {
    *ok = false;

    if(!furi_hal_region_is_frequency_allowed(freq_hz)) {
        return 0;
    }

    /* Take ownership of the radio (releases any other subghz user). */
    furi_hal_subghz_reset();
    furi_hal_subghz_idle();

    /* set_frequency_and_path returns the actually-tuned frequency; ignore it,
     * we only need RSSI at (approximately) freq_hz. Must be in idle first. */
    furi_hal_subghz_set_frequency_and_path(freq_hz);

    furi_hal_subghz_flush_rx();
    furi_hal_subghz_rx();
    furi_delay_us(BTCPC_SUBGHZ_SETTLE_US);

    float sum = 0.0f;
    for(size_t i = 0; i < BTCPC_SUBGHZ_SAMPLES; i++) {
        sum += furi_hal_subghz_get_rssi();
        furi_delay_ms(BTCPC_SUBGHZ_SAMPLE_GAP_MS);
    }

    /* Return the radio to a safe idle/sleep state and release it. */
    furi_hal_subghz_idle();
    furi_hal_subghz_sleep();

    float avg = sum / (float)BTCPC_SUBGHZ_SAMPLES;
    /* Clamp to int8 range and round. */
    if(avg > 0.0f) avg = 0.0f;
    if(avg < -128.0f) avg = -128.0f;
    *ok = true;
    return (int8_t)(avg - 0.5f); /* round toward the (negative) reading */
}

/* Convenience wrapper: sample at the default observation frequency. Used by
 * both the manual sub-GHz scene and the auto-rotation scene. */
int8_t btcpc_subghz_sample_once(bool* ok) {
    return btcpc_subghz_sample_rssi(BTCPC_SUBGHZ_FREQ_HZ, ok);
}

void btcpc_scene_subghz_on_enter(void* context) {
    BtcpcApp* app = context;
    static char text[SUBGHZ_TEXT_LEN];

    if(!app->has_identity) {
        text_box_reset(app->text_box);
        text_box_set_text(app->text_box,
            "No device key.\nOpen Identity / Key\nfirst to generate one.");
        view_dispatcher_switch_to_view(app->view_dispatcher, BtcpcViewTextBox);
        return;
    }

    bool ok = false;
    int8_t rssi = btcpc_subghz_sample_rssi(BTCPC_SUBGHZ_FREQ_HZ, &ok);

    if(!ok) {
        snprintf(text, sizeof(text),
            "Sub-GHz observe\n\n%lu Hz not permitted\nin this region.",
            (unsigned long)BTCPC_SUBGHZ_FREQ_HZ);
        text_box_reset(app->text_box);
        text_box_set_text(app->text_box, text);
        view_dispatcher_switch_to_view(app->view_dispatcher, BtcpcViewTextBox);
        return;
    }

    /* Build the signed observation frame. */
    BtcpcSubGhzObs obs = {
        .freq_hz    = BTCPC_SUBGHZ_FREQ_HZ,
        .rssi_dbm   = rssi,
        .modulation = 2, /* OOK — matches the preset used to sample */
        .bandwidth  = 0, /* wideband RSSI, not a demod bandwidth */
    };

    static BtcpcFrame frame;
    size_t frame_len = btcpc_build_subghz(&frame, &obs, app->sk);

    /* Self-check: verify our own signature so the capture->sign path is
     * provable on-device even without a phone connected. */
    bool sig_ok = btcpc_frame_verify(&frame.hdr, frame.payload, app->pk);

    /* Send the signed frame to the paired phone over BLE serial. */
    bool sent = btcpc_ble_send(app, (const uint8_t*)&frame, frame_len);

    snprintf(text, sizeof(text),
        "Sub-GHz observe\n\n"
        "Freq: %lu.%02lu MHz\n"
        "RSSI: %d dBm\n"
        "Frame: %u bytes\n"
        "Signature: %s\n"
        "BLE TX: %s\n\n"
        "Signed with device key.",
        (unsigned long)(BTCPC_SUBGHZ_FREQ_HZ / 1000000UL),
        (unsigned long)((BTCPC_SUBGHZ_FREQ_HZ % 1000000UL) / 10000UL),
        (int)rssi,
        (unsigned)frame_len,
        sig_ok ? "valid" : "FAILED",
        sent ? "sent" : (app->ble_connected ? "tx failed" : "no phone"));

    text_box_reset(app->text_box);
    text_box_set_text(app->text_box, text);
    view_dispatcher_switch_to_view(app->view_dispatcher, BtcpcViewTextBox);
}

bool btcpc_scene_subghz_on_event(void* context, SceneManagerEvent event) {
    UNUSED(context);
    UNUSED(event);
    /* Back button returns to the main menu via the scene manager default. */
    return false;
}

void btcpc_scene_subghz_on_exit(void* context) {
    BtcpcApp* app = context;
    text_box_reset(app->text_box);
    /* Defensive: ensure the radio is asleep even if we exited mid-sample. */
    furi_hal_subghz_sleep();
}
