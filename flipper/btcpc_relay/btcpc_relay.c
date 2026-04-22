/**
 * btcpc_relay.c — BTCPC Relay Flipper Zero app
 *
 * Guides the user through pairing their Flipper with the BTCPC Android app
 * over BLE, and shows the device's BLE MAC address for manual entry if the
 * automatic scan fails.
 *
 * Pages (Left / Right to navigate, Back to exit from page 0):
 *   0 — Welcome + pairing overview
 *   1 — Step 1: open BLE UART Bridge on the Flipper
 *   2 — Step 2: your BLE MAC address (for manual entry in the app)
 *   3 — Step 3: connect via USB or tap Pair in the Android app
 *   4 — Ready / waiting screen
 *
 * Build: copy this directory into your Flipper firmware's applications/external/
 * or use ufbt:  ufbt build  (from this directory)
 */

#include <furi.h>
#include <gui/gui.h>
#include <input/input.h>
#include <furi_hal_bt.h>

#define TAG          "BTCPCRelay"
#define PAGE_COUNT   5

typedef struct {
    FuriMutex* mutex;
    uint8_t    page;
    char       mac[18]; /* "AA:BB:CC:DD:EE:FF\0" */
} RelayApp;

/* ------------------------------------------------------------------ drawing */

static void draw_header(Canvas* c, const char* title) {
    canvas_set_font(c, FontPrimary);
    canvas_draw_str(c, 2, 11, title);
    canvas_draw_line(c, 0, 14, 128, 14);
}

static void draw_nav(Canvas* c, bool has_prev, bool has_next) {
    canvas_set_font(c, FontSecondary);
    if (has_prev) canvas_draw_str(c, 2,  63, "<");
    if (has_next) canvas_draw_str(c, 120, 63, ">");
}

static void draw_callback(Canvas* c, void* ctx) {
    RelayApp* app = ctx;
    furi_mutex_acquire(app->mutex, FuriWaitForever);

    canvas_clear(c);

    switch (app->page) {

    /* ---- page 0: welcome ---- */
    case 0:
        draw_header(c, "BTCPC Relay");
        canvas_set_font(c, FontSecondary);
        canvas_draw_str(c, 2, 26, "Earn BTCPC by relaying");
        canvas_draw_str(c, 2, 36, "sensor data to the chain.");
        canvas_draw_str(c, 2, 50, "Follow steps 1-3 to pair");
        canvas_draw_str(c, 2, 60, "with your Android phone.");
        draw_nav(c, false, true);
        break;

    /* ---- page 1: step 1 — enable BLE UART Bridge ---- */
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

    /* ---- page 2: step 2 — show BLE MAC ---- */
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

    /* ---- page 3: step 3 — connect phone ---- */
    case 3:
        draw_header(c, "Step 3 of 3");
        canvas_set_font(c, FontSecondary);
        canvas_draw_str(c, 2, 25, "On your Android phone:");
        canvas_draw_str(c, 2, 35, "  Open BTCPC app");
        canvas_draw_str(c, 2, 45, "  Tap Flipper tab");
        canvas_draw_str(c, 2, 55, "  Plug USB  OR");
        canvas_draw_str(c, 2, 63, "  Tap \"Pair via BLE\"");
        draw_nav(c, true, true);
        break;

    /* ---- page 4: ready / waiting ---- */
    case 4:
        draw_header(c, "Ready");
        canvas_set_font(c, FontSecondary);
        canvas_draw_str(c, 2, 25, "Waiting for BTCPC app...");
        canvas_draw_str(c, 2, 38, "Keep BLE UART Bridge");
        canvas_draw_str(c, 2, 48, "open in background.");
        canvas_draw_str(c, 2, 60, "Back = exit");
        draw_nav(c, true, false);
        break;

    default:
        break;
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

    RelayApp* app  = malloc(sizeof(RelayApp));
    app->mutex     = furi_mutex_alloc(FuriMutexTypeNormal);
    app->page      = 0;

    /* Read BLE public address (little-endian, reversed for display) */
    uint8_t mac[6] = {0};
    furi_hal_bt_get_public_address(mac);
    snprintf(app->mac, sizeof(app->mac),
             "%02X:%02X:%02X:%02X:%02X:%02X",
             mac[5], mac[4], mac[3], mac[2], mac[1], mac[0]);

    FURI_LOG_I(TAG, "BLE MAC: %s", app->mac);

    FuriMessageQueue* queue  = furi_message_queue_alloc(8, sizeof(InputEvent));
    ViewPort*         vp     = view_port_alloc();
    view_port_draw_callback_set(vp, draw_callback, app);
    view_port_input_callback_set(vp, input_callback, queue);

    Gui* gui = furi_record_open(RECORD_GUI);
    gui_add_view_port(gui, vp, GuiLayerFullscreen);

    bool       running = true;
    InputEvent ev;

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
            default:
                break;
            }
            furi_mutex_release(app->mutex);
            view_port_update(vp);
        }
    }

    gui_remove_view_port(gui, vp);
    furi_record_close(RECORD_GUI);
    view_port_free(vp);
    furi_message_queue_free(queue);
    furi_mutex_free(app->mutex);
    free(app);

    return 0;
}
