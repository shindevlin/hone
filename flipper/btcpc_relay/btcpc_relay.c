/**
 * btcpc_relay.c — BTCPC Relay Flipper Zero app
 *
 * Announces the Flipper's BLE MAC over USB CDC so the PC pairing tool
 * can push it to the BTCPC Android app.  If a posting key is stored in
 * /int/.btcpc/posting.key, the app computes HMAC-SHA256(mac, key) and
 * includes the signature — the key itself never leaves the Flipper.
 *
 * Announce format (one line, newline-terminated):
 *   With key:    BTCPC_MAC:<mac> BTCPC_SIG:<64-char-hex-hmac>\n
 *   Without key: BTCPC_MAC:<mac>\n
 *
 * The phone/PC verifies the HMAC using the account's known posting key.
 * This proves the Flipper belongs to the account without exposing the key.
 *
 * Key file: /int/.btcpc/posting.key
 *   - Flipper internal flash, NOT accessible via USB mass storage
 *   - NOT visible in the Flipper file browser
 *   - Written once by the btcpcflipper pairing tool: scripts/set-flipper-key.js
 *
 * Pages: Left/Right to navigate, Back from page 0 to exit.
 *   0  Welcome          3  Step 3 of 3
 *   1  Step 1 of 3      4  Ready (shows key status)
 *   2  Step 2 of 3
 *
 * Build:  ufbt build
 */

#include <furi.h>
#include <gui/gui.h>
#include <input/input.h>
#include <storage/storage.h>
#include <furi_hal_version.h>
#include <furi_hal_usb.h>
#include <furi_hal_usb_cdc.h>
#include <string.h>
#include "sha256.h"

#define TAG            "BTCPCRelay"
#define PAGE_COUNT     5
#define KEY_PATH       "/int/.btcpc/posting.key"
#define KEY_HEX_LEN    64

typedef struct {
    FuriMutex* mutex;
    uint8_t    page;
    char       mac[18];              /* "AA:BB:CC:DD:EE:FF\0" */
    uint8_t    key_bytes[32];        /* raw key, never displayed */
    bool       key_loaded;
} RelayApp;

/* ------------------------------------------------------------------ key I/O */

static bool load_posting_key(uint8_t out_bytes[32]) {
    Storage* storage = furi_record_open(RECORD_STORAGE);
    File*    file    = storage_file_alloc(storage);
    bool     ok      = false;

    if(storage_file_open(file, KEY_PATH, FSAM_READ, FSOM_OPEN_EXISTING)) {
        char hex[KEY_HEX_LEN + 4];
        uint16_t n = storage_file_read(file, hex, sizeof(hex) - 1);
        storage_file_close(file);
        while(n > 0 && (hex[n-1]=='\n'||hex[n-1]=='\r'||hex[n-1]==' ')) n--;
        hex[n] = '\0';
        if(n == KEY_HEX_LEN) {
            ok = true;
            for(int i = 0; i < 32; i++) {
                uint8_t hi, lo;
                char c;
                c = hex[i*2];
                hi = (c>='0'&&c<='9') ? c-'0' : (c>='a'&&c<='f') ? c-'a'+10 :
                     (c>='A'&&c<='F') ? c-'A'+10 : 255;
                c = hex[i*2+1];
                lo = (c>='0'&&c<='9') ? c-'0' : (c>='a'&&c<='f') ? c-'a'+10 :
                     (c>='A'&&c<='F') ? c-'A'+10 : 255;
                if(hi==255 || lo==255) { ok = false; break; }
                out_bytes[i] = (hi << 4) | lo;
            }
        }
    }

    storage_file_free(file);
    furi_record_close(RECORD_STORAGE);
    return ok;
}

/* ------------------------------------------------------------------ drawing */

static void draw_header(Canvas* c, const char* title) {
    canvas_set_font(c, FontPrimary);
    canvas_draw_str(c, 2, 11, title);
    canvas_draw_line(c, 0, 14, 128, 14);
}

static void draw_nav(Canvas* c, bool has_prev, bool has_next) {
    canvas_set_font(c, FontSecondary);
    if(has_prev) canvas_draw_str(c, 2,   63, "<");
    if(has_next) canvas_draw_str(c, 120, 63, ">");
}

static void draw_callback(Canvas* c, void* ctx) {
    RelayApp* app = ctx;
    furi_mutex_acquire(app->mutex, FuriWaitForever);
    canvas_clear(c);

    switch(app->page) {
    case 0:
        draw_header(c, "BTCPC Relay");
        canvas_set_font(c, FontSecondary);
        canvas_draw_str(c, 2, 26, "Earn BTCPC by relaying");
        canvas_draw_str(c, 2, 36, "sensor data to the chain.");
        canvas_draw_str(c, 2, 50, "Follow steps 1-3 to pair");
        canvas_draw_str(c, 2, 60, "with your Android phone.");
        draw_nav(c, false, true);
        break;
    case 1:
        draw_header(c, "Step 1 of 3");
        canvas_set_font(c, FontSecondary);
        canvas_draw_str(c, 2, 25, "Enable BLE UART Bridge:");
        canvas_draw_str(c, 2, 35, "  Apps > GPIO >");
        canvas_draw_str(c, 2, 45, "  UART BLE Bridge");
        canvas_draw_str(c, 2, 55, "Leave it running, then");
        canvas_draw_str(c, 2, 63, "press > to continue.");
        draw_nav(c, true, true);
        break;
    case 2:
        draw_header(c, "Step 2 of 3");
        canvas_set_font(c, FontSecondary);
        canvas_draw_str(c, 2, 25, "Your BLE address:");
        canvas_set_font(c, FontPrimary);
        canvas_draw_str(c, 2, 38, app->mac);
        canvas_set_font(c, FontSecondary);
        canvas_draw_str(c, 2, 50, "If auto-scan fails, enter");
        canvas_draw_str(c, 2, 60, "this in the BTCPC app.");
        draw_nav(c, true, true);
        break;
    case 3:
        draw_header(c, "Step 3 of 3");
        canvas_set_font(c, FontSecondary);
        canvas_draw_str(c, 2, 25, "On your Android phone:");
        canvas_draw_str(c, 2, 35, "  Open BTCPC app");
        canvas_draw_str(c, 2, 45, "  Tap Flipper tab");
        canvas_draw_str(c, 2, 55, "  Plug USB (auto-pairs)");
        canvas_draw_str(c, 2, 63, "  OR tap \"Pair via BLE\"");
        draw_nav(c, true, true);
        break;
    case 4:
        draw_header(c, "Ready");
        canvas_set_font(c, FontSecondary);
        canvas_draw_str(c, 2, 25, "Waiting for BTCPC app...");
        canvas_draw_str(c, 2, 38, "Keep BLE UART Bridge");
        canvas_draw_str(c, 2, 48, "open in background.");
        canvas_draw_str(c, 2, 58, app->key_loaded ? "Key: loaded" : "Key: MISSING");
        canvas_draw_str(c, 2, 63, "Back = exit");
        draw_nav(c, true, false);
        break;
    default: break;
    }

    furi_mutex_release(app->mutex);
}

/* ------------------------------------------------------------------ input */

static void input_callback(InputEvent* event, void* ctx) {
    FuriMessageQueue* q = ctx;
    furi_message_queue_put(q, event, FuriWaitForever);
}

/* ------------------------------------------------------------------ main */

int32_t btcpc_relay_app(void* p) {
    UNUSED(p);

    RelayApp* app   = malloc(sizeof(RelayApp));
    app->mutex      = furi_mutex_alloc(FuriMutexTypeNormal);
    app->page       = 0;
    app->key_loaded = false;
    memset(app->key_bytes, 0, sizeof(app->key_bytes));

    /* BLE MAC — LSB-first from HAL, reversed for Android */
    const uint8_t* mac = furi_hal_version_get_ble_mac();
    snprintf(app->mac, sizeof(app->mac),
             "%02X:%02X:%02X:%02X:%02X:%02X",
             mac[5], mac[4], mac[3], mac[2], mac[1], mac[0]);
    FURI_LOG_I(TAG, "BLE MAC: %s", app->mac);

    /* Load posting key — bytes never leave the device */
    app->key_loaded = load_posting_key(app->key_bytes);
    FURI_LOG_I(TAG, app->key_loaded ? "Posting key loaded" : "Posting key not found");

    /* USB CDC setup */
    FuriHalUsbInterface* usb_prev = furi_hal_usb_get_config();
    bool usb_changed = (usb_prev != &usb_cdc_single);
    if(usb_changed) {
        furi_hal_usb_unlock();
        furi_hal_usb_set_config(&usb_cdc_single, NULL);
        furi_delay_ms(200);
    }

    /* Build announce: MAC + HMAC-SHA256(mac_string, key) if key is present */
    char announce[128];
    int  announce_len;
    if(app->key_loaded) {
        uint8_t sig[32];
        hmac_sha256(app->key_bytes, 32,
                    (const uint8_t*)app->mac, strlen(app->mac),
                    sig);
        /* encode sig as 64 hex chars */
        char sig_hex[65];
        for(int i = 0; i < 32; i++)
            snprintf(sig_hex + i*2, 3, "%02x", sig[i]);
        sig_hex[64] = '\0';
        announce_len = snprintf(announce, sizeof(announce),
                                "BTCPC_MAC:%s BTCPC_SIG:%s\n",
                                app->mac, sig_hex);
    } else {
        announce_len = snprintf(announce, sizeof(announce),
                                "BTCPC_MAC:%s\n", app->mac);
    }
    furi_hal_cdc_send(0, (uint8_t*)announce, (uint16_t)announce_len);

    FuriMessageQueue* queue = furi_message_queue_alloc(8, sizeof(InputEvent));
    ViewPort*         vp    = view_port_alloc();
    view_port_draw_callback_set(vp, draw_callback, app);
    view_port_input_callback_set(vp, input_callback, queue);

    Gui* gui = furi_record_open(RECORD_GUI);
    gui_add_view_port(gui, vp, GuiLayerFullscreen);

    bool       running        = true;
    InputEvent ev;
    uint32_t   last_announce  = furi_get_tick();
    const uint32_t ANNOUNCE_INTERVAL = 5000;

    while(running) {
        if(furi_message_queue_get(queue, &ev, 100) == FuriStatusOk) {
            if(ev.type != InputTypePress && ev.type != InputTypeShort) continue;
            furi_mutex_acquire(app->mutex, FuriWaitForever);
            switch(ev.key) {
            case InputKeyRight:
            case InputKeyOk:
                if(app->page < PAGE_COUNT - 1) app->page++;
                break;
            case InputKeyLeft:
                if(app->page > 0) app->page--;
                break;
            case InputKeyBack:
                if(app->page == 0) running = false;
                else               app->page = 0;
                break;
            default: break;
            }
            furi_mutex_release(app->mutex);
            view_port_update(vp);
        }

        uint32_t now = furi_get_tick();
        if(now - last_announce >= ANNOUNCE_INTERVAL) {
            furi_hal_cdc_send(0, (uint8_t*)announce, (uint16_t)announce_len);
            last_announce = now;
        }
    }

    gui_remove_view_port(gui, vp);
    furi_record_close(RECORD_GUI);
    view_port_free(vp);
    furi_message_queue_free(queue);
    /* Zero key bytes before freeing */
    memset(app->key_bytes, 0, sizeof(app->key_bytes));
    furi_mutex_free(app->mutex);
    free(app);

    if(usb_changed) {
        furi_hal_usb_set_config(usb_prev, NULL);
        furi_hal_usb_lock();
    }

    return 0;
}
