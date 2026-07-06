/*
 * btcpc_scene_rotate.c — adaptive auto-rotation capture scene
 *
 * Drives btcpc_scheduler: each timer tick asks the scheduler which single
 * sensor to run, performs that sensor's capture, reports the yield back so the
 * scheduler adapts (barren sensors back off, productive ones get more slots,
 * event sensors stop once they hit the chain's per-epoch reward cap), then
 * transmits any signed frame to the phone over BLE.
 *
 * Only sub-GHz and heartbeat have real capture implementations today; the other
 * event sensors (NFC/RFID/iButton) report "not found" until their capture is
 * wired, which the scheduler correctly treats as barren and backs off. Adding
 * one is a single capture function — the rotation logic does not change.
 *
 * Shin Devlin — btcpc.network
 */

#include "../btcpc.h"
#include "../btcpc_ble.h"
#include "../btcpc_scheduler.h"
#include "btcpc_scene_rotate.h"
#include "btcpc_scene_subghz.h" /* reuses the sub-GHz sampling helper */

#include <furi_hal_subghz.h>
#include <furi_hal_region.h>
#include <furi_hal_power.h>
#include <gui/modules/text_box.h>

#define ROTATE_TEXT_LEN 256

/* Scheduler + timer live for the lifetime of the scene. Stored on the app so
 * the timer callback (which only gets the app) can reach them. Kept file-local
 * via a single static instance — the app is a singleton on the Flipper. */
static BtcpcScheduler g_sched;
static FuriTimer*     g_timer = NULL;
static char           g_status[ROTATE_TEXT_LEN];
static const char*    g_last_sensor = "—";
static BtcpcSensorKind g_last_kind = BtcpcSensorHeartbeat;
static bool           g_last_found = false;
static bool           g_last_sent = false;
static int            g_last_value = 0; /* e.g. RSSI for sub-GHz */

static const char* sensor_name(BtcpcSensorKind k) {
    switch(k) {
    case BtcpcSensorSubGhz:    return "Sub-GHz";
    case BtcpcSensorNfc:       return "NFC";
    case BtcpcSensorRfid:      return "RFID";
    case BtcpcSensorIButton:   return "iButton";
    case BtcpcSensorHeartbeat: return "Heartbeat";
    default:                   return "?";
    }
}

/*
 * Perform one capture for `kind`. Returns true if a real reading was produced.
 * On a real reading, builds+signs a frame and (best-effort) sends it over BLE;
 * sets *value for display where meaningful.
 */
static bool capture(BtcpcApp* app, BtcpcSensorKind kind, int* value, bool* sent) {
    *value = 0;
    *sent = false;
    static BtcpcFrame frame;
    size_t frame_len = 0;

    switch(kind) {
    case BtcpcSensorSubGhz: {
        bool ok = false;
        int8_t rssi = btcpc_subghz_sample_once(&ok);
        if(!ok) return false;
        *value = (int)rssi;
        BtcpcSubGhzObs obs = {
            .freq_hz = 433920000UL, .rssi_dbm = rssi, .modulation = 2, .bandwidth = 0,
        };
        frame_len = btcpc_build_subghz(&frame, &obs, app->sk);
        /* A live RSSI sample always "found" data (the spectrum is never silent
         * enough to be worthless); productive class = continuous. */
        break;
    }
    case BtcpcSensorHeartbeat: {
        uint8_t batt = 0;
        (void)furi_hal_power_get_battery_charge_voltage; /* keep include used */
        batt = (uint8_t)furi_hal_power_get_pct();
        BtcpcHeartbeat hb = { .battery_pct = batt, .uptime_s = 0, .fw_version = "0.2.0" };
        frame_len = btcpc_build_heartbeat(&frame, &hb, app->sk);
        *value = (int)batt;
        break;
    }
    case BtcpcSensorNfc:
    case BtcpcSensorRfid:
    case BtcpcSensorIButton:
    default:
        /* Capture not wired yet — report barren so the scheduler backs off. */
        return false;
    }

    if(frame_len > 0) {
        *sent = btcpc_ble_send(app, (const uint8_t*)&frame, frame_len);
        return true;
    }
    return false;
}

static void rotate_render(BtcpcApp* app) {
    snprintf(g_status, sizeof(g_status),
        "Auto Rotate\n\n"
        "Cycle: %lu\n"
        "Last: %s %s\n"
        "Value: %d\n"
        "BLE: %s\n\n"
        "Sub-GHz y:%lu  NFC y:%lu\n"
        "RFID y:%lu  iBtn y:%lu",
        (unsigned long)g_sched.cycle,
        g_last_sensor, g_last_found ? "(hit)" : "(none)",
        g_last_value,
        g_last_sent ? "sent" : (app->ble_connected ? "—" : "no phone"),
        (unsigned long)g_sched.sensors[BtcpcSensorSubGhz].total_yield,
        (unsigned long)g_sched.sensors[BtcpcSensorNfc].total_yield,
        (unsigned long)g_sched.sensors[BtcpcSensorRfid].total_yield,
        (unsigned long)g_sched.sensors[BtcpcSensorIButton].total_yield);
    text_box_set_text(app->text_box, g_status);
}

static void rotate_timer_cb(void* context) {
    BtcpcApp* app = context;

    /* Keep battery fresh for pacing/heartbeat. */
    btcpc_sched_set_battery(&g_sched, (uint8_t)furi_hal_power_get_pct());

    BtcpcSensorKind k = btcpc_sched_next(&g_sched);
    int value = 0; bool sent = false;
    bool found = capture(app, k, &value, &sent);
    btcpc_sched_report(&g_sched, k, found);

    g_last_kind = k;
    g_last_sensor = sensor_name(k);
    g_last_found = found;
    g_last_sent = sent;
    g_last_value = value;

    rotate_render(app);

    /* Reschedule with the battery-aware delay. */
    furi_timer_start(g_timer, furi_ms_to_ticks(btcpc_sched_cycle_delay_ms(&g_sched)));
}

void btcpc_scene_rotate_on_enter(void* context) {
    BtcpcApp* app = context;

    if(!app->has_identity) {
        text_box_reset(app->text_box);
        text_box_set_text(app->text_box,
            "No device key.\nOpen Identity / Key\nfirst to generate one.");
        view_dispatcher_switch_to_view(app->view_dispatcher, BtcpcViewTextBox);
        return;
    }

    btcpc_sched_init(&g_sched);
    g_last_sensor = "—";
    g_last_found = false;
    g_last_sent = false;
    g_last_value = 0;

    text_box_reset(app->text_box);
    rotate_render(app);
    view_dispatcher_switch_to_view(app->view_dispatcher, BtcpcViewTextBox);

    if(!g_timer) {
        g_timer = furi_timer_alloc(rotate_timer_cb, FuriTimerTypeOnce, app);
    }
    furi_timer_start(g_timer, furi_ms_to_ticks(btcpc_sched_cycle_delay_ms(&g_sched)));
}

bool btcpc_scene_rotate_on_event(void* context, SceneManagerEvent event) {
    UNUSED(context);
    UNUSED(event);
    return false;
}

void btcpc_scene_rotate_on_exit(void* context) {
    BtcpcApp* app = context;
    if(g_timer) {
        furi_timer_stop(g_timer);
        furi_timer_free(g_timer);
        g_timer = NULL;
    }
    /* Make sure the radio is released when we leave. */
    furi_hal_subghz_sleep();
    text_box_reset(app->text_box);
}
