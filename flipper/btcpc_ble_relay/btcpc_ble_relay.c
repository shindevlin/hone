/**
 * btcpc_ble_relay.c — BTCPC BLE Relay Flipper app
 *
 * BTCPC-branded replacement for the stock UART BLE Bridge.
 * Starts BLE NUS (Nordic UART Service) peripheral so the BTCPC Android
 * app can connect and receive sensor readings.
 *
 * Two data channels:
 *   Passive relay  — bytes received on BLE NUS are forwarded to USB CDC,
 *                    and bytes from USB CDC are forwarded back over BLE.
 *                    Keeps full compatibility with any tool using the serial bridge.
 *
 *   Active sensor  — every SENSOR_INTERVAL ms when a phone is connected,
 *                    the app reads the Flipper's own sensors (battery, CPU
 *                    temperature) and sends a newline-terminated JSON line:
 *                      {"id":"<account>/flipper-zero","value":<float>,
 *                       "metadata":{"type":"<type>","unit":"<unit>",
 *                       "battery":<pct>,"source":"flipper-ble"}}
 *
 * Config files (Flipper internal flash, not accessible via USB mass storage):
 *   /int/.btcpc/posting.key  — 32-byte hex HMAC key (optional; set by pair-flipper.js)
 *   /int/.btcpc/account.txt  — BTCPC account name, e.g. "josh" (optional; default = "user")
 *
 * Screen:
 *   ┌──────────────────────────────┐
 *   │ BTCPC BLE Relay              │
 *   │ ────────────────────────────│
 *   │ BLE: Waiting...              │
 *   │ TX: 0   RX: 0               │
 *   │ Battery: 78%  CPU: 32.1°C   │
 *   │ Back = exit                  │
 *   └──────────────────────────────┘
 *
 * Build:  cd flipper/btcpc_ble_relay && ufbt build
 */

#include <furi.h>
#include <furi_hal_version.h>
#include <furi_hal_adc.h>
#include <furi_hal_power.h>
#include <furi_hal_usb_cdc.h>
#include <furi_hal_usb.h>
#include <gui/gui.h>
#include <input/input.h>
#include <storage/storage.h>
#include <applications/services/bt/bt_service/bt.h>
#include <profiles/serial_profile.h>
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include "sha256.h"

#define TAG               "BTCPCBLERelay"
#define KEY_PATH          "/int/.btcpc/posting.key"
#define ACCOUNT_PATH      "/int/.btcpc/account.txt"
#define KEY_HEX_LEN       64
#define ACCOUNT_MAX       32
#define SENSOR_INTERVAL   5000   /* ms between active sensor packets */
#define BLE_TX_MAX        243    /* BLE_SVC_SERIAL_CHAR_VALUE_LEN_MAX */

typedef struct {
    FuriMutex*           mutex;
    /* BLE state */
    Bt*                  bt;
    FuriHalBleProfileBase* ble_profile;
    volatile bool        ble_connected;
    volatile bool        running;
    /* stats */
    volatile uint32_t    tx_count;
    volatile uint32_t    rx_count;
    /* last sensor snapshot */
    uint8_t              battery_pct;
    float                cpu_temp;
    /* identity */
    char                 account[ACCOUNT_MAX];
    char                 mac[18];               /* "AA:BB:CC:DD:EE:FF\0" */
    uint8_t              key[32];
    bool                 key_loaded;
    /* UI queue */
    FuriMessageQueue*    input_queue;
} RelayApp;

/* ---- Storage helpers ---- */

static bool load_account(char out[ACCOUNT_MAX]) {
    Storage* s = furi_record_open(RECORD_STORAGE);
    File*    f = storage_file_alloc(s);
    bool     ok = false;
    if(storage_file_open(f, ACCOUNT_PATH, FSAM_READ, FSOM_OPEN_EXISTING)) {
        uint16_t n = storage_file_read(f, out, ACCOUNT_MAX - 1);
        storage_file_close(f);
        while(n > 0 && (out[n-1]=='\n'||out[n-1]=='\r'||out[n-1]==' ')) n--;
        out[n] = '\0';
        ok = (n > 0);
    }
    storage_file_free(f);
    furi_record_close(RECORD_STORAGE);
    return ok;
}

static bool load_posting_key(uint8_t key[32]) {
    Storage* s = furi_record_open(RECORD_STORAGE);
    File*    f = storage_file_alloc(s);
    bool     ok = false;
    if(storage_file_open(f, KEY_PATH, FSAM_READ, FSOM_OPEN_EXISTING)) {
        char hex[KEY_HEX_LEN + 4];
        uint16_t n = storage_file_read(f, hex, sizeof(hex) - 1);
        storage_file_close(f);
        while(n > 0 && (hex[n-1]=='\n'||hex[n-1]=='\r'||hex[n-1]==' ')) n--;
        hex[n] = '\0';
        if(n == KEY_HEX_LEN) {
            ok = true;
            for(int i = 0; i < 32; i++) {
                uint8_t hi, lo;
                char c;
                c = hex[i*2];
                hi = (c>='0'&&c<='9')?c-'0':(c>='a'&&c<='f')?c-'a'+10:(c>='A'&&c<='F')?c-'A'+10:255;
                c = hex[i*2+1];
                lo = (c>='0'&&c<='9')?c-'0':(c>='a'&&c<='f')?c-'a'+10:(c>='A'&&c<='F')?c-'A'+10:255;
                if(hi==255||lo==255){ ok=false; break; }
                key[i] = (hi<<4)|lo;
            }
        }
    }
    storage_file_free(f);
    furi_record_close(RECORD_STORAGE);
    return ok;
}

/* ---- Sensor reading ---- */

static void read_sensors(RelayApp* app) {
    app->battery_pct = furi_hal_power_get_pct();

    FuriHalAdcHandle* adc = furi_hal_adc_acquire();
    if(adc) {
        furi_hal_adc_configure(adc);
        uint16_t raw = furi_hal_adc_read(adc, FuriHalAdcChannelTEMPSENSOR);
        app->cpu_temp = furi_hal_adc_convert_temp(adc, raw);
        furi_hal_adc_release(adc);
    } else {
        app->cpu_temp = 0.0f;
    }
}

/* ---- Build and send a sensor JSON line over BLE ---- */

static void send_sensor_reading(RelayApp* app, const char* sensor_type,
                                const char* unit, float value) {
    if(!app->ble_connected || !app->ble_profile) return;

    char json[BLE_TX_MAX];
    int len;

    /* Build compact JSON the Android LocalRelayService can parse */
    if(app->key_loaded) {
        /* Sign the value string with HMAC-SHA256 for authentication */
        char val_str[32];
        snprintf(val_str, sizeof(val_str), "%.4f", (double)value);
        uint8_t sig[32];
        hmac_sha256(app->key, 32, (const uint8_t*)val_str, strlen(val_str), sig);
        char sig_hex[65];
        for(int i=0; i<32; i++) snprintf(sig_hex+i*2, 3, "%02x", sig[i]);
        sig_hex[64]='\0';

        len = snprintf(json, sizeof(json),
            "{\"id\":\"%s/flipper-zero\","
            "\"value\":%.4f,"
            "\"metadata\":{"
              "\"type\":\"%s\","
              "\"unit\":\"%s\","
              "\"battery\":%u,"
              "\"source\":\"flipper-ble\","
              "\"sig\":\"%s\""
            "}}\n",
            app->account, (double)value,
            sensor_type, unit,
            app->battery_pct,
            sig_hex);
    } else {
        len = snprintf(json, sizeof(json),
            "{\"id\":\"%s/flipper-zero\","
            "\"value\":%.4f,"
            "\"metadata\":{"
              "\"type\":\"%s\","
              "\"unit\":\"%s\","
              "\"battery\":%u,"
              "\"source\":\"flipper-ble\""
            "}}\n",
            app->account, (double)value,
            sensor_type, unit,
            app->battery_pct);
    }

    if(len <= 0 || len >= (int)sizeof(json)) return;

    if(ble_profile_serial_tx(app->ble_profile, (uint8_t*)json, (uint16_t)len)) {
        app->tx_count++;
    }
}

/* ---- BLE callbacks ---- */

static void bt_status_cb(BtStatus status, void* ctx) {
    RelayApp* app = ctx;
    furi_mutex_acquire(app->mutex, FuriWaitForever);
    app->ble_connected = (status == BtStatusConnected);
    furi_mutex_release(app->mutex);
    FURI_LOG_I(TAG, "BLE status: %d", status);
}

static uint16_t ble_serial_cb(SerialServiceEvent event, void* ctx) {
    RelayApp* app = ctx;
    if(event.event == SerialServiceEventTypeDataReceived) {
        /* Relay received bytes to USB CDC for passthrough */
        if(event.data.buffer && event.data.size > 0) {
            furi_hal_cdc_send(0, event.data.buffer, (uint16_t)event.data.size);
            app->rx_count++;
        }
    }
    return 0;
}

/* ---- Drawing ---- */

static void draw_cb(Canvas* c, void* ctx) {
    RelayApp* app = ctx;
    furi_mutex_acquire(app->mutex, FuriWaitForever);

    canvas_clear(c);
    canvas_set_font(c, FontPrimary);
    canvas_draw_str(c, 2, 11, "BTCPC BLE Relay");
    canvas_draw_line(c, 0, 14, 128, 14);

    canvas_set_font(c, FontSecondary);

    /* BLE status */
    char ble_line[48];
    if(app->ble_connected) {
        snprintf(ble_line, sizeof(ble_line), "BLE: Connected");
    } else {
        snprintf(ble_line, sizeof(ble_line), "BLE: Waiting...");
    }
    canvas_draw_str(c, 2, 25, ble_line);

    /* Account + short MAC */
    char id_line[48];
    snprintf(id_line, sizeof(id_line), "ID: %s (%.8s)", app->account, app->mac);
    canvas_draw_str(c, 2, 35, id_line);

    /* TX/RX counters */
    char stats_line[48];
    snprintf(stats_line, sizeof(stats_line), "TX:%-4lu  RX:%-4lu",
             (unsigned long)app->tx_count, (unsigned long)app->rx_count);
    canvas_draw_str(c, 2, 45, stats_line);

    /* Sensors */
    char sensor_line[48];
    if(app->cpu_temp > 0.0f) {
        snprintf(sensor_line, sizeof(sensor_line), "Bat:%u%%  CPU:%.1fC",
                 app->battery_pct, (double)app->cpu_temp);
    } else {
        snprintf(sensor_line, sizeof(sensor_line), "Battery: %u%%", app->battery_pct);
    }
    canvas_draw_str(c, 2, 55, sensor_line);

    /* Key status */
    canvas_draw_str(c, 2, 63, app->key_loaded ? "Key:OK  Back=exit" : "Key:-   Back=exit");

    furi_mutex_release(app->mutex);
}

static void input_cb(InputEvent* ev, void* ctx) {
    FuriMessageQueue* q = ctx;
    furi_message_queue_put(q, ev, FuriWaitForever);
}

/* ---- Main ---- */

int32_t btcpc_ble_relay_app(void* p) {
    UNUSED(p);

    RelayApp* app = malloc(sizeof(RelayApp));
    if(!app) return 1;
    memset(app, 0, sizeof(RelayApp));

    app->mutex       = furi_mutex_alloc(FuriMutexTypeNormal);
    app->input_queue = furi_message_queue_alloc(8, sizeof(InputEvent));
    app->running     = true;

    /* Load identity */
    if(!load_account(app->account)) {
        strncpy(app->account, "user", sizeof(app->account) - 1);
    }
    app->key_loaded = load_posting_key(app->key);

    /* BLE MAC (LSB-first from HAL → reverse to MSB-first for display) */
    const uint8_t* mac = furi_hal_version_get_ble_mac();
    snprintf(app->mac, sizeof(app->mac),
             "%02X:%02X:%02X:%02X:%02X:%02X",
             mac[5], mac[4], mac[3], mac[2], mac[1], mac[0]);

    /* Read sensors once at startup */
    read_sensors(app);

    /* USB CDC setup (preserve existing config) */
    FuriHalUsbInterface* usb_prev = furi_hal_usb_get_config();
    bool usb_changed = (usb_prev != &usb_cdc_single);
    if(usb_changed) {
        furi_hal_usb_unlock();
        furi_hal_usb_set_config(&usb_cdc_single, NULL);
        furi_delay_ms(200);
    }

    /* Start BLE serial profile (NUS) */
    app->bt = furi_record_open(RECORD_BT);
    bt_set_status_changed_callback(app->bt, bt_status_cb, app);
    app->ble_profile = bt_profile_start(app->bt, ble_profile_serial, NULL);
    if(app->ble_profile) {
        ble_profile_serial_set_event_callback(
            app->ble_profile, 256, ble_serial_cb, app);
        FURI_LOG_I(TAG, "BLE NUS profile started");
    } else {
        FURI_LOG_E(TAG, "Failed to start BLE serial profile");
    }

    /* GUI */
    ViewPort* vp = view_port_alloc();
    view_port_draw_callback_set(vp, draw_cb, app);
    view_port_input_callback_set(vp, input_cb, app->input_queue);
    Gui* gui = furi_record_open(RECORD_GUI);
    gui_add_view_port(gui, vp, GuiLayerFullscreen);

    /* Event loop */
    InputEvent ev;
    uint32_t   last_sensor  = furi_get_tick();
    uint32_t   last_sensor_send = furi_get_tick();

    while(app->running) {
        /* Handle input */
        if(furi_message_queue_get(app->input_queue, &ev, 50) == FuriStatusOk) {
            if(ev.type == InputTypeShort || ev.type == InputTypePress) {
                if(ev.key == InputKeyBack) {
                    app->running = false;
                }
            }
        }

        uint32_t now = furi_get_tick();

        /* Refresh sensor readings for display every second */
        if(now - last_sensor >= 1000) {
            furi_mutex_acquire(app->mutex, FuriWaitForever);
            read_sensors(app);
            furi_mutex_release(app->mutex);
            last_sensor = now;
            view_port_update(vp);
        }

        /* Send sensor packet over BLE every SENSOR_INTERVAL ms when connected */
        if(app->ble_connected && (now - last_sensor_send >= SENSOR_INTERVAL)) {
            /* Send battery as primary reading */
            send_sensor_reading(app, "battery", "%", (float)app->battery_pct);
            /* Send CPU temperature as a separate reading */
            if(app->cpu_temp > 0.0f) {
                furi_delay_ms(20); /* small gap so Android can parse two packets */
                send_sensor_reading(app, "cpu_temp", "C", app->cpu_temp);
            }
            last_sensor_send = now;
        }

        /* Pass USB CDC → BLE when connected */
        if(app->ble_profile && app->ble_connected) {
            uint8_t cdc_buf[64];
            int n = furi_hal_cdc_receive(0, cdc_buf, sizeof(cdc_buf));
            if(n > 0) {
                if(ble_profile_serial_tx(app->ble_profile, cdc_buf, (uint16_t)n)) {
                    app->tx_count++;
                }
            }
        }
    }

    /* Cleanup */
    gui_remove_view_port(gui, vp);
    furi_record_close(RECORD_GUI);
    view_port_free(vp);

    if(app->ble_profile) {
        bt_profile_restore_default(app->bt);
    }
    bt_set_status_changed_callback(app->bt, NULL, NULL);
    furi_record_close(RECORD_BT);

    furi_message_queue_free(app->input_queue);
    furi_mutex_free(app->mutex);
    memset(app->key, 0, sizeof(app->key));
    free(app);

    if(usb_changed) {
        furi_hal_usb_set_config(usb_prev, NULL);
        furi_hal_usb_lock();
    }

    return 0;
}
