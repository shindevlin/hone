/*
 * hone_protocol.c — BLE message serialisation, signing, and parsing
 *
 * Shin Devlin — honemesh.network
 */

#include "hone_protocol.h"
#include "../crypto/ed25519.h"

#include <string.h>
#include <stdbool.h>

/* ─── Internal helpers ─────────────────────────────────────────────────── */

static const uint8_t MAGIC[HONE_MAGIC_LEN] = HONE_FRAME_MAGIC;

static void frame_init(HoneFrameHeader* hdr, HoneMsgType type, uint16_t payload_len) {
    memcpy(hdr->magic, MAGIC, HONE_MAGIC_LEN);
    hdr->msg_type    = (uint8_t)type;
    hdr->payload_len = payload_len;
    memset(hdr->sig, 0, HONE_SIG_LEN);
}

/*
 * hone_frame_sign()
 *
 * Fill hdr->sig with an ed25519 signature over the payload.
 * The caller must have already set hdr->payload_len and populated payload[].
 */
static void frame_sign(HoneFrameHeader* hdr,
                       const uint8_t*    payload,
                       const uint8_t     sk[HONE_ED25519_SK_LEN]) {
    if(hdr->payload_len == 0 || payload == NULL) {
        memset(hdr->sig, 0, HONE_SIG_LEN);
        return;
    }
    hone_ed25519_sign(hdr->sig, payload, hdr->payload_len, sk);
}

/*
 * hone_frame_verify()
 *
 * Returns true if the frame signature is valid for the given public key.
 */
bool hone_frame_verify(const HoneFrameHeader* hdr,
                        const uint8_t*          payload,
                        const uint8_t           pk[HONE_ED25519_PK_LEN]) {
    if(memcmp(hdr->magic, MAGIC, HONE_MAGIC_LEN) != 0) return false;
    if(hdr->payload_len == 0 || payload == NULL) return false;
    return hone_ed25519_verify(hdr->sig, payload, hdr->payload_len, pk) == 0;
}

/* ─── Serialisers (Flipper → phone) ────────────────────────────────────── */

/*
 * hone_build_subghz()
 *
 * Serialise a Sub-GHz observation into `frame`, signing with `sk`.
 * Returns total bytes (header + payload) ready for BLE TX.
 */
size_t hone_build_subghz(HoneFrame*           frame,
                           const HoneSubGhzObs* obs,
                           const uint8_t         sk[HONE_ED25519_SK_LEN]) {
    uint16_t plen = (uint16_t)sizeof(HoneSubGhzObs);
    frame_init(&frame->hdr, HONE_MSG_SUBGHZ_OBS, plen);
    memcpy(frame->payload, obs, plen);
    frame_sign(&frame->hdr, frame->payload, sk);
    return HONE_FRAME_HEADER_SIZE + plen;
}

size_t hone_build_rfid(HoneFrame*          frame,
                         const HoneRfidScan* scan,
                         const uint8_t        sk[HONE_ED25519_SK_LEN]) {
    uint16_t plen = (uint16_t)sizeof(HoneRfidScan);
    frame_init(&frame->hdr, HONE_MSG_RFID_SCAN, plen);
    memcpy(frame->payload, scan, plen);
    frame_sign(&frame->hdr, frame->payload, sk);
    return HONE_FRAME_HEADER_SIZE + plen;
}

size_t hone_build_nfc(HoneFrame*         frame,
                        const HoneNfcScan* scan,
                        const uint8_t       sk[HONE_ED25519_SK_LEN]) {
    uint16_t plen = (uint16_t)sizeof(HoneNfcScan);
    frame_init(&frame->hdr, HONE_MSG_NFC_SCAN, plen);
    memcpy(frame->payload, scan, plen);
    frame_sign(&frame->hdr, frame->payload, sk);
    return HONE_FRAME_HEADER_SIZE + plen;
}

size_t hone_build_ibutton(HoneFrame*        frame,
                             const HoneIButton* btn,
                             const uint8_t       sk[HONE_ED25519_SK_LEN]) {
    uint16_t plen = (uint16_t)sizeof(HoneIButton);
    frame_init(&frame->hdr, HONE_MSG_IBUTTON, plen);
    memcpy(frame->payload, btn, plen);
    frame_sign(&frame->hdr, frame->payload, sk);
    return HONE_FRAME_HEADER_SIZE + plen;
}

size_t hone_build_heartbeat(HoneFrame*            frame,
                               const HoneHeartbeat*  hb,
                               const uint8_t          sk[HONE_ED25519_SK_LEN]) {
    uint16_t plen = (uint16_t)sizeof(HoneHeartbeat);
    frame_init(&frame->hdr, HONE_MSG_HEARTBEAT, plen);
    memcpy(frame->payload, hb, plen);
    frame_sign(&frame->hdr, frame->payload, sk);
    return HONE_FRAME_HEADER_SIZE + plen;
}

/* ─── Parser (phone → Flipper) ─────────────────────────────────────────── */

/*
 * hone_parse_frame()
 *
 * Parse an incoming BLE buffer into hdr + payload pointers.
 * Does NOT verify signature (phone→flipper messages are not signed by phone).
 *
 * Returns true if the frame is structurally valid (magic OK, length fits).
 * `payload_out` points into `buf` — no copy is made.
 */
bool hone_parse_frame(const uint8_t*          buf,
                        size_t                  buf_len,
                        HoneFrameHeader*       hdr_out,
                        const uint8_t**         payload_out) {
    if(buf_len < HONE_FRAME_HEADER_SIZE) return false;
    memcpy(hdr_out, buf, HONE_FRAME_HEADER_SIZE);
    if(memcmp(hdr_out->magic, MAGIC, HONE_MAGIC_LEN) != 0) return false;
    if(buf_len < HONE_FRAME_HEADER_SIZE + hdr_out->payload_len) return false;
    *payload_out = buf + HONE_FRAME_HEADER_SIZE;
    return true;
}

/*
 * hone_parse_clock_sync()
 *
 * Extract unix_ms from a HONE_MSG_CLOCK_SYNC payload.
 * Returns true on success.
 */
bool hone_parse_clock_sync(const uint8_t* payload,
                              uint16_t       payload_len,
                              uint64_t*      unix_ms_out) {
    if(payload_len < sizeof(HoneClockSync)) return false;
    HoneClockSync cs;
    memcpy(&cs, payload, sizeof(cs));
    *unix_ms_out = cs.unix_ms;
    return true;
}

/*
 * hone_parse_gps()
 *
 * Extract GPS coordinates from a HONE_MSG_GPS payload.
 * Returns true on success.
 */
bool hone_parse_gps(const uint8_t* payload,
                      uint16_t       payload_len,
                      HoneGps*      gps_out) {
    if(payload_len < sizeof(HoneGps)) return false;
    memcpy(gps_out, payload, sizeof(HoneGps));
    return true;
}
