#include <furi.h>
#include <furi_hal_adc.h>
#include <furi_hal_bt.h>
#include <furi_hal_gpio.h>
#include <furi_hal_random.h>
#include <furi_hal_nfc.h>
#include <furi_hal_power.h>
#include <furi_hal_subghz.h>
#include <furi_hal_crypto.h>
#include <subghz/devices/devices.h>
#include <subghz/devices/cc1101_configs.h>
#include <gui/gui.h>
#include <gui/canvas.h>
#include <gui/modules/submenu.h>
#include <gui/modules/text_box.h>
#include <gui/view.h>
#include <gui/view_dispatcher.h>
#include <storage/storage.h>
#include <toolbox/version.h>
#include <stdlib.h>
#include <string.h>

#include "hone_sensor.h"
#include "qrcodegen.h"

#define HONE_VIEW_MENU 0
#define HONE_VIEW_RESULT 1
#define HONE_VIEW_PAIR 2
#define HONE_VIEW_DIAGNOSTICS 3

#define HONE_MENU_BATTERY 0
#define HONE_MENU_CPU_TEMP 1
#define HONE_MENU_GPIO 2
#define HONE_MENU_NFC 3
#define HONE_MENU_BLE 4
#define HONE_MENU_SUBGHZ 5
#define HONE_MENU_CYCLE 6
#define HONE_MENU_PAIR 7
#define HONE_MENU_UNPAIR 8
#define HONE_MENU_DIAGNOSTICS 9
#define HONE_MENU_DIAGNOSTICS_SEND 10

static const char* HONE_SUBGHZ_DEVICE = "cc1101_int";
static const uint32_t HONE_SUBGHZ_FREQ = 433920000UL;

static const uint32_t hone_cycle_order[] = {
    HONE_MENU_BATTERY,
    HONE_MENU_CPU_TEMP,
    HONE_MENU_GPIO,
    HONE_MENU_NFC,
    HONE_MENU_BLE,
    HONE_MENU_SUBGHZ,
};

typedef struct {
    Gui* gui;
    Storage* storage;
    ViewDispatcher* view_dispatcher;
    Submenu* submenu;
    Submenu* diagnostics_submenu;
    TextBox* text_box;
    View* pair_view;
    char text[256];
    char result[256];
    uint32_t active_view;
    bool cycle_active;
    size_t cycle_index;
    uint32_t pair_id;
    uint32_t beacon_seq;
    char pair_token[32];
    char device_id[32];
    uint8_t posting_key[32];
    bool identity_ready;
} HoneSensorApp;

typedef struct {
    char title[24];
    char token[32];
    uint8_t qrcode[qrcodegen_BUFFER_LEN_FOR_VERSION(1)];
    uint8_t qr_size;
    bool valid;
} HonePairViewModel;

static void refresh_pair_id(HoneSensorApp* app) {
    if(!app) return;
    uint32_t next = furi_hal_random_get();
    if(next == 0) next = 0xB7C0C001u;
    app->pair_id = next;
    app->beacon_seq = 1;
    snprintf(app->pair_token, sizeof(app->pair_token), "HONE:PAIR:%08lX", (unsigned long)app->pair_id);
}

static void hex_encode_upper(const uint8_t* input, size_t input_len, char* out, size_t out_len) {
    static const char* digits = "0123456789ABCDEF";
    if(!out || out_len == 0) return;
    size_t pos = 0;
    for(size_t i = 0; i < input_len && pos + 1 < out_len; i++) {
        out[pos++] = digits[(input[i] >> 4) & 0xF];
        if(pos + 1 < out_len) {
            out[pos++] = digits[input[i] & 0xF];
        }
    }
    out[pos] = '\0';
}

static bool load_or_create_posting_key(HoneSensorApp* app) {
    if(!app || !app->storage) return false;

    File* file = storage_file_alloc(app->storage);
    if(!file) return false;

    const char* key_path = APP_DATA_PATH("hone_sensor.key");
    bool ok = false;
    uint8_t key[32] = {0};
    if(storage_file_open(file, key_path, FSAM_READ_WRITE, FSOM_OPEN_ALWAYS)) {
        size_t read = storage_file_read(file, key, sizeof(key));
        if(read < sizeof(key)) {
            for(size_t i = 0; i < sizeof(key); i++) {
                key[i] = (uint8_t)furi_hal_random_get();
            }
            storage_file_seek(file, 0, true);
            storage_file_truncate(file);
            ok = storage_file_write(file, key, sizeof(key)) == sizeof(key);
            storage_file_sync(file);
        } else {
            ok = true;
        }
        storage_file_close(file);
    }

    storage_file_free(file);
    if(ok) {
        memcpy(app->posting_key, key, sizeof(key));
    }
    return ok;
}

static void init_device_identity(HoneSensorApp* app) {
    if(!app) return;
    const uint8_t* uid = furi_hal_version_uid();
    size_t uid_len = furi_hal_version_uid_size();
    if(uid && uid_len > 0) {
        hex_encode_upper(uid, uid_len, app->device_id, sizeof(app->device_id));
    }
    if(app->device_id[0] == '\0') {
        snprintf(app->device_id, sizeof(app->device_id), "FLIPPER-%08lX", (unsigned long)furi_hal_random_get());
    }
    app->identity_ready = load_or_create_posting_key(app);
}

static void update_pair_view_model(HoneSensorApp* app, const char* title) {
    if(!app || !app->pair_view) return;
    HonePairViewModel* model = view_get_model(app->pair_view);
    if(!model) return;

    memset(model, 0, sizeof(*model));
    snprintf(model->title, sizeof(model->title), "%s", title ? title : "Pair Phone");
    snprintf(model->token, sizeof(model->token), "%s", app->pair_token);

    uint8_t temp[qrcodegen_BUFFER_LEN_FOR_VERSION(1)] = {0};
    model->valid = qrcodegen_encodeText(
        model->token,
        temp,
        model->qrcode,
        qrcodegen_Ecc_LOW,
        1,
        1,
        qrcodegen_Mask_AUTO,
        true);
    model->qr_size = model->valid ? (uint8_t)qrcodegen_getSize(model->qrcode) : 0;
    view_commit_model(app->pair_view, true);
}

static void draw_qr(Canvas* canvas, const HonePairViewModel* model) {
    if(!model || !model->valid || model->qr_size == 0) {
        canvas_draw_str(canvas, 8, 32, "QR unavailable");
        return;
    }

    const int32_t scale = 1;
    const int32_t quiet_zone = 4;
    const int32_t qr_px = (int32_t)(model->qr_size + quiet_zone * 2) * scale;
    const int32_t origin_x = (int32_t)((canvas_width(canvas) - qr_px) / 2);
    const int32_t origin_y = 14;

    canvas_set_color(canvas, ColorBlack);
    for(int32_t y = -quiet_zone; y < (int32_t)model->qr_size + quiet_zone; y++) {
        for(int32_t x = -quiet_zone; x < (int32_t)model->qr_size + quiet_zone; x++) {
            bool dark = false;
            if(x >= 0 && y >= 0 && x < (int32_t)model->qr_size && y < (int32_t)model->qr_size) {
                dark = qrcodegen_getModule(model->qrcode, x, y);
            }
            if(dark) {
                canvas_draw_box(
                    canvas,
                    origin_x + (x + quiet_zone) * scale,
                    origin_y + (y + quiet_zone) * scale,
                    scale,
                    scale);
            }
        }
    }
}

static void pair_view_draw(Canvas* canvas, void* model) {
    const HonePairViewModel* pair_model = model;
    canvas_clear(canvas);
    canvas_set_color(canvas, ColorBlack);
    canvas_set_font(canvas, FontPrimary);
    canvas_draw_str(canvas, 4, 8, pair_model && pair_model->title[0] ? pair_model->title : "Pair Phone");
    draw_qr(canvas, pair_model);
    canvas_set_font(canvas, FontSecondary);
    canvas_draw_str(
        canvas,
        4,
        62,
        pair_model && pair_model->token[0] ? pair_model->token : "HONE:PAIR:00000000");
}

static bool navigation_callback(void* context) {
    HoneSensorApp* app = context;
    if(!app) return true;
    if(app->active_view == HONE_VIEW_PAIR || app->active_view == HONE_VIEW_DIAGNOSTICS) {
        app->active_view = HONE_VIEW_MENU;
        view_dispatcher_switch_to_view(app->view_dispatcher, HONE_VIEW_MENU);
        return true;
    }
    if(app->active_view == HONE_VIEW_RESULT) {
        if(app->cycle_active) {
            app->cycle_active = false;
            app->cycle_index = 0;
            app->active_view = HONE_VIEW_MENU;
            view_dispatcher_switch_to_view(app->view_dispatcher, HONE_VIEW_MENU);
            return true;
        }
        app->active_view = HONE_VIEW_MENU;
        view_dispatcher_switch_to_view(app->view_dispatcher, HONE_VIEW_MENU);
        return true;
    }
    view_dispatcher_stop(app->view_dispatcher);
    return true;
}

static void show_result(HoneSensorApp* app, const char* title, const char* body) {
    if(!app || !app->text_box) return;
    snprintf(
        app->result,
        sizeof(app->result),
        "%s\n\n%s\n\nBack = return to menu",
        title ? title : "HONE Sensor",
        body ? body : "");
    text_box_set_text(app->text_box, app->result);
    text_box_set_focus(app->text_box, TextBoxFocusStart);
    app->active_view = HONE_VIEW_RESULT;
    view_dispatcher_switch_to_view(app->view_dispatcher, HONE_VIEW_RESULT);
}

static void probe_cycle_next(HoneSensorApp* app);
static bool broadcast_beacon(HoneSensorApp* app, const char* source, const char* reading);
static void send_diagnostics(HoneSensorApp* app);

static void show_pair_token(HoneSensorApp* app, const char* heading, const char* body) {
    if(!app) return;
    (void)body;
    update_pair_view_model(app, heading ? heading : "Pair Phone");
    app->active_view = HONE_VIEW_PAIR;
    view_dispatcher_switch_to_view(app->view_dispatcher, HONE_VIEW_PAIR);
}

static void show_diagnostics(HoneSensorApp* app) {
    if(!app || !app->diagnostics_submenu) return;
    app->active_view = HONE_VIEW_DIAGNOSTICS;
    view_dispatcher_switch_to_view(app->view_dispatcher, HONE_VIEW_DIAGNOSTICS);
}

static void derive_packet_iv(uint32_t pair_id, uint32_t seq, uint8_t iv[12]) {
    memset(iv, 0, 12);
    memcpy(iv, &pair_id, sizeof(pair_id));
    memcpy(iv + 4, &seq, sizeof(seq));
}

static void append_part(char* out, size_t out_size, const char* part) {
    if(!out || !part || out_size == 0) return;
    size_t len = strlen(out);
    if(len >= out_size - 1) return;
    if(len > 0) {
        if(len < out_size - 1) {
            out[len++] = '|';
            out[len] = '\0';
        }
    }
    while(*part != '\0' && len < out_size - 1) {
        out[len++] = *part++;
    }
    out[len] = '\0';
}

static bool build_signed_packet(
    HoneSensorApp* app,
    const char* source,
    const char* reading,
    char* packet,
    size_t packet_size) {
    if(!app || !packet || packet_size == 0 || !app->identity_ready) {
        return false;
    }

    char base[192];
    char reading_buf[64];
    const char* safe_source = source ? source : "unknown";
    const char* safe_reading = reading ? reading : "";
    snprintf(reading_buf, sizeof(reading_buf), "%s", safe_reading);
    uint32_t seq = app->beacon_seq++;
    int written = snprintf(
        base,
        sizeof(base),
        "HONE1|dev=%s|sid=%08lX|seq=%lu|src=%s|%s",
        app->device_id,
        (unsigned long)app->pair_id,
        (unsigned long)seq,
        safe_source,
        reading_buf);
    if(written <= 0 || (size_t)written >= sizeof(base)) return false;

    uint8_t iv[12] = {0};
    derive_packet_iv(app->pair_id, seq, iv);

    uint8_t tag[16] = {0};
    uint8_t dummy_in = 0;
    uint8_t dummy_out = 0;
    if(!furi_hal_crypto_gcm_encrypt_and_tag(
           app->posting_key,
           iv,
           (const uint8_t*)base,
           (size_t)written,
           &dummy_in,
           &dummy_out,
           0,
           tag)) {
        return false;
    }

    char sig_hex[33];
    hex_encode_upper(tag, sizeof(tag), sig_hex, sizeof(sig_hex));
    written = snprintf(packet, packet_size, "%s|tag=%s", base, sig_hex);
    return written > 0 && (size_t)written < packet_size;
}

static void send_diagnostics(HoneSensorApp* app) {
    if(!app) return;

    uint8_t battery_pct = furi_hal_power_get_pct();
    float battery_voltage = furi_hal_power_get_battery_voltage(FuriHalPowerICFuelGauge);

    FuriHalAdcHandle* adc = furi_hal_adc_acquire();
    uint16_t cpu_raw = 0;
    float cpu_temp = 0.0f;
    uint16_t gpio_raw = 0;
    float gpio_mv = 0.0f;
    if(adc) {
        furi_hal_adc_configure(adc);
        cpu_raw = furi_hal_adc_read(adc, FuriHalAdcChannelTEMPSENSOR);
        cpu_temp = furi_hal_adc_convert_temp(adc, cpu_raw);
        gpio_raw = furi_hal_adc_read(adc, FuriHalAdcChannel4);
        gpio_mv = furi_hal_adc_convert_to_voltage(adc, gpio_raw);
        furi_hal_adc_release(adc);
    }

    bool nfc_present = false;
    bool nfc_ok = false;
    if(furi_hal_nfc_acquire() == FuriHalNfcErrorNone) {
        if(furi_hal_nfc_field_detect_start() == FuriHalNfcErrorNone) {
            furi_delay_ms(50);
            nfc_present = furi_hal_nfc_field_is_present();
            furi_hal_nfc_field_detect_stop();
            nfc_ok = true;
        }
        furi_hal_nfc_release();
    }

    float ble_rssi = 0.0f;
    bool ble_ok = false;
    if(!furi_hal_bt_is_active()) {
        furi_hal_bt_start_rx(37);
        furi_delay_ms(50);
        ble_rssi = furi_hal_bt_get_rssi();
        furi_hal_bt_stop_rx();
        ble_ok = true;
    }

    furi_hal_subghz_reset();
    furi_hal_subghz_idle();
    furi_hal_subghz_set_path(FuriHalSubGhzPath433);
    furi_hal_subghz_set_frequency_and_path(HONE_SUBGHZ_FREQ);
    furi_hal_subghz_rx();
    furi_delay_ms(50);
    float subghz_rssi = furi_hal_subghz_get_rssi();
    furi_hal_subghz_idle();

    char summary[192] = {0};
    char part[48];

    snprintf(part, sizeof(part), "bat=%u", battery_pct);
    append_part(summary, sizeof(summary), part);
    snprintf(part, sizeof(part), "v=%.2f", (double)battery_voltage);
    append_part(summary, sizeof(summary), part);
    if(adc) {
        snprintf(part, sizeof(part), "cpu=%.1f", (double)cpu_temp);
        append_part(summary, sizeof(summary), part);
        snprintf(part, sizeof(part), "gpio=%.0f", (double)gpio_mv);
        append_part(summary, sizeof(summary), part);
    } else {
        append_part(summary, sizeof(summary), "adc=fail");
    }
    if(nfc_ok) {
        append_part(summary, sizeof(summary), nfc_present ? "nfc=1" : "nfc=0");
    } else {
        append_part(summary, sizeof(summary), "nfc=fail");
    }
    if(ble_ok) {
        snprintf(part, sizeof(part), "ble=%.0f", (double)ble_rssi);
        append_part(summary, sizeof(summary), part);
    } else {
        append_part(summary, sizeof(summary), "ble=busy");
    }
    snprintf(part, sizeof(part), "sg=%.0f", (double)subghz_rssi);
    append_part(summary, sizeof(summary), part);

    show_result(app, "Diagnostics Saved", summary);
    broadcast_beacon(app, "diag", summary);
}

static bool broadcast_beacon(HoneSensorApp* app, const char* source, const char* reading) {
    if(!app) return false;

    const SubGhzDevice* device = subghz_devices_get_by_name(HONE_SUBGHZ_DEVICE);
    if(!device) return false;
    if(!subghz_devices_begin(device)) return false;

    uint8_t preset_data[64] = {0};
    subghz_devices_load_preset(
        device,
        FuriHalSubGhzPresetGFSK9_99KbAsync,
        preset_data);
    subghz_devices_set_frequency(device, HONE_SUBGHZ_FREQ);
    if(!subghz_devices_set_tx(device)) {
        subghz_devices_idle(device);
        subghz_devices_end(device);
        return false;
    }

    char packet[256];
    if(build_signed_packet(app, source, reading, packet, sizeof(packet))) {
        size_t packet_len = strlen(packet);
        if(packet_len > 0 && packet_len < 255) {
            subghz_devices_write_packet(device, (const uint8_t*)packet, (uint8_t)packet_len);
        }
    }
    subghz_devices_idle(device);
    subghz_devices_end(device);
    return true;
}

static void probe_battery(HoneSensorApp* app) {
    uint8_t pct = furi_hal_power_get_pct();
    float voltage = furi_hal_power_get_battery_voltage(FuriHalPowerICFuelGauge);
    char body[64];
    snprintf(body, sizeof(body), "battery=%u%%\nvoltage=%.3f V", pct, (double)voltage);
    show_result(app, "Battery Probe", body);
    broadcast_beacon(app, "battery", body);
}

static void probe_cpu_temp(HoneSensorApp* app) {
    FuriHalAdcHandle* adc = furi_hal_adc_acquire();
    if(!adc) {
        show_result(app, "CPU Temp Probe", "Failed to acquire ADC.");
        return;
    }

    furi_hal_adc_configure(adc);
    uint16_t raw = furi_hal_adc_read(adc, FuriHalAdcChannelTEMPSENSOR);
    float temp = furi_hal_adc_convert_temp(adc, raw);
    furi_hal_adc_release(adc);

    char body[64];
    snprintf(body, sizeof(body), "raw=%u\ntemp=%.1f C", raw, (double)temp);
    show_result(app, "CPU Temp Probe", body);
    broadcast_beacon(app, "cpu_temp", body);
}

static void probe_gpio(HoneSensorApp* app) {
    furi_hal_gpio_init_simple(&gpio_ext_pa4, GpioModeAnalog);
    FuriHalAdcHandle* adc = furi_hal_adc_acquire();
    if(!adc) {
        show_result(app, "GPIO Probe", "Failed to acquire ADC.");
        return;
    }

    furi_hal_adc_configure(adc);
    uint16_t raw = furi_hal_adc_read(adc, FuriHalAdcChannel4);
    float mv = furi_hal_adc_convert_to_voltage(adc, raw);
    furi_hal_adc_release(adc);

    char body[64];
    snprintf(body, sizeof(body), "raw=%u\npa4=%.1f mV", raw, (double)mv);
    show_result(app, "GPIO Probe", body);
    broadcast_beacon(app, "gpio", body);
}

static void probe_nfc(HoneSensorApp* app) {
    if(furi_hal_nfc_acquire() != FuriHalNfcErrorNone) {
        show_result(app, "NFC Probe", "Failed to acquire NFC.");
        return;
    }

    char body[64];
    if(furi_hal_nfc_field_detect_start() == FuriHalNfcErrorNone) {
        furi_delay_ms(100);
        bool present = furi_hal_nfc_field_is_present();
        furi_hal_nfc_field_detect_stop();
        snprintf(body, sizeof(body), "field_present=%s", present ? "true" : "false");
    } else {
        snprintf(body, sizeof(body), "Failed to start field detect.");
    }
    furi_hal_nfc_release();
    show_result(app, "NFC Probe", body);
    broadcast_beacon(app, "nfc", body);
}

static void probe_ble(HoneSensorApp* app) {
    if(furi_hal_bt_is_active()) {
        show_result(app, "BLE Probe", "Bluetooth stack is already active.");
        return;
    }

    furi_hal_bt_start_rx(37);
    furi_delay_ms(150);
    float rssi = furi_hal_bt_get_rssi();
    furi_hal_bt_stop_rx();

    char body[64];
    snprintf(body, sizeof(body), "channel=37\nrssi=%.1f dBm", (double)rssi);
    show_result(app, "BLE Probe", body);
    broadcast_beacon(app, "ble", body);
}

static void probe_subghz(HoneSensorApp* app) {
    furi_hal_subghz_reset();
    furi_hal_subghz_idle();
    furi_hal_subghz_set_path(FuriHalSubGhzPath433);
    furi_hal_subghz_set_frequency_and_path(433920000UL);
    furi_hal_subghz_rx();
    furi_delay_ms(150);
    float rssi = furi_hal_subghz_get_rssi();
    furi_hal_subghz_idle();

    char body[64];
    snprintf(body, sizeof(body), "freq=433920000\nrssi=%.1f dBm", (double)rssi);
    show_result(app, "SubGHz Probe", body);
    broadcast_beacon(app, "subghz", body);
}

static void cycle_tick(void* context) {
    HoneSensorApp* app = context;
    if(!app || !app->cycle_active) return;
    probe_cycle_next(app);
}

static void probe_cycle_next(HoneSensorApp* app) {
    if(!app) return;
    size_t count = sizeof(hone_cycle_order) / sizeof(hone_cycle_order[0]);
    if(count == 0) return;
    if(app->cycle_index >= count) app->cycle_index = 0;
    uint32_t probe_id = hone_cycle_order[app->cycle_index];
    app->cycle_index = (app->cycle_index + 1) % count;
    switch(probe_id) {
    case HONE_MENU_BATTERY:
        probe_battery(app);
        break;
    case HONE_MENU_CPU_TEMP:
        probe_cpu_temp(app);
        break;
    case HONE_MENU_GPIO:
        probe_gpio(app);
        break;
    case HONE_MENU_NFC:
        probe_nfc(app);
        break;
    case HONE_MENU_BLE:
        probe_ble(app);
        break;
    case HONE_MENU_SUBGHZ:
        probe_subghz(app);
        break;
    default:
        break;
    }
}

static void submenu_callback(void* context, uint32_t index) {
    HoneSensorApp* app = context;
    if(!app) return;
    switch(index) {
    case HONE_MENU_BATTERY:
        probe_battery(app);
        break;
    case HONE_MENU_CPU_TEMP:
        probe_cpu_temp(app);
        break;
    case HONE_MENU_GPIO:
        probe_gpio(app);
        break;
    case HONE_MENU_NFC:
        probe_nfc(app);
        break;
    case HONE_MENU_BLE:
        probe_ble(app);
        break;
    case HONE_MENU_SUBGHZ:
        probe_subghz(app);
        break;
    case HONE_MENU_CYCLE:
        app->cycle_active = true;
        app->cycle_index = 0;
        show_result(
            app,
            "Sensor Cycle",
            "Automatic cycling enabled.\n\nBack = stop cycle and return to menu.");
        probe_cycle_next(app);
        break;
    case HONE_MENU_DIAGNOSTICS:
        show_diagnostics(app);
        break;
    case HONE_MENU_DIAGNOSTICS_SEND:
        send_diagnostics(app);
        break;
    case HONE_MENU_PAIR:
        show_pair_token(app, "Pair Phone", "This token binds a specific phone to this Flipper session.");
        break;
    case HONE_MENU_UNPAIR:
        refresh_pair_id(app);
        show_pair_token(app, "Unpair Phone", "Pairing rotated. The previous phone binding is no longer valid.");
        break;
    default:
        break;
    }
}

static HoneSensorApp* app_alloc(void) {
    HoneSensorApp* app = malloc(sizeof(HoneSensorApp));
    if(!app) return NULL;
    memset(app, 0, sizeof(*app));

    app->gui = furi_record_open(RECORD_GUI);
    app->storage = furi_record_open(RECORD_STORAGE);
    app->view_dispatcher = view_dispatcher_alloc();
    app->submenu = submenu_alloc();
    app->diagnostics_submenu = submenu_alloc();
    app->text_box = text_box_alloc();
    app->cycle_active = false;
    app->cycle_index = 0;
    app->pair_id = furi_hal_random_get();
    app->beacon_seq = 1;
    if(app->pair_id == 0) app->pair_id = 0xB7C0C001u;
    snprintf(app->pair_token, sizeof(app->pair_token), "HONE:PAIR:%08lX", (unsigned long)app->pair_id);

    if(!app->gui || !app->view_dispatcher || !app->submenu || !app->diagnostics_submenu || !app->text_box) {
        return app;
    }
    init_device_identity(app);

    snprintf(
        app->text,
        sizeof(app->text),
        "HONE Sensor\n\nDev: %s\nPair ID: %08lX\n\nBack = exit",
        app->device_id,
        (unsigned long)app->pair_id);

    submenu_set_header(app->submenu, "HONE Sensor");
    submenu_add_item(app->submenu, "Pair Phone", HONE_MENU_PAIR, submenu_callback, app);
    submenu_add_item(app->submenu, "Unpair Phone", HONE_MENU_UNPAIR, submenu_callback, app);
    submenu_add_item(app->submenu, "Start Sensor Cycle", HONE_MENU_CYCLE, submenu_callback, app);
    submenu_add_item(app->submenu, "Diagnostics", HONE_MENU_DIAGNOSTICS, submenu_callback, app);

    submenu_set_header(app->diagnostics_submenu, "Diagnostics");
    submenu_add_item(app->diagnostics_submenu, "Save/Send Diagnostics", HONE_MENU_DIAGNOSTICS_SEND, submenu_callback, app);
    submenu_add_item(app->diagnostics_submenu, "Probe Battery", HONE_MENU_BATTERY, submenu_callback, app);
    submenu_add_item(app->diagnostics_submenu, "Probe CPU Temp", HONE_MENU_CPU_TEMP, submenu_callback, app);
    submenu_add_item(app->diagnostics_submenu, "Probe GPIO", HONE_MENU_GPIO, submenu_callback, app);
    submenu_add_item(app->diagnostics_submenu, "Probe NFC", HONE_MENU_NFC, submenu_callback, app);
    submenu_add_item(app->diagnostics_submenu, "Probe BLE", HONE_MENU_BLE, submenu_callback, app);
    submenu_add_item(app->diagnostics_submenu, "Probe SubGHz", HONE_MENU_SUBGHZ, submenu_callback, app);

    text_box_set_font(app->text_box, TextBoxFontText);
    text_box_set_focus(app->text_box, TextBoxFocusStart);
    text_box_set_text(app->text_box, app->text);

    app->pair_view = view_alloc();
    if(app->pair_view) {
        view_allocate_model(app->pair_view, ViewModelTypeLockFree, sizeof(HonePairViewModel));
        view_set_draw_callback(app->pair_view, pair_view_draw);
        view_set_context(app->pair_view, app);
        update_pair_view_model(app, "Pair Phone");
    }

    view_dispatcher_add_view(app->view_dispatcher, HONE_VIEW_MENU, submenu_get_view(app->submenu));
    view_dispatcher_add_view(app->view_dispatcher, HONE_VIEW_RESULT, text_box_get_view(app->text_box));
    if(app->pair_view) {
        view_dispatcher_add_view(app->view_dispatcher, HONE_VIEW_PAIR, app->pair_view);
    }
    view_dispatcher_add_view(
        app->view_dispatcher,
        HONE_VIEW_DIAGNOSTICS,
        submenu_get_view(app->diagnostics_submenu));
    view_dispatcher_set_event_callback_context(app->view_dispatcher, app);
    view_dispatcher_set_navigation_event_callback(app->view_dispatcher, navigation_callback);
    view_dispatcher_set_tick_event_callback(app->view_dispatcher, cycle_tick, 3000);
    view_dispatcher_attach_to_gui(app->view_dispatcher, app->gui, ViewDispatcherTypeFullscreen);
    app->active_view = HONE_VIEW_MENU;
    view_dispatcher_switch_to_view(app->view_dispatcher, HONE_VIEW_MENU);
    return app;
}

static void app_free(HoneSensorApp* app) {
    if(!app) return;
    if(app->view_dispatcher) {
        view_dispatcher_set_tick_event_callback(app->view_dispatcher, NULL, 0);
        view_dispatcher_remove_view(app->view_dispatcher, HONE_VIEW_MENU);
        view_dispatcher_remove_view(app->view_dispatcher, HONE_VIEW_RESULT);
        if(app->pair_view) {
            view_dispatcher_remove_view(app->view_dispatcher, HONE_VIEW_PAIR);
        }
        view_dispatcher_remove_view(app->view_dispatcher, HONE_VIEW_DIAGNOSTICS);
        view_dispatcher_free(app->view_dispatcher);
    }
    if(app->submenu) submenu_free(app->submenu);
    if(app->diagnostics_submenu) submenu_free(app->diagnostics_submenu);
    if(app->text_box) text_box_free(app->text_box);
    if(app->pair_view) {
        view_free_model(app->pair_view);
        view_free(app->pair_view);
    }
    if(app->gui) furi_record_close(RECORD_GUI);
    if(app->storage) furi_record_close(RECORD_STORAGE);
    free(app);
}

int32_t hone_sensor_app(void* p) {
    (void)p;
    HoneSensorApp* app = app_alloc();
    if(!app || !app->gui || !app->view_dispatcher || !app->submenu || !app->diagnostics_submenu || !app->text_box) {
        app_free(app);
        return 1;
    }

    view_dispatcher_run(app->view_dispatcher);
    app_free(app);
    return 0;
}
