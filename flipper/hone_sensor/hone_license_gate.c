/*
 * UFL 3.4 Decentralized acceptance gate (UFL Section 9) for HONE Flipper apps.
 * Identical copies live in every HONE Flipper app directory; keep them in sync.
 */
#include "hone_license_gate.h"

#include <furi.h>
#include <furi_hal_rtc.h>
#include <storage/storage.h>
#include <dialogs/dialogs.h>
#include <stdio.h>
#include <string.h>

#define HONE_LICENSE_DIR    EXT_PATH("apps_data/hone")
#define HONE_LICENSE_RECORD EXT_PATH("apps_data/hone/license-acceptance.json")

typedef struct {
    const char* header;
    const char* text;
    const char* right;
} HoneLicensePage;

/* The screen fits about three short lines per page. Full text: LICENSE in the
 * HONE repository, as page 2 says. */
static const HoneLicensePage pages[] = {
    {"HONE license 1/4", "UFL 3.4, Operational\nScope: Decentralized.\nFree to use.", "Next"},
    {"HONE license 2/4", "Full text: LICENSE at\ngithub.com/shindevlin/\nhone", "Next"},
    {"HONE license 3/4", "Disputes: online ICC\narbitration, California\nseat, individual only.", "Next"},
    {"Accept license? 4/4", "Liability cap: US$1 in\nvalue (money or HONE).\nForks/bypass: own risk.", "Accept"},
};

static bool hone_license_record_matches(Storage* storage) {
    bool ok = false;
    File* file = storage_file_alloc(storage);
    if(storage_file_open(file, HONE_LICENSE_RECORD, FSAM_READ, FSOM_OPEN_EXISTING)) {
        char buf[256];
        size_t n = storage_file_read(file, buf, sizeof(buf) - 1);
        buf[n] = '\0';
        ok = strstr(buf, "\"license\": \"UFL\"") && strstr(buf, "\"version\": \"3.4\"") &&
             strstr(buf, "\"scope\": \"Decentralized\"");
    }
    storage_file_close(file);
    storage_file_free(file);
    return ok;
}

static void hone_license_record_write(Storage* storage, const char* app_name) {
    /* The Flipper RTC keeps device-local time with no time zone, so the time is
     * recorded as the device clock shows it (no trailing Z). */
    DateTime dt;
    furi_hal_rtc_get_datetime(&dt);
    char buf[256];
    int len = snprintf(
        buf,
        sizeof(buf),
        "{\n  \"license\": \"UFL\",\n  \"version\": \"3.4\",\n  \"scope\": \"Decentralized\",\n"
        "  \"accepted_at\": \"%04u-%02u-%02uT%02u:%02u:%02u\",\n  \"binary\": \"%s\"\n}\n",
        (unsigned)dt.year,
        (unsigned)dt.month,
        (unsigned)dt.day,
        (unsigned)dt.hour,
        (unsigned)dt.minute,
        (unsigned)dt.second,
        app_name);
    if(len <= 0 || (size_t)len >= sizeof(buf)) return;

    storage_simply_mkdir(storage, EXT_PATH("apps_data"));
    storage_simply_mkdir(storage, HONE_LICENSE_DIR);
    File* file = storage_file_alloc(storage);
    if(storage_file_open(file, HONE_LICENSE_RECORD, FSAM_WRITE, FSOM_CREATE_ALWAYS)) {
        storage_file_write(file, buf, (size_t)len);
    }
    /* If the write fails, acceptance stands for this run; the user is asked
     * again next launch. */
    storage_file_close(file);
    storage_file_free(file);
}

bool hone_license_gate(const char* app_name) {
    Storage* storage = furi_record_open(RECORD_STORAGE);
    bool accepted = hone_license_record_matches(storage);

    if(!accepted) {
        DialogsApp* dialogs = furi_record_open(RECORD_DIALOGS);
        DialogMessage* msg = dialog_message_alloc();
        const size_t count = sizeof(pages) / sizeof(pages[0]);
        bool declined = false;
        for(size_t i = 0; i < count && !declined; i++) {
            dialog_message_set_header(msg, pages[i].header, 64, 0, AlignCenter, AlignTop);
            dialog_message_set_text(msg, pages[i].text, 64, 32, AlignCenter, AlignCenter);
            dialog_message_set_buttons(msg, "Decline", NULL, pages[i].right);
            DialogMessageButton button = dialog_message_show(dialogs, msg);
            if(button != DialogMessageButtonRight) declined = true;
        }
        dialog_message_free(msg);
        furi_record_close(RECORD_DIALOGS);

        if(!declined) {
            hone_license_record_write(storage, app_name);
            accepted = true;
        }
    }

    furi_record_close(RECORD_STORAGE);
    return accepted;
}
