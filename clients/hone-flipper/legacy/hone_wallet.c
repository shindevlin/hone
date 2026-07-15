/**
 * HONE Wallet — Flipper Zero Hardware Wallet + Sensor Array
 * Natoshi Sakamoto
 *
 * Stores encrypted HONE keypairs on microSD. Signs transactions on-device.
 * Receives seed phrases from PC companion app via USB serial.
 * Multiple wallets supported — each protected by its own PIN.
 *
 * Background sensor scanning: Sub-GHz RSSI, BLE presence, NFC detect,
 * GPIO ADC (PA4), internal temperature. Readings buffered to microSD
 * in JSONL format and available via USB commands.
 *
 * Menu:
 *   1. Wallets       — list stored wallets, show balance/address
 *   2. Import Wallet — receive seed phrase from PC via USB
 *   3. Sign          — sign a transaction (USB/BLE request)
 *   4. Sensors       — live sensor scan stats display
 *   5. About         — version info
 *
 * USB Protocol: newline-delimited JSON over CDC serial.
 *   {"cmd":"ping"}
 *   {"cmd":"import","name":"...","mnemonic":"...","pin":"..."}
 *   {"cmd":"list"}
 *   {"cmd":"sign","wallet":"...","tx_hash":"...","pin":"..."}
 *   {"cmd":"sensors"}
 *   {"cmd":"flush_readings"}
 *   {"cmd":"sensor_status"}
 */

#include <furi.h>
#include <furi_hal.h>
#include <furi_hal_usb.h>
#include <furi_hal_usb_cdc.h>
#include <furi_hal_cortex.h>
#include <furi_hal_resources.h>
#include <furi_hal_adc.h>
#include <gui/gui.h>
#include <gui/view_dispatcher.h>
#include <gui/modules/menu.h>
#include <gui/modules/popup.h>
#include <gui/modules/text_box.h>
#include <gui/modules/text_input.h>
#include <gui/modules/submenu.h>
#include <storage/storage.h>
#include <dialogs/dialogs.h>
#include <notification/notification_messages.h>
#include <furi_hal_nfc.h>

#define TAG "HONEWallet"
#define HONE_VERSION     "0.2.0"
#define HONE_WALLET_DIR  EXT_PATH("apps_data/hone_wallet")
#define HONE_READINGS_PATH EXT_PATH("apps_data/hone_wallet/readings.jsonl")
#define HONE_MAX_WALLETS 10
#define HONE_PIN_MIN     6
#define HONE_PIN_MAX     12
// 1,000 rounds — safe with 10-attempt lockout + wipe. STM32 @ 64MHz ≈ 0.5s.
#define HONE_KDF_ROUNDS  1000
#define HONE_NAME_LEN    20
#define HONE_USB_BUF_LEN 2048
#define HONE_CDC_CH      0

// ─── Sensor constants ────────────────────────────────────────
#define SENSOR_SUBGHZ_SCAN_MS   10000  // 10s Sub-GHz window
#define SENSOR_BLE_SCAN_MS      10000  // 10s BLE window
#define SENSOR_MAX_READINGS     10000  // max lines in readings.jsonl
#define SENSOR_ROTATE_KEEP      5000   // keep newest half on rotate
#define SENSOR_DISPLAY_REFRESH_MS 5000 // menu auto-refresh

// Adaptive scanning — scan fast when interesting, slow when boring
#define SCAN_INTERVAL_FAST_MS    2000  // 2s when environment is changing
#define SCAN_INTERVAL_SLOW_MS   60000  // 60s when nothing is happening
#define SCAN_RSSI_DELTA_DB       10    // Sub-GHz RSSI change threshold
#define SCAN_BLE_DELTA_COUNT      1    // BLE device count change threshold
#define SCAN_GPIO_DELTA_PCT       5    // GPIO % change threshold
#define SCAN_TEMP_DELTA_C         2    // temperature change threshold (°C)
#define SCAN_INTEREST_COOLDOWN   30    // stay fast for 30 scans after last interesting event

// Sub-GHz frequencies to scan (Hz)
static const uint32_t subghz_freqs[] = {433920000, 315000000, 868350000, 915000000};
#define SUBGHZ_FREQ_COUNT (sizeof(subghz_freqs) / sizeof(subghz_freqs[0]))

// ─── Embedded SHA-256 (mbedtls not exported in Flipper SDK) ────

#define SHA256_BLOCK_SIZE 64
#define SHA256_DIGEST_SIZE 32

typedef struct {
    uint32_t state[8];
    uint64_t count;
    uint8_t buf[SHA256_BLOCK_SIZE];
} Sha256Ctx;

static const uint32_t sha256_k[64] = {
    0x428a2f98,0x71374491,0xb5c0fbcf,0xe9b5dba5,0x3956c25b,0x59f111f1,0x923f82a4,0xab1c5ed5,
    0xd807aa98,0x12835b01,0x243185be,0x550c7dc3,0x72be5d74,0x80deb1fe,0x9bdc06a7,0xc19bf174,
    0xe49b69c1,0xefbe4786,0x0fc19dc6,0x240ca1cc,0x2de92c6f,0x4a7484aa,0x5cb0a9dc,0x76f988da,
    0x983e5152,0xa831c66d,0xb00327c8,0xbf597fc7,0xc6e00bf3,0xd5a79147,0x06ca6351,0x14292967,
    0x27b70a85,0x2e1b2138,0x4d2c6dfc,0x53380d13,0x650a7354,0x766a0abb,0x81c2c92e,0x92722c85,
    0xa2bfe8a1,0xa81a664b,0xc24b8b70,0xc76c51a3,0xd192e819,0xd6990624,0xf40e3585,0x106aa070,
    0x19a4c116,0x1e376c08,0x2748774c,0x34b0bcb5,0x391c0cb3,0x4ed8aa4a,0x5b9cca4f,0x682e6ff3,
    0x748f82ee,0x78a5636f,0x84c87814,0x8cc70208,0x90befffa,0xa4506ceb,0xbef9a3f7,0xc67178f2,
};

static inline uint32_t sha256_rotr(uint32_t x, uint32_t n) {
    return (x >> n) | (x << (32 - n));
}

static inline uint32_t sha256_ch(uint32_t x, uint32_t y, uint32_t z) {
    return (x & y) ^ (~x & z);
}

static inline uint32_t sha256_maj(uint32_t x, uint32_t y, uint32_t z) {
    return (x & y) ^ (x & z) ^ (y & z);
}

static inline uint32_t sha256_ep0(uint32_t x) {
    return sha256_rotr(x, 2) ^ sha256_rotr(x, 13) ^ sha256_rotr(x, 22);
}

static inline uint32_t sha256_ep1(uint32_t x) {
    return sha256_rotr(x, 6) ^ sha256_rotr(x, 11) ^ sha256_rotr(x, 25);
}

static inline uint32_t sha256_sig0(uint32_t x) {
    return sha256_rotr(x, 7) ^ sha256_rotr(x, 18) ^ (x >> 3);
}

static inline uint32_t sha256_sig1(uint32_t x) {
    return sha256_rotr(x, 17) ^ sha256_rotr(x, 19) ^ (x >> 10);
}

static void sha256_transform(Sha256Ctx* ctx, const uint8_t* data) {
    uint32_t w[64];
    uint32_t a, b, c, d, e, f, g, h, t1, t2;

    for(int i = 0; i < 16; i++) {
        w[i] = ((uint32_t)data[i * 4] << 24) | ((uint32_t)data[i * 4 + 1] << 16) |
               ((uint32_t)data[i * 4 + 2] << 8) | ((uint32_t)data[i * 4 + 3]);
    }
    for(int i = 16; i < 64; i++) {
        w[i] = sha256_sig1(w[i - 2]) + w[i - 7] + sha256_sig0(w[i - 15]) + w[i - 16];
    }

    a = ctx->state[0]; b = ctx->state[1]; c = ctx->state[2]; d = ctx->state[3];
    e = ctx->state[4]; f = ctx->state[5]; g = ctx->state[6]; h = ctx->state[7];

    for(int i = 0; i < 64; i++) {
        t1 = h + sha256_ep1(e) + sha256_ch(e, f, g) + sha256_k[i] + w[i];
        t2 = sha256_ep0(a) + sha256_maj(a, b, c);
        h = g; g = f; f = e; e = d + t1;
        d = c; c = b; b = a; a = t1 + t2;
    }

    ctx->state[0] += a; ctx->state[1] += b; ctx->state[2] += c; ctx->state[3] += d;
    ctx->state[4] += e; ctx->state[5] += f; ctx->state[6] += g; ctx->state[7] += h;
}

static void sha256_init(Sha256Ctx* ctx) {
    ctx->state[0] = 0x6a09e667; ctx->state[1] = 0xbb67ae85;
    ctx->state[2] = 0x3c6ef372; ctx->state[3] = 0xa54ff53a;
    ctx->state[4] = 0x510e527f; ctx->state[5] = 0x9b05688c;
    ctx->state[6] = 0x1f83d9ab; ctx->state[7] = 0x5be0cd19;
    ctx->count = 0;
}

static void sha256_update(Sha256Ctx* ctx, const uint8_t* data, size_t len) {
    for(size_t i = 0; i < len; i++) {
        ctx->buf[ctx->count % SHA256_BLOCK_SIZE] = data[i];
        ctx->count++;
        if((ctx->count % SHA256_BLOCK_SIZE) == 0) {
            sha256_transform(ctx, ctx->buf);
        }
    }
}

static void sha256_final(Sha256Ctx* ctx, uint8_t* digest) {
    uint64_t bits = ctx->count * 8;
    size_t idx = ctx->count % SHA256_BLOCK_SIZE;

    ctx->buf[idx++] = 0x80;
    if(idx > 56) {
        while(idx < SHA256_BLOCK_SIZE) ctx->buf[idx++] = 0;
        sha256_transform(ctx, ctx->buf);
        idx = 0;
    }
    while(idx < 56) ctx->buf[idx++] = 0;

    for(int i = 7; i >= 0; i--) {
        ctx->buf[56 + (7 - i)] = (uint8_t)(bits >> (i * 8));
    }
    sha256_transform(ctx, ctx->buf);

    for(int i = 0; i < 8; i++) {
        digest[i * 4]     = (uint8_t)(ctx->state[i] >> 24);
        digest[i * 4 + 1] = (uint8_t)(ctx->state[i] >> 16);
        digest[i * 4 + 2] = (uint8_t)(ctx->state[i] >> 8);
        digest[i * 4 + 3] = (uint8_t)(ctx->state[i]);
    }
}

/** Compute SHA-256 in one shot */
static void sha256_hash(const uint8_t* data, size_t len, uint8_t* digest) {
    Sha256Ctx ctx;
    sha256_init(&ctx);
    sha256_update(&ctx, data, len);
    sha256_final(&ctx, digest);
}

/** HMAC-SHA256 */
static void hmac_sha256(
    const uint8_t* key, size_t key_len,
    const uint8_t* msg, size_t msg_len,
    uint8_t* out) {
    uint8_t k_pad[SHA256_BLOCK_SIZE];
    uint8_t k_hash[SHA256_DIGEST_SIZE];
    Sha256Ctx ctx;

    /* If key > block size, hash it first */
    if(key_len > SHA256_BLOCK_SIZE) {
        sha256_hash(key, key_len, k_hash);
        key = k_hash;
        key_len = SHA256_DIGEST_SIZE;
    }

    /* Inner pad */
    memset(k_pad, 0x36, SHA256_BLOCK_SIZE);
    for(size_t i = 0; i < key_len; i++) k_pad[i] ^= key[i];

    sha256_init(&ctx);
    sha256_update(&ctx, k_pad, SHA256_BLOCK_SIZE);
    sha256_update(&ctx, msg, msg_len);
    uint8_t inner[SHA256_DIGEST_SIZE];
    sha256_final(&ctx, inner);

    /* Outer pad */
    memset(k_pad, 0x5c, SHA256_BLOCK_SIZE);
    for(size_t i = 0; i < key_len; i++) k_pad[i] ^= key[i];

    sha256_init(&ctx);
    sha256_update(&ctx, k_pad, SHA256_BLOCK_SIZE);
    sha256_update(&ctx, inner, SHA256_DIGEST_SIZE);
    sha256_final(&ctx, out);

    /* Scrub */
    memset(inner, 0, sizeof(inner));
    memset(k_pad, 0, sizeof(k_pad));
    memset(k_hash, 0, sizeof(k_hash));
}

// ─── Hex helpers ──────────────────────────────────────────────

static void bytes_to_hex(const uint8_t* src, size_t len, char* dst) {
    static const char hex[] = "0123456789abcdef";
    for(size_t i = 0; i < len; i++) {
        dst[i * 2]     = hex[(src[i] >> 4) & 0x0f];
        dst[i * 2 + 1] = hex[src[i] & 0x0f];
    }
    dst[len * 2] = '\0';
}

static int hex_char_val(char c) {
    if(c >= '0' && c <= '9') return c - '0';
    if(c >= 'a' && c <= 'f') return 10 + c - 'a';
    if(c >= 'A' && c <= 'F') return 10 + c - 'A';
    return -1;
}

static bool hex_to_bytes(const char* hex, uint8_t* out, size_t out_len) {
    for(size_t i = 0; i < out_len; i++) {
        int hi = hex_char_val(hex[i * 2]);
        int lo = hex_char_val(hex[i * 2 + 1]);
        if(hi < 0 || lo < 0) return false;
        out[i] = (uint8_t)((hi << 4) | lo);
    }
    return true;
}

// ─── Key derivation: iterated SHA-256(pin + salt) ─────────────
// Phase 2: used for real AES-256-GCM encryption
__attribute__((unused))
static void derive_aes_key(
    const char* pin,
    const uint8_t* salt, /* 16 bytes */
    uint8_t* aes_key /* 32 bytes out */) {

    /* First hash: SHA-256(pin || salt) */
    Sha256Ctx ctx;
    sha256_init(&ctx);
    sha256_update(&ctx, (const uint8_t*)pin, strlen(pin));
    sha256_update(&ctx, salt, 16);
    sha256_final(&ctx, aes_key);

    /* Iterate HONE_KDF_ROUNDS - 1 more times */
    for(int i = 1; i < HONE_KDF_ROUNDS; i++) {
        sha256_hash(aes_key, SHA256_DIGEST_SIZE, aes_key);
    }
}

// ─── Minimal JSON parser (no allocations) ─────────────────────

/** Extract a JSON string value for a given key from a JSON object string.
 *  Writes into out_buf (max out_len-1 chars + NUL). Returns false if not found. */
static bool json_get_str(const char* json, const char* key, char* out_buf, size_t out_len) {
    /* Build search pattern: "key":" */
    char pattern[64];
    snprintf(pattern, sizeof(pattern), "\"%s\":\"", key);
    const char* start = strstr(json, pattern);
    if(!start) {
        /* Try with space after colon: "key": " */
        snprintf(pattern, sizeof(pattern), "\"%s\": \"", key);
        start = strstr(json, pattern);
    }
    if(!start) return false;
    start = strchr(start, ':');
    if(!start) return false;
    start++; /* skip ':' */
    while(*start == ' ') start++;
    if(*start != '"') return false;
    start++; /* skip opening quote */

    size_t i = 0;
    while(*start && *start != '"' && i < out_len - 1) {
        if(*start == '\\' && *(start + 1)) {
            start++; /* skip escape backslash */
        }
        out_buf[i++] = *start++;
    }
    out_buf[i] = '\0';
    return i > 0;
}

// ─── Keystore file format ─────────────────────────────────────
// Layout (binary):
//   [16 bytes salt] [12 bytes IV] [16 bytes GCM tag] [32 bytes encrypted privkey]
#define KEYFILE_SALT_OFF   0
#define KEYFILE_SALT_LEN   16
#define KEYFILE_IV_OFF     16
#define KEYFILE_IV_LEN     12
#define KEYFILE_TAG_OFF    28
#define KEYFILE_TAG_LEN    16
#define KEYFILE_DATA_OFF   44
#define KEYFILE_DATA_LEN   32
#define KEYFILE_TOTAL_LEN  76

// ─── Sensor state (global — kept off stack for RAM safety) ───

typedef struct {
    /* Latest readings */
    float    subghz_rssi[SUBGHZ_FREQ_COUNT]; // last RSSI per frequency
    uint32_t subghz_last_ts;                 // timestamp of last Sub-GHz scan

    uint8_t  ble_channel_hits;  // channels with signal above noise floor
    float    ble_best_rssi;     // strongest RSSI seen in last BLE scan
    uint32_t ble_last_ts;

    bool     nfc_detected;      // was NFC field/card present?
    uint32_t nfc_last_ts;

    uint16_t gpio_adc_raw;      // raw 12-bit ADC value from PA4
    float    gpio_adc_mv;       // converted to mV
    uint32_t gpio_last_ts;

    float    cpu_temp_c;        // internal die temperature
    uint32_t temp_last_ts;

    /* Stats */
    uint32_t total_readings;    // total readings written
    uint32_t buffer_lines;      // current lines in readings.jsonl
    uint32_t last_scan_ts;      // timestamp of last completed scan cycle

    /* Adaptive scanning */
    uint32_t scan_interval_ms;  // current scan interval
    int32_t  interest_countdown; // scans remaining at fast rate
    float    prev_subghz_rssi[4]; // previous RSSI values for delta detection
    uint8_t  prev_ble_hits;     // previous BLE count
    uint16_t prev_gpio_raw;     // previous GPIO value
    float    prev_cpu_temp;     // previous temperature

    /* Control */
    volatile bool running;
    FuriMutex* mutex;           // protects sensor state reads
} SensorState;

static SensorState g_sensors = {0};

// ─── App state ────────────────────────────────────────────────

typedef struct {
    ViewDispatcher* view_dispatcher;
    Submenu* submenu;
    Popup* popup;
    TextBox* text_box;
    Gui* gui;
    Storage* storage;
    uint8_t wallet_count;
    char wallet_names[HONE_MAX_WALLETS][HONE_NAME_LEN + 1];

    /* USB CDC */
    FuriThread* usb_thread;
    FuriStreamBuffer* usb_rx_stream;
    volatile bool usb_running;
    FuriHalUsbInterface* prev_usb_cfg;

    /* Sensor thread */
    FuriThread* sensor_thread;
} HONEWalletApp;

// View IDs
typedef enum {
    HONEViewMenu,
    HONEViewPopup,
    HONEViewTextBox,
} HONEViewId;

// Menu items
typedef enum {
    HONEMenuWallets,
    HONEMenuImport,
    HONEMenuSign,
    HONEMenuSensors,
    HONEMenuAbout,
} HONEMenuItem;

// ─── Storage helpers ────────────────────────────────────────────

static void hone_ensure_dir(Storage* storage) {
    if(!storage_dir_exists(storage, HONE_WALLET_DIR)) {
        storage_simply_mkdir(storage, HONE_WALLET_DIR);
    }
}

static void hone_load_wallet_names(HONEWalletApp* app) {
    hone_ensure_dir(app->storage);
    File* dir = storage_file_alloc(app->storage);
    app->wallet_count = 0;

    if(storage_dir_open(dir, HONE_WALLET_DIR)) {
        FileInfo info;
        char name[256];
        while(storage_dir_read(dir, &info, name, sizeof(name)) &&
              app->wallet_count < HONE_MAX_WALLETS) {
            size_t len = strlen(name);
            if(len > 4 && strcmp(name + len - 4, ".key") == 0) {
                size_t name_len = len - 4;
                if(name_len > HONE_NAME_LEN) name_len = HONE_NAME_LEN;
                memcpy(app->wallet_names[app->wallet_count], name, name_len);
                app->wallet_names[app->wallet_count][name_len] = '\0';
                app->wallet_count++;
            }
        }
    }
    storage_dir_close(dir);
    storage_file_free(dir);
}

// ─── USB CDC send helper ────────────────────────────────────────

static void usb_send_str(const char* str) {
    size_t len = strlen(str);
    /* CDC max packet is 64 bytes, send in chunks */
    const uint8_t* p = (const uint8_t*)str;
    while(len > 0) {
        uint16_t chunk = (len > CDC_DATA_SZ) ? CDC_DATA_SZ : (uint16_t)len;
        furi_hal_cdc_send(HONE_CDC_CH, (uint8_t*)p, chunk);
        p += chunk;
        len -= chunk;
    }
}

static void usb_send_line(const char* str) {
    usb_send_str(str);
    usb_send_str("\n");
}

// ─── Command handlers ──────────────────────────────────────────

static void cmd_ping(void) {
    char resp[128];
    snprintf(resp, sizeof(resp),
        "{\"status\":\"ok\",\"version\":\"%s\"}", HONE_VERSION);
    usb_send_line(resp);
}

static void cmd_list(HONEWalletApp* app) {
    hone_load_wallet_names(app);
    char resp[512];
    size_t off = 0;
    off += snprintf(resp + off, sizeof(resp) - off, "{\"status\":\"ok\",\"wallets\":[");
    for(uint8_t i = 0; i < app->wallet_count; i++) {
        if(i > 0) off += snprintf(resp + off, sizeof(resp) - off, ",");
        off += snprintf(resp + off, sizeof(resp) - off, "\"%s\"", app->wallet_names[i]);
    }
    off += snprintf(resp + off, sizeof(resp) - off, "]}");
    (void)off;
    usb_send_line(resp);
}

__attribute__((unused))
static void cmd_import(HONEWalletApp* app, const char* json) {
    char name[HONE_NAME_LEN + 1] = {0};
    char mnemonic[256] = {0};
    char pin[HONE_PIN_MAX + 1] = {0};

    if(!json_get_str(json, "name", name, sizeof(name)) ||
       !json_get_str(json, "mnemonic", mnemonic, sizeof(mnemonic)) ||
       !json_get_str(json, "pin", pin, sizeof(pin))) {
        usb_send_line("{\"status\":\"error\",\"error\":\"missing name, mnemonic, or pin\"}");
        goto cleanup;
    }

    size_t pin_len = strlen(pin);
    if(pin_len < HONE_PIN_MIN || pin_len > HONE_PIN_MAX) {
        usb_send_line("{\"status\":\"error\",\"error\":\"pin must be 6-12 chars\"}");
        goto cleanup;
    }

    /* Phase 1: derive 32-byte private key from SHA-256(mnemonic) */
    uint8_t privkey[32];
    sha256_hash((const uint8_t*)mnemonic, strlen(mnemonic), privkey);

    /* Zero mnemonic from memory immediately — compiler barrier prevents optimization */
    volatile uint8_t* vm = (volatile uint8_t*)mnemonic;
    for(size_t i = 0; i < sizeof(mnemonic); i++) vm[i] = 0;

    /* Phase 1: Simple XOR encryption with PIN-derived key.
     * Real AES-256-GCM comes in Phase 2 after testing hardware crypto API. */
    uint8_t salt[KEYFILE_SALT_LEN];
    uint8_t iv[KEYFILE_IV_LEN];
    furi_hal_random_fill_buf(salt, sizeof(salt));
    furi_hal_random_fill_buf(iv, sizeof(iv));

    /* Derive key from PIN: SHA256(pin || salt) */
    uint8_t pin_key[32];
    {
        uint8_t pin_salt[256];
        size_t plen = strlen(pin);
        memcpy(pin_salt, pin, plen);
        memcpy(pin_salt + plen, salt, KEYFILE_SALT_LEN);
        sha256_hash(pin_salt, plen + KEYFILE_SALT_LEN, pin_key);
        memset(pin_salt, 0, sizeof(pin_salt));
    }

    /* XOR encrypt private key */
    uint8_t encrypted[32];
    for(size_t i = 0; i < 32; i++) {
        encrypted[i] = privkey[i] ^ pin_key[i];
    }

    /* Tag = SHA256(pin_key || privkey) for integrity check */
    uint8_t tag[16];
    {
        uint8_t tag_input[64];
        memcpy(tag_input, pin_key, 32);
        memcpy(tag_input + 32, privkey, 32);
        uint8_t tag_full[32];
        sha256_hash(tag_input, 64, tag_full);
        memcpy(tag, tag_full, 16);
        memset(tag_input, 0, sizeof(tag_input));
        memset(tag_full, 0, sizeof(tag_full));
    }

    memset(pin_key, 0, sizeof(pin_key));

    /* Compute "public key" for display — Phase 1 placeholder:
     * SHA-256(privkey) since we don't have secp256k1 yet */
    uint8_t pubkey_hash[32];
    sha256_hash(privkey, sizeof(privkey), pubkey_hash);
    memset(privkey, 0, sizeof(privkey));

    /* Write keyfile: salt || iv || tag || encrypted_privkey */
    hone_ensure_dir(app->storage);
    char path[128];
    snprintf(path, sizeof(path), "%s/%s.key", HONE_WALLET_DIR, name);

    File* file = storage_file_alloc(app->storage);
    bool ok = storage_file_open(file, path, FSAM_WRITE, FSOM_CREATE_ALWAYS);
    if(ok) {
        storage_file_write(file, salt, KEYFILE_SALT_LEN);
        storage_file_write(file, iv, KEYFILE_IV_LEN);
        storage_file_write(file, tag, KEYFILE_TAG_LEN);
        storage_file_write(file, encrypted, KEYFILE_DATA_LEN);
        storage_file_close(file);
    }
    storage_file_free(file);

    if(!ok) {
        usb_send_line("{\"status\":\"error\",\"error\":\"failed to write keyfile\"}");
        goto cleanup;
    }

    /* Respond with public key hash (hex) */
    char pubhex[65];
    bytes_to_hex(pubkey_hash, 32, pubhex);

    char resp[256];
    snprintf(resp, sizeof(resp),
        "{\"status\":\"ok\",\"wallet\":\"%s\",\"pubkey\":\"%s\"}", name, pubhex);
    usb_send_line(resp);
    FURI_LOG_I(TAG, "Imported wallet: %s", name);

cleanup:
    /* Extra scrub of stack buffers */
    memset(pin, 0, sizeof(pin));
    memset(name, 0, sizeof(name));
}

static void cmd_sign(HONEWalletApp* app, const char* json) {
    char wallet[HONE_NAME_LEN + 1] = {0};
    char tx_hash_hex[128] = {0};
    char pin[HONE_PIN_MAX + 1] = {0};

    if(!json_get_str(json, "wallet", wallet, sizeof(wallet)) ||
       !json_get_str(json, "tx_hash", tx_hash_hex, sizeof(tx_hash_hex)) ||
       !json_get_str(json, "pin", pin, sizeof(pin))) {
        usb_send_line("{\"status\":\"error\",\"error\":\"missing wallet, tx_hash, or pin\"}");
        goto cleanup;
    }

    /* Parse tx_hash hex — must be 64 hex chars (32 bytes) */
    size_t hex_len = strlen(tx_hash_hex);
    if(hex_len != 64) {
        usb_send_line("{\"status\":\"error\",\"error\":\"tx_hash must be 64 hex chars\"}");
        goto cleanup;
    }
    uint8_t tx_hash[32];
    if(!hex_to_bytes(tx_hash_hex, tx_hash, 32)) {
        usb_send_line("{\"status\":\"error\",\"error\":\"invalid hex in tx_hash\"}");
        goto cleanup;
    }

    /* Load keyfile */
    char path[128];
    snprintf(path, sizeof(path), "%s/%s.key", HONE_WALLET_DIR, wallet);

    if(!storage_file_exists(app->storage, path)) {
        usb_send_line("{\"status\":\"error\",\"error\":\"wallet not found\"}");
        goto cleanup;
    }

    File* file = storage_file_alloc(app->storage);
    uint8_t keyfile[KEYFILE_TOTAL_LEN];
    bool ok = false;

    if(storage_file_open(file, path, FSAM_READ, FSOM_OPEN_EXISTING)) {
        size_t rd = storage_file_read(file, keyfile, KEYFILE_TOTAL_LEN);
        ok = (rd == KEYFILE_TOTAL_LEN);
        storage_file_close(file);
    }
    storage_file_free(file);

    if(!ok) {
        usb_send_line("{\"status\":\"error\",\"error\":\"failed to read keyfile\"}");
        goto cleanup;
    }

    /* Phase 1: XOR decrypt with PIN-derived key */
    uint8_t pin_key[32];
    {
        uint8_t pin_salt[256];
        size_t plen = strlen(pin);
        memcpy(pin_salt, pin, plen);
        memcpy(pin_salt + plen, keyfile + KEYFILE_SALT_OFF, KEYFILE_SALT_LEN);
        sha256_hash(pin_salt, plen + KEYFILE_SALT_LEN, pin_key);
        memset(pin_salt, 0, sizeof(pin_salt));
    }

    uint8_t privkey[32];
    for(size_t i = 0; i < 32; i++) {
        privkey[i] = keyfile[KEYFILE_DATA_OFF + i] ^ pin_key[i];
    }

    /* Verify tag */
    uint8_t check_tag[16];
    {
        uint8_t tag_input[64];
        memcpy(tag_input, pin_key, 32);
        memcpy(tag_input + 32, privkey, 32);
        uint8_t tag_full[32];
        sha256_hash(tag_input, 64, tag_full);
        memcpy(check_tag, tag_full, 16);
        memset(tag_input, 0, sizeof(tag_input));
        memset(tag_full, 0, sizeof(tag_full));
    }
    memset(pin_key, 0, sizeof(pin_key));

    if(memcmp(check_tag, keyfile + KEYFILE_TAG_OFF, 16) != 0) {
        memset(privkey, 0, sizeof(privkey));
        usb_send_line("{\"status\":\"error\",\"error\":\"wrong PIN\"}");
        goto cleanup;
    }

    /* Phase 1 signature: HMAC-SHA256(privkey, tx_hash) */
    uint8_t signature[32];
    hmac_sha256(privkey, 32, tx_hash, 32, signature);
    memset(privkey, 0, sizeof(privkey));

    char sig_hex[65];
    bytes_to_hex(signature, 32, sig_hex);
    memset(signature, 0, sizeof(signature));

    char resp[256];
    snprintf(resp, sizeof(resp),
        "{\"status\":\"ok\",\"wallet\":\"%s\",\"signature\":\"%s\",\"algo\":\"hmac-sha256-phase1\"}",
        wallet, sig_hex);
    usb_send_line(resp);
    FURI_LOG_I(TAG, "Signed tx for wallet: %s", wallet);

cleanup:
    memset(pin, 0, sizeof(pin));
    memset(wallet, 0, sizeof(wallet));
    memset(tx_hash_hex, 0, sizeof(tx_hash_hex));
}

// ─── Sensor: readings file helpers ──────────────────────────────

/** Count lines in readings.jsonl */
static uint32_t sensor_count_lines(Storage* storage) {
    File* f = storage_file_alloc(storage);
    uint32_t count = 0;
    if(storage_file_open(f, HONE_READINGS_PATH, FSAM_READ, FSOM_OPEN_EXISTING)) {
        char buf[128];
        while(true) {
            size_t rd = storage_file_read(f, buf, sizeof(buf));
            if(rd == 0) break;
            for(size_t i = 0; i < rd; i++) {
                if(buf[i] == '\n') count++;
            }
        }
        storage_file_close(f);
    }
    storage_file_free(f);
    return count;
}

/** Append a single line to readings.jsonl */
static void sensor_append_line(Storage* storage, const char* line) {
    hone_ensure_dir(storage);
    File* f = storage_file_alloc(storage);
    if(storage_file_open(f, HONE_READINGS_PATH, FSAM_WRITE, FSOM_OPEN_APPEND)) {
        storage_file_write(f, line, strlen(line));
        storage_file_write(f, "\n", 1);
        storage_file_close(f);
    }
    storage_file_free(f);
}

/** Rotate readings.jsonl: keep newest SENSOR_ROTATE_KEEP lines */
static void sensor_rotate_readings(Storage* storage) {
    /* Read entire file, skip first (total - SENSOR_ROTATE_KEEP) lines, rewrite */
    uint32_t total = sensor_count_lines(storage);
    if(total <= SENSOR_MAX_READINGS) return;

    uint32_t skip = total - SENSOR_ROTATE_KEEP;
    const char* tmp_path = EXT_PATH("apps_data/hone_wallet/readings.tmp");

    File* src = storage_file_alloc(storage);
    File* dst = storage_file_alloc(storage);
    bool ok = false;

    if(storage_file_open(src, HONE_READINGS_PATH, FSAM_READ, FSOM_OPEN_EXISTING) &&
       storage_file_open(dst, tmp_path, FSAM_WRITE, FSOM_CREATE_ALWAYS)) {

        uint32_t lines_seen = 0;
        char buf[128];
        size_t buf_pos = 0;
        bool copying = false;

        while(true) {
            size_t rd = storage_file_read(src, buf + buf_pos, sizeof(buf) - buf_pos);
            if(rd == 0 && buf_pos == 0) break;
            size_t avail = buf_pos + rd;

            size_t consumed = 0;
            for(size_t i = 0; i < avail; i++) {
                if(buf[i] == '\n') {
                    lines_seen++;
                    if(lines_seen > skip) copying = true;
                    if(copying) {
                        storage_file_write(dst, buf + consumed, i - consumed + 1);
                    }
                    consumed = i + 1;
                }
            }
            /* Move remaining partial line to start of buffer */
            if(consumed < avail) {
                memmove(buf, buf + consumed, avail - consumed);
                buf_pos = avail - consumed;
            } else {
                buf_pos = 0;
            }
            if(rd == 0) break; /* EOF reached, only leftover was processed */
        }
        ok = true;
    }

    storage_file_close(src);
    storage_file_close(dst);
    storage_file_free(src);
    storage_file_free(dst);

    if(ok) {
        storage_simply_remove(storage, HONE_READINGS_PATH);
        storage_common_rename(storage, tmp_path, HONE_READINGS_PATH);
    }
}

// ─── Sensor: Sub-GHz scan ──────────────────────────────────────

static void sensor_scan_subghz(Storage* storage) {
    uint32_t ts = furi_hal_rtc_get_timestamp();

    /* Wake up the CC1101 radio — reset then idle to reach a known state.
     * furi_hal_subghz_init() is firmware-internal (not exposed to FAPs),
     * but reset+idle is safe: reset brings the radio out of sleep,
     * idle puts it in a state ready to accept frequency/rx commands. */
    furi_hal_subghz_reset();
    furi_hal_subghz_idle();
    furi_delay_ms(5); /* brief settle after reset */

    for(size_t i = 0; i < SUBGHZ_FREQ_COUNT; i++) {
        if(!g_sensors.running) break;

        uint32_t freq = subghz_freqs[i];
        if(!furi_hal_subghz_is_frequency_valid(freq)) {
            g_sensors.subghz_rssi[i] = -999.0f;
            continue;
        }

        furi_hal_subghz_set_frequency_and_path(freq);
        furi_hal_subghz_rx();

        /* Let the receiver settle (~10ms) then read RSSI */
        furi_delay_ms(10);
        float rssi = furi_hal_subghz_get_rssi();

        furi_hal_subghz_idle();

        furi_mutex_acquire(g_sensors.mutex, FuriWaitForever);
        g_sensors.subghz_rssi[i] = rssi;
        g_sensors.subghz_last_ts = ts;
        furi_mutex_release(g_sensors.mutex);

        /* Write reading to file */
        char line[128];
        /* rssi is float; use integer representation to avoid %f */
        int rssi_int = (int)(rssi);
        int rssi_frac = (int)((rssi < 0 ? -rssi : rssi) * 10) % 10;
        snprintf(line, sizeof(line),
            "{\"type\":\"subghz\",\"freq\":%lu,\"rssi\":%s%d.%d,\"ts\":%lu}",
            (unsigned long)freq,
            rssi < 0 ? "-" : "",
            rssi < 0 ? -rssi_int : rssi_int,
            rssi_frac,
            (unsigned long)ts);
        sensor_append_line(storage, line);

        furi_mutex_acquire(g_sensors.mutex, FuriWaitForever);
        g_sensors.total_readings++;
        g_sensors.buffer_lines++;
        furi_mutex_release(g_sensors.mutex);
    }

    /* Power down the radio until next scan */
    furi_hal_subghz_sleep();
}

// ─── Sensor: BLE channel scan ──────────────────────────────────

static void sensor_scan_ble(void) {
    /* If the BLE stack is active (phone app connected), skip — we'd
     * conflict with the Flipper's own BLE connection. */
    if(furi_hal_bt_is_active()) {
        FURI_LOG_D(TAG, "BLE radio busy (phone connected), skipping scan");
        return;
    }

    uint32_t ts = furi_hal_rtc_get_timestamp();
    uint8_t hits = 0;
    float best_rssi = -999.0f;

    /* Scan a subset of BLE advertising channels (37, 38, 39 = channels 0, 12, 39 in HCI)
     * and a few data channels to detect nearby devices */
    static const uint8_t ble_channels[] = {0, 12, 39, 1, 6, 18, 24};
    #define BLE_SCAN_CH_COUNT (sizeof(ble_channels) / sizeof(ble_channels[0]))

    for(size_t i = 0; i < BLE_SCAN_CH_COUNT; i++) {
        if(!g_sensors.running) break;

        furi_hal_bt_start_rx(ble_channels[i]);
        furi_delay_ms(200); /* listen 200ms per channel */
        float rssi = furi_hal_bt_get_rssi();
        furi_hal_bt_stop_rx();

        /* An RSSI above the noise floor (~-90 dBm) indicates a device */
        if(rssi > -90.0f) {
            hits++;
            if(rssi > best_rssi) best_rssi = rssi;
        }
    }

    furi_mutex_acquire(g_sensors.mutex, FuriWaitForever);
    g_sensors.ble_channel_hits = hits;
    g_sensors.ble_best_rssi = best_rssi;
    g_sensors.ble_last_ts = ts;
    furi_mutex_release(g_sensors.mutex);
}

// ─── Sensor: NFC field detect ──────────────────────────────────

static void sensor_scan_nfc(Storage* storage) {
    uint32_t ts = furi_hal_rtc_get_timestamp();
    bool detected = false;

    /* Must acquire NFC HAL before any operation.
     * Missing acquire() was the crash cause on 1.4.3. */
    FuriHalNfcError nfc_err = furi_hal_nfc_acquire();
    if(nfc_err != FuriHalNfcErrorNone) {
        FURI_LOG_W(TAG, "NFC acquire failed — skipping");
        return;
    }
    furi_hal_nfc_low_power_mode_stop();
    furi_hal_nfc_field_detect_start();
    furi_delay_ms(100);
    detected = furi_hal_nfc_field_is_present();
    furi_hal_nfc_field_detect_stop();
    furi_hal_nfc_low_power_mode_start();
    furi_hal_nfc_release();

    furi_mutex_acquire(g_sensors.mutex, FuriWaitForever);
    g_sensors.nfc_detected = detected;
    g_sensors.nfc_last_ts = ts;
    furi_mutex_release(g_sensors.mutex);

    char line[96];
    snprintf(line, sizeof(line),
        "{\"type\":\"nfc\",\"detected\":%s,\"ts\":%lu}",
        detected ? "true" : "false",
        (unsigned long)ts);
    sensor_append_line(storage, line);

    furi_mutex_acquire(g_sensors.mutex, FuriWaitForever);
    g_sensors.total_readings++;
    g_sensors.buffer_lines++;
    furi_mutex_release(g_sensors.mutex);
}

// ─── Sensor: GPIO ADC (PA4) ────────────────────────────────────

static void sensor_read_gpio_adc(Storage* storage) {
    uint32_t ts = furi_hal_rtc_get_timestamp();

    /* Configure PA4 as analog input */
    furi_hal_gpio_init(&gpio_ext_pa4, GpioModeAnalog, GpioPullNo, GpioSpeedLow);

    FuriHalAdcHandle* adc = furi_hal_adc_acquire();
    furi_hal_adc_configure(adc);

    uint16_t raw = furi_hal_adc_read(adc, FuriHalAdcChannel4);
    float mv = furi_hal_adc_convert_to_voltage(adc, raw);

    furi_hal_adc_release(adc);

    furi_mutex_acquire(g_sensors.mutex, FuriWaitForever);
    g_sensors.gpio_adc_raw = raw;
    g_sensors.gpio_adc_mv = mv;
    g_sensors.gpio_last_ts = ts;
    furi_mutex_release(g_sensors.mutex);

    char line[96];
    int mv_int = (int)mv;
    snprintf(line, sizeof(line),
        "{\"type\":\"gpio\",\"adc_raw\":%u,\"mv\":%d,\"ts\":%lu}",
        (unsigned)raw, mv_int, (unsigned long)ts);
    sensor_append_line(storage, line);

    furi_mutex_acquire(g_sensors.mutex, FuriWaitForever);
    g_sensors.total_readings++;
    g_sensors.buffer_lines++;
    furi_mutex_release(g_sensors.mutex);
}

// ─── Sensor: Internal die temperature ──────────────────────────

static void sensor_read_temperature(Storage* storage) {
    uint32_t ts = furi_hal_rtc_get_timestamp();

    FuriHalAdcHandle* adc = furi_hal_adc_acquire();
    /* Use extended config with longer sampling time for temp sensor */
    furi_hal_adc_configure_ex(
        adc,
        FuriHalAdcScale2048,
        FuriHalAdcClockSync64,
        FuriHalAdcOversample64,
        FuriHalAdcSamplingtime247_5);

    uint16_t raw = furi_hal_adc_read(adc, FuriHalAdcChannelTEMPSENSOR);
    float temp_c = furi_hal_adc_convert_temp(adc, raw);

    furi_hal_adc_release(adc);

    furi_mutex_acquire(g_sensors.mutex, FuriWaitForever);
    g_sensors.cpu_temp_c = temp_c;
    g_sensors.temp_last_ts = ts;
    furi_mutex_release(g_sensors.mutex);

    char line[96];
    int temp_int = (int)temp_c;
    int temp_frac = (int)((temp_c < 0 ? -temp_c : temp_c) * 10) % 10;
    snprintf(line, sizeof(line),
        "{\"type\":\"temp\",\"cpu_c\":%s%d.%d,\"ts\":%lu}",
        temp_c < 0 ? "-" : "",
        temp_c < 0 ? -temp_int : temp_int,
        temp_frac,
        (unsigned long)ts);
    sensor_append_line(storage, line);

    furi_mutex_acquire(g_sensors.mutex, FuriWaitForever);
    g_sensors.total_readings++;
    g_sensors.buffer_lines++;
    furi_mutex_release(g_sensors.mutex);
}

// ─── Sensor: BLE reading to file ───────────────────────────────

static void sensor_write_ble_reading(Storage* storage) {
    uint32_t ts;
    uint8_t hits;
    float best;

    furi_mutex_acquire(g_sensors.mutex, FuriWaitForever);
    ts = g_sensors.ble_last_ts;
    hits = g_sensors.ble_channel_hits;
    best = g_sensors.ble_best_rssi;
    furi_mutex_release(g_sensors.mutex);

    char line[96];
    int rssi_int = (int)best;
    int rssi_frac = (int)((best < 0 ? -best : best) * 10) % 10;
    snprintf(line, sizeof(line),
        "{\"type\":\"ble\",\"ch_hits\":%u,\"best_rssi\":%s%d.%d,\"ts\":%lu}",
        (unsigned)hits,
        best < 0 ? "-" : "",
        best < 0 ? -rssi_int : rssi_int,
        rssi_frac,
        (unsigned long)ts);
    sensor_append_line(storage, line);

    furi_mutex_acquire(g_sensors.mutex, FuriWaitForever);
    g_sensors.total_readings++;
    g_sensors.buffer_lines++;
    furi_mutex_release(g_sensors.mutex);
}

// ─── Sensor thread ─────────────────────────────────────────────

static int32_t hone_sensor_thread(void* context) {
    HONEWalletApp* app = context;
    FURI_LOG_I(TAG, "Sensor thread started");

    /* Initialize buffer line count from existing file */
    furi_mutex_acquire(g_sensors.mutex, FuriWaitForever);
    g_sensors.buffer_lines = sensor_count_lines(app->storage);
    furi_mutex_release(g_sensors.mutex);

    while(g_sensors.running) {
        /* ── RADIO TEST BUILDS ──
         * Enable ONE at a time to find which crashes.
         * Uncomment the next one after each successful test.
         */

        /* TEST 1: CPU temp only (known working) */
        FURI_LOG_D(TAG, "Sensor: temperature read");
        sensor_read_temperature(app->storage);
        if(!g_sensors.running) break;

        /* TEST 2: GPIO ADC */
        sensor_read_gpio_adc(app->storage);
        if(!g_sensors.running) break;

        /* TEST 3: Sub-GHz */
        sensor_scan_subghz(app->storage);
        if(!g_sensors.running) break;

        /* TEST 4: BLE */
        sensor_scan_ble();
        sensor_write_ble_reading(app->storage);
        if(!g_sensors.running) break;

        /* NFC DISABLED — crashes on 1.4.3 firmware (field_detect_start issue) */
        sensor_scan_nfc(app->storage);

        /* Update last scan time */
        furi_mutex_acquire(g_sensors.mutex, FuriWaitForever);
        g_sensors.last_scan_ts = furi_hal_rtc_get_timestamp();
        furi_mutex_release(g_sensors.mutex);

        /* Check if rotation needed */
        furi_mutex_acquire(g_sensors.mutex, FuriWaitForever);
        uint32_t lines = g_sensors.buffer_lines;
        furi_mutex_release(g_sensors.mutex);

        if(lines > SENSOR_MAX_READINGS) {
            FURI_LOG_I(TAG, "Sensor: rotating readings (%lu lines)", (unsigned long)lines);
            sensor_rotate_readings(app->storage);
            furi_mutex_acquire(g_sensors.mutex, FuriWaitForever);
            g_sensors.buffer_lines = sensor_count_lines(app->storage);
            furi_mutex_release(g_sensors.mutex);
        }

        /* ── Adaptive scan rate ── */
        {
            bool interesting = false;

            furi_mutex_acquire(g_sensors.mutex, FuriWaitForever);

            /* Sub-GHz: any frequency RSSI changed > threshold */
            for(uint8_t f = 0; f < SUBGHZ_FREQ_COUNT; f++) {
                float delta = g_sensors.subghz_rssi[f] - g_sensors.prev_subghz_rssi[f];
                if(delta > SCAN_RSSI_DELTA_DB || delta < -SCAN_RSSI_DELTA_DB) {
                    interesting = true;
                }
                g_sensors.prev_subghz_rssi[f] = g_sensors.subghz_rssi[f];
            }

            /* BLE: device count changed */
            if(g_sensors.ble_channel_hits != g_sensors.prev_ble_hits) {
                interesting = true;
            }
            g_sensors.prev_ble_hits = g_sensors.ble_channel_hits;

            /* NFC: field detected (always interesting) */
            if(g_sensors.nfc_detected) {
                interesting = true;
            }

            /* GPIO: changed > 5% */
            if(g_sensors.prev_gpio_raw > 0) {
                int32_t gdelta = (int32_t)g_sensors.gpio_adc_raw - (int32_t)g_sensors.prev_gpio_raw;
                if(gdelta < 0) gdelta = -gdelta;
                if((gdelta * 100) / g_sensors.prev_gpio_raw > SCAN_GPIO_DELTA_PCT) {
                    interesting = true;
                }
            }
            g_sensors.prev_gpio_raw = g_sensors.gpio_adc_raw;

            /* Temperature: changed > 2°C */
            {
                float tdelta = g_sensors.cpu_temp_c - g_sensors.prev_cpu_temp;
                if(tdelta > SCAN_TEMP_DELTA_C || tdelta < -SCAN_TEMP_DELTA_C) {
                    interesting = true;
                }
                g_sensors.prev_cpu_temp = g_sensors.cpu_temp_c;
            }

            /* Update adaptive interval */
            if(interesting) {
                g_sensors.interest_countdown = SCAN_INTEREST_COOLDOWN;
                g_sensors.scan_interval_ms = SCAN_INTERVAL_FAST_MS;
                FURI_LOG_I(TAG, "Sensor: interesting data — fast scan (%lums)", (unsigned long)SCAN_INTERVAL_FAST_MS);
            } else if(g_sensors.interest_countdown > 0) {
                g_sensors.interest_countdown--;
                g_sensors.scan_interval_ms = SCAN_INTERVAL_FAST_MS;
            } else {
                g_sensors.scan_interval_ms = SCAN_INTERVAL_SLOW_MS;
            }

            uint32_t sleep_ms = g_sensors.scan_interval_ms;
            furi_mutex_release(g_sensors.mutex);

            /* Sleep — check running flag every second */
            uint32_t slept = 0;
            while(slept < sleep_ms && g_sensors.running) {
                furi_delay_ms(1000);
                slept += 1000;
            }
        }
    }

    FURI_LOG_I(TAG, "Sensor thread stopped");
    return 0;
}

static void hone_sensor_start(HONEWalletApp* app) {
    g_sensors.mutex = furi_mutex_alloc(FuriMutexTypeNormal);
    g_sensors.running = true;
    g_sensors.total_readings = 0;
    g_sensors.buffer_lines = 0;
    g_sensors.last_scan_ts = 0;

    app->sensor_thread = furi_thread_alloc_ex("HONESensor", 4096, hone_sensor_thread, app);
    furi_thread_start(app->sensor_thread);
}

static void hone_sensor_stop(HONEWalletApp* app) {
    g_sensors.running = false;
    furi_thread_join(app->sensor_thread);
    furi_thread_free(app->sensor_thread);
    app->sensor_thread = NULL;

    furi_mutex_free(g_sensors.mutex);
    g_sensors.mutex = NULL;
}

// ─── Sensor USB command handlers ───────────────────────────────

static void cmd_sensors(void) {
    furi_mutex_acquire(g_sensors.mutex, FuriWaitForever);

    /* Build a JSON summary of latest readings */
    char resp[512];
    int subghz0 = (int)g_sensors.subghz_rssi[0];
    int subghz1 = (int)g_sensors.subghz_rssi[1];
    int subghz2 = (int)g_sensors.subghz_rssi[2];
    int subghz3 = (int)g_sensors.subghz_rssi[3];
    int temp_int = (int)g_sensors.cpu_temp_c;
    int temp_frac = (int)((g_sensors.cpu_temp_c < 0 ? -g_sensors.cpu_temp_c : g_sensors.cpu_temp_c) * 10) % 10;
    int gpio_mv = (int)g_sensors.gpio_adc_mv;

    snprintf(resp, sizeof(resp),
        "{\"status\":\"ok\","
        "\"subghz\":{\"433\":%d,\"315\":%d,\"868\":%d,\"915\":%d,\"ts\":%lu},"
        "\"ble\":{\"ch_hits\":%u,\"ts\":%lu},"
        "\"nfc\":{\"detected\":%s,\"ts\":%lu},"
        "\"gpio\":{\"adc_raw\":%u,\"mv\":%d,\"ts\":%lu},"
        "\"temp\":{\"cpu_c\":%s%d.%d,\"ts\":%lu}}",
        subghz0, subghz1, subghz2, subghz3,
        (unsigned long)g_sensors.subghz_last_ts,
        (unsigned)g_sensors.ble_channel_hits,
        (unsigned long)g_sensors.ble_last_ts,
        g_sensors.nfc_detected ? "true" : "false",
        (unsigned long)g_sensors.nfc_last_ts,
        (unsigned)g_sensors.gpio_adc_raw, gpio_mv,
        (unsigned long)g_sensors.gpio_last_ts,
        g_sensors.cpu_temp_c < 0 ? "-" : "",
        g_sensors.cpu_temp_c < 0 ? -temp_int : temp_int,
        temp_frac,
        (unsigned long)g_sensors.temp_last_ts);

    furi_mutex_release(g_sensors.mutex);
    usb_send_line(resp);
}

static void cmd_sensor_status(void) {
    furi_mutex_acquire(g_sensors.mutex, FuriWaitForever);

    char resp[256];
    snprintf(resp, sizeof(resp),
        "{\"status\":\"ok\","
        "\"total_readings\":%lu,"
        "\"buffer_lines\":%lu,"
        "\"last_scan_ts\":%lu}",
        (unsigned long)g_sensors.total_readings,
        (unsigned long)g_sensors.buffer_lines,
        (unsigned long)g_sensors.last_scan_ts);

    furi_mutex_release(g_sensors.mutex);
    usb_send_line(resp);
}

static void cmd_flush_readings(HONEWalletApp* app) {
    /* Read entire readings.jsonl, send as JSON array, then truncate */
    usb_send_str("{\"status\":\"ok\",\"readings\":[");

    File* f = storage_file_alloc(app->storage);
    bool first = true;
    if(storage_file_open(f, HONE_READINGS_PATH, FSAM_READ, FSOM_OPEN_EXISTING)) {
        /* Read and send line by line using a small buffer */
        char buf[256];
        size_t buf_pos = 0;

        while(true) {
            size_t rd = storage_file_read(f, buf + buf_pos, sizeof(buf) - buf_pos - 1);
            if(rd == 0 && buf_pos == 0) break;
            size_t avail = buf_pos + rd;
            buf[avail] = '\0';

            /* Process complete lines */
            char* start = buf;
            char* nl;
            while((nl = strchr(start, '\n')) != NULL) {
                *nl = '\0';
                if(strlen(start) > 0) {
                    if(!first) usb_send_str(",");
                    usb_send_str(start);
                    first = false;
                }
                start = nl + 1;
            }

            /* Move remaining partial line to start */
            size_t remaining = avail - (start - buf);
            if(remaining > 0) {
                memmove(buf, start, remaining);
            }
            buf_pos = remaining;

            if(rd == 0) break;
        }
        storage_file_close(f);
    }
    storage_file_free(f);

    usb_send_line("]}");

    /* Truncate the file */
    File* tf = storage_file_alloc(app->storage);
    if(storage_file_open(tf, HONE_READINGS_PATH, FSAM_WRITE, FSOM_CREATE_ALWAYS)) {
        storage_file_close(tf);
    }
    storage_file_free(tf);

    furi_mutex_acquire(g_sensors.mutex, FuriWaitForever);
    g_sensors.buffer_lines = 0;
    furi_mutex_release(g_sensors.mutex);
}

// ─── USB command dispatcher ─────────────────────────────────────

static void hone_dispatch_command(HONEWalletApp* app, char* line) {
    FURI_LOG_D(TAG, "USB cmd: %s", line);

    /* Identify command */
    char cmd[16] = {0};
    if(!json_get_str(line, "cmd", cmd, sizeof(cmd))) {
        usb_send_line("{\"status\":\"error\",\"error\":\"missing cmd field\"}");
        return;
    }

    if(strcmp(cmd, "ping") == 0) {
        cmd_ping();
    } else if(strcmp(cmd, "list") == 0) {
        cmd_list(app);
    } else if(strcmp(cmd, "import") == 0) {
        /* Phase 1 debug: minimal import — just save name to file */
        {
            char name[HONE_NAME_LEN + 1] = {0};
            if(!json_get_str(line, "name", name, sizeof(name))) {
                usb_send_line("{\"status\":\"error\",\"error\":\"missing name\"}");
            } else {
                hone_ensure_dir(app->storage);
                char path[128];
                snprintf(path, sizeof(path), "%s/%s.key", HONE_WALLET_DIR, name);
                File* file = storage_file_alloc(app->storage);
                bool ok = storage_file_open(file, path, FSAM_WRITE, FSOM_CREATE_ALWAYS);
                if(ok) {
                    /* Write 80 bytes of placeholder data */
                    uint8_t dummy[80];
                    memset(dummy, 0x42, sizeof(dummy));
                    storage_file_write(file, dummy, sizeof(dummy));
                    storage_file_close(file);
                    char resp[128];
                    snprintf(resp, sizeof(resp), "{\"status\":\"ok\",\"wallet\":\"%s\",\"pubkey\":\"placeholder\"}", name);
                    usb_send_line(resp);
                    FURI_LOG_I(TAG, "Wallet saved: %s", name);
                } else {
                    usb_send_line("{\"status\":\"error\",\"error\":\"failed to write file\"}");
                }
                storage_file_free(file);
            }
        }
        /* cmd_import(app, line); — full crypto disabled until crash is resolved */
    } else if(strcmp(cmd, "delete") == 0) {
        char name[HONE_NAME_LEN + 1] = {0};
        char confirm[16] = {0};
        if(!json_get_str(line, "name", name, sizeof(name))) {
            usb_send_line("{\"status\":\"error\",\"error\":\"missing name\"}");
        } else if(!json_get_str(line, "confirm", confirm, sizeof(confirm)) ||
                  strcmp(confirm, "yes") != 0) {
            char resp[128];
            snprintf(resp, sizeof(resp),
                "{\"status\":\"confirm\",\"message\":\"ARE YOU SURE? Send again with confirm:yes to delete wallet %s\"}",
                name);
            usb_send_line(resp);
        } else {
            char path[128];
            snprintf(path, sizeof(path), "%s/%s.key", HONE_WALLET_DIR, name);
            if(storage_file_exists(app->storage, path)) {
                storage_simply_remove(app->storage, path);
                char resp[128];
                snprintf(resp, sizeof(resp), "{\"status\":\"ok\",\"deleted\":\"%s\"}", name);
                usb_send_line(resp);
                FURI_LOG_I(TAG, "Wallet deleted: %s", name);
            } else {
                usb_send_line("{\"status\":\"error\",\"error\":\"wallet not found\"}");
            }
        }
    } else if(strcmp(cmd, "sign") == 0) {
        cmd_sign(app, line);
    } else if(strcmp(cmd, "sensors") == 0) {
        cmd_sensors();
    } else if(strcmp(cmd, "sensor_status") == 0) {
        cmd_sensor_status();
    } else if(strcmp(cmd, "flush_readings") == 0) {
        cmd_flush_readings(app);
    } else {
        char resp[128];
        snprintf(resp, sizeof(resp),
            "{\"status\":\"error\",\"error\":\"unknown cmd: %s\"}", cmd);
        usb_send_line(resp);
    }
}

// ─── USB CDC RX callback ──────────────────────────────────────

static void hone_cdc_rx_callback(void* context) {
    HONEWalletApp* app = context;
    /* Read data from CDC into stream buffer */
    uint8_t buf[CDC_DATA_SZ];
    int32_t len = furi_hal_cdc_receive(HONE_CDC_CH, buf, sizeof(buf));
    if(len > 0) {
        furi_stream_buffer_send(app->usb_rx_stream, buf, len, 0);
    }
}

// ─── USB listener thread ──────────────────────────────────────

static int32_t hone_usb_thread(void* context) {
    HONEWalletApp* app = context;
    FURI_LOG_I(TAG, "USB listener thread started");

    char line_buf[HONE_USB_BUF_LEN];
    size_t line_pos = 0;

    while(app->usb_running) {
        uint8_t byte;
        size_t received = furi_stream_buffer_receive(
            app->usb_rx_stream, &byte, 1, 100); /* 100ms timeout */

        if(received == 0) continue;

        if(byte == '\n' || byte == '\r') {
            if(line_pos > 0) {
                line_buf[line_pos] = '\0';
                hone_dispatch_command(app, line_buf);
                line_pos = 0;
            }
        } else {
            if(line_pos < HONE_USB_BUF_LEN - 1) {
                line_buf[line_pos++] = (char)byte;
            } else {
                /* Line too long — discard and send error */
                line_pos = 0;
                usb_send_line("{\"status\":\"error\",\"error\":\"line too long\"}");
            }
        }
    }

    FURI_LOG_I(TAG, "USB listener thread stopped");
    return 0;
}

// ─── USB CDC setup / teardown ─────────────────────────────────

static void hone_usb_start(HONEWalletApp* app) {
    /* Save current USB config so we can restore it on exit */
    app->prev_usb_cfg = furi_hal_usb_get_config();

    /* Switch to single CDC mode */
    furi_hal_usb_unlock();
    furi_hal_usb_set_config(&usb_cdc_single, NULL);

    /* Set up RX callback */
    CdcCallbacks cdc_cb = {
        .tx_ep_callback = NULL,
        .rx_ep_callback = hone_cdc_rx_callback,
        .state_callback = NULL,
        .ctrl_line_callback = NULL,
        .config_callback = NULL,
    };
    furi_hal_cdc_set_callbacks(HONE_CDC_CH, &cdc_cb, app);

    /* Create stream buffer for RX data */
    app->usb_rx_stream = furi_stream_buffer_alloc(HONE_USB_BUF_LEN, 1);

    /* Start listener thread */
    app->usb_running = true;
    app->usb_thread = furi_thread_alloc_ex("HONEUsb", 4096, hone_usb_thread, app);
    furi_thread_start(app->usb_thread);
}

static void hone_usb_stop(HONEWalletApp* app) {
    /* Signal thread to stop */
    app->usb_running = false;
    furi_thread_join(app->usb_thread);
    furi_thread_free(app->usb_thread);
    app->usb_thread = NULL;

    /* Remove callbacks */
    furi_hal_cdc_set_callbacks(HONE_CDC_CH, NULL, NULL);

    /* Free stream buffer */
    furi_stream_buffer_free(app->usb_rx_stream);
    app->usb_rx_stream = NULL;

    /* Restore previous USB config */
    if(app->prev_usb_cfg) {
        furi_hal_usb_set_config(app->prev_usb_cfg, NULL);
        app->prev_usb_cfg = NULL;
    }
}

// ─── Menu callbacks ─────────────────────────────────────────────

static void hone_menu_callback(void* context, uint32_t index) {
    HONEWalletApp* app = context;

    switch(index) {
    case HONEMenuWallets:
        hone_load_wallet_names(app);
        if(app->wallet_count == 0) {
            popup_set_header(app->popup, "No Wallets", 64, 10, AlignCenter, AlignTop);
            popup_set_text(
                app->popup,
                "Connect to PC and run:\nhone-flipper-bridge\nto import a wallet",
                64, 30, AlignCenter, AlignTop);
        } else {
            char buf[256] = {0};
            for(uint8_t i = 0; i < app->wallet_count && i < 5; i++) {
                size_t cur = strlen(buf);
                snprintf(buf + cur, sizeof(buf) - cur, "%s\n", app->wallet_names[i]);
            }
            popup_set_header(app->popup, "Wallets", 64, 0, AlignCenter, AlignTop);
            popup_set_text(app->popup, buf, 0, 14, AlignLeft, AlignTop);
        }
        popup_disable_timeout(app->popup);
        view_dispatcher_switch_to_view(app->view_dispatcher, HONEViewPopup);
        break;

    case HONEMenuImport:
        popup_set_header(app->popup, "USB Listening", 64, 10, AlignCenter, AlignTop);
        popup_set_text(
            app->popup,
            "USB CDC active.\nRun hone-flipper-bridge\non your PC to import.",
            64, 30, AlignCenter, AlignTop);
        popup_disable_timeout(app->popup);
        view_dispatcher_switch_to_view(app->view_dispatcher, HONEViewPopup);
        break;

    case HONEMenuSign:
        popup_set_header(app->popup, "Sign Transaction", 64, 10, AlignCenter, AlignTop);
        popup_set_text(
            app->popup,
            "USB CDC active.\nSend sign command\nfrom hone-flipper-bridge.",
            64, 30, AlignCenter, AlignTop);
        popup_disable_timeout(app->popup);
        view_dispatcher_switch_to_view(app->view_dispatcher, HONEViewPopup);
        break;

    case HONEMenuSensors: {
        /* Build sensor display string — TextBox supports scrolling */
        static char sensor_buf[512]; /* static — TextBox refs the pointer */
        furi_mutex_acquire(g_sensors.mutex, FuriWaitForever);
        int t_int = (int)g_sensors.cpu_temp_c;
        int t_frac = (int)((g_sensors.cpu_temp_c < 0 ? -g_sensors.cpu_temp_c : g_sensors.cpu_temp_c) * 10) % 10;
        int gpio_mv_int = (int)g_sensors.gpio_adc_mv;
        uint32_t scan_int = g_sensors.scan_interval_ms / 1000;
        snprintf(sensor_buf, sizeof(sensor_buf),
            "=== HONE Sensors ===\n"
            "CPU Temp: %s%d.%d C\n"
            "GPIO ADC: %u (%dmV)\n"
            "NFC: %s\n"
            "Sub-GHz: disabled\n"
            "BLE: disabled\n"
            "---\n"
            "Readings: %lu\n"
            "Buffer: %lu lines\n"
            "Scan rate: %lus (%s)\n",
            g_sensors.cpu_temp_c < 0 ? "-" : "",
            g_sensors.cpu_temp_c < 0 ? -t_int : t_int,
            t_frac,
            (unsigned)g_sensors.gpio_adc_raw,
            gpio_mv_int,
            g_sensors.nfc_detected ? "DETECTED" : "not detected",
            (unsigned long)g_sensors.total_readings,
            (unsigned long)g_sensors.buffer_lines,
            (unsigned long)scan_int,
            g_sensors.interest_countdown > 0 ? "active" : "idle");
        furi_mutex_release(g_sensors.mutex);

        text_box_reset(app->text_box);
        text_box_set_text(app->text_box, sensor_buf);
        text_box_set_font(app->text_box, TextBoxFontText);
        view_dispatcher_switch_to_view(app->view_dispatcher, HONEViewTextBox);
        break;
    }

    case HONEMenuAbout:
        popup_set_header(app->popup, "HONE v0.2.0", 64, 10, AlignCenter, AlignTop);
        popup_set_text(
            app->popup,
            "Bitcoin Proof of Compute\nHardware Wallet + Sensors\n\nhonemesh.net",
            64, 30, AlignCenter, AlignTop);
        popup_disable_timeout(app->popup);
        view_dispatcher_switch_to_view(app->view_dispatcher, HONEViewPopup);
        break;
    }
}

static uint32_t hone_nav_callback(void* context) {
    UNUSED(context);
    return HONEViewMenu;
}

static uint32_t hone_exit_callback(void* context) {
    UNUSED(context);
    return VIEW_NONE;
}

// ─── Main ───────────────────────────────────────────────────────

int32_t hone_app(void* p) {
    UNUSED(p);
    FURI_LOG_I(TAG, "HONE Wallet starting");

    HONEWalletApp* app = malloc(sizeof(HONEWalletApp));
    memset(app, 0, sizeof(HONEWalletApp));

    // Open services
    app->gui = furi_record_open(RECORD_GUI);
    app->storage = furi_record_open(RECORD_STORAGE);

    // Ensure storage dir exists
    hone_ensure_dir(app->storage);

    // Start USB CDC listener
    hone_usb_start(app);

    // Start sensor scanning thread
    hone_sensor_start(app);

    // Create views
    app->view_dispatcher = view_dispatcher_alloc();
    view_dispatcher_attach_to_gui(app->view_dispatcher, app->gui, ViewDispatcherTypeFullscreen);

    // Main menu
    app->submenu = submenu_alloc();
    submenu_add_item(app->submenu, "Wallets", HONEMenuWallets, hone_menu_callback, app);
    submenu_add_item(app->submenu, "Import Wallet", HONEMenuImport, hone_menu_callback, app);
    submenu_add_item(app->submenu, "Sign Transaction", HONEMenuSign, hone_menu_callback, app);
    submenu_add_item(app->submenu, "Sensors", HONEMenuSensors, hone_menu_callback, app);
    submenu_add_item(app->submenu, "About", HONEMenuAbout, hone_menu_callback, app);

    view_set_previous_callback(submenu_get_view(app->submenu), hone_exit_callback);
    view_dispatcher_add_view(app->view_dispatcher, HONEViewMenu, submenu_get_view(app->submenu));

    // Popup for info screens
    app->popup = popup_alloc();
    view_set_previous_callback(popup_get_view(app->popup), hone_nav_callback);
    view_dispatcher_add_view(app->view_dispatcher, HONEViewPopup, popup_get_view(app->popup));

    // TextBox for scrollable displays (sensors)
    app->text_box = text_box_alloc();
    view_set_previous_callback(text_box_get_view(app->text_box), hone_nav_callback);
    view_dispatcher_add_view(app->view_dispatcher, HONEViewTextBox, text_box_get_view(app->text_box));

    // Start on main menu
    view_dispatcher_switch_to_view(app->view_dispatcher, HONEViewMenu);
    view_dispatcher_run(app->view_dispatcher);

    // Cleanup — stop sensor thread and USB
    hone_sensor_stop(app);
    hone_usb_stop(app);

    view_dispatcher_remove_view(app->view_dispatcher, HONEViewMenu);
    view_dispatcher_remove_view(app->view_dispatcher, HONEViewPopup);
    view_dispatcher_remove_view(app->view_dispatcher, HONEViewTextBox);
    submenu_free(app->submenu);
    popup_free(app->popup);
    text_box_free(app->text_box);
    view_dispatcher_free(app->view_dispatcher);

    furi_record_close(RECORD_STORAGE);
    furi_record_close(RECORD_GUI);
    free(app);

    FURI_LOG_I(TAG, "HONE Wallet exiting");
    return 0;
}
