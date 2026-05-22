/*
 * btcpc_protocol.c — BLE message serialisation, signing, and parsing
 *
 * Shin Devlin — btcpc.network
 */

#include "btcpc_protocol.h"
#include "../crypto/ed25519.h"

#include <string.h>
#include <stdbool.h>

/* ─── Internal helpers ─────────────────────────────────────────────────── */

static const uint8_t MAGIC[BTCPC_MAGIC_LEN] = BTCPC_FRAME_MAGIC;

static void frame_init(BtcpcFrameHeader* hdr, BtcpcMsgType type, uint16_t payload_len) {
    memcpy(hdr->magic, MAGIC, BTCPC_MAGIC_LEN);
    hdr->msg_type    = (uint8_t)type;
    hdr->payload_len = payload_len;
    memset(hdr->sig, 0, BTCPC_SIG_LEN);
}

/*
 * btcpc_frame_sign()
 *
 * Fill hdr->sig with an ed25519 signature over the payload.
 * The caller must have already set hdr->payload_len and populated payload[].
 */
static void frame_sign(BtcpcFrameHeader* hdr,
                       const uint8_t*    payload,
                       const uint8_t     sk[BTCPC_ED25519_SK_LEN]) {
    if(hdr->payload_len == 0 || payload == NULL) {
        memset(hdr->sig, 0, BTCPC_SIG_LEN);
        return;
    }
    btcpc_ed25519_sign(hdr->sig, payload, hdr->payload_len, sk);
}

/*
 * btcpc_frame_verify()
 *
 * Returns true if the frame signature is valid for the given public key.
 */
bool btcpc_frame_verify(const BtcpcFrameHeader* hdr,
                        const uint8_t*          payload,
                        const uint8_t           pk[BTCPC_ED25519_PK_LEN]) {
    if(memcmp(hdr->magic, MAGIC, BTCPC_MAGIC_LEN) != 0) return false;
    if(hdr->payload_len == 0 || payload == NULL) return false;
    return btcpc_ed25519_verify(hdr->sig, payload, hdr->payload_len, pk) == 0;
}

/* ─── Serialisers (Flipper → phone) ────────────────────────────────────── */

/*
 * btcpc_build_subghz()
 *
 * Serialise a Sub-GHz observation into `frame`, signing with `sk`.
 * Returns total bytes (header + payload) ready for BLE TX.
 */
size_t btcpc_build_subghz(BtcpcFrame*           frame,
                           const BtcpcSubGhzObs* obs,
                           const uint8_t         sk[BTCPC_ED25519_SK_LEN]) {
    uint16_t plen = (uint16_t)sizeof(BtcpcSubGhzObs);
    frame_init(&frame->hdr, BTCPC_MSG_SUBGHZ_OBS, plen);
    memcpy(frame->payload, obs, plen);
    frame_sign(&frame->hdr, frame->payload, sk);
    return BTCPC_FRAME_HEADER_SIZE + plen;
}

size_t btcpc_build_rfid(BtcpcFrame*          frame,
                         const BtcpcRfidScan* scan,
                         const uint8_t        sk[BTCPC_ED25519_SK_LEN]) {
    uint16_t plen = (uint16_t)sizeof(BtcpcRfidScan);
    frame_init(&frame->hdr, BTCPC_MSG_RFID_SCAN, plen);
    memcpy(frame->payload, scan, plen);
    frame_sign(&frame->hdr, frame->payload, sk);
    return BTCPC_FRAME_HEADER_SIZE + plen;
}

size_t btcpc_build_nfc(BtcpcFrame*         frame,
                        const BtcpcNfcScan* scan,
                        const uint8_t       sk[BTCPC_ED25519_SK_LEN]) {
    uint16_t plen = (uint16_t)sizeof(BtcpcNfcScan);
    frame_init(&frame->hdr, BTCPC_MSG_NFC_SCAN, plen);
    memcpy(frame->payload, scan, plen);
    frame_sign(&frame->hdr, frame->payload, sk);
    return BTCPC_FRAME_HEADER_SIZE + plen;
}

size_t btcpc_build_ibutton(BtcpcFrame*        frame,
                             const BtcpcIButton* btn,
                             const uint8_t       sk[BTCPC_ED25519_SK_LEN]) {
    uint16_t plen = (uint16_t)sizeof(BtcpcIButton);
    frame_init(&frame->hdr, BTCPC_MSG_IBUTTON, plen);
    memcpy(frame->payload, btn, plen);
    frame_sign(&frame->hdr, frame->payload, sk);
    return BTCPC_FRAME_HEADER_SIZE + plen;
}

size_t btcpc_build_heartbeat(BtcpcFrame*            frame,
                               const BtcpcHeartbeat*  hb,
                               const uint8_t          sk[BTCPC_ED25519_SK_LEN]) {
    uint16_t plen = (uint16_t)sizeof(BtcpcHeartbeat);
    frame_init(&frame->hdr, BTCPC_MSG_HEARTBEAT, plen);
    memcpy(frame->payload, hb, plen);
    frame_sign(&frame->hdr, frame->payload, sk);
    return BTCPC_FRAME_HEADER_SIZE + plen;
}

/* ─── Parser (phone → Flipper) ─────────────────────────────────────────── */

/*
 * btcpc_parse_frame()
 *
 * Parse an incoming BLE buffer into hdr + payload pointers.
 * Does NOT verify signature (phone→flipper messages are not signed by phone).
 *
 * Returns true if the frame is structurally valid (magic OK, length fits).
 * `payload_out` points into `buf` — no copy is made.
 */
bool btcpc_parse_frame(const uint8_t*          buf,
                        size_t                  buf_len,
                        BtcpcFrameHeader*       hdr_out,
                        const uint8_t**         payload_out) {
    if(buf_len < BTCPC_FRAME_HEADER_SIZE) return false;
    memcpy(hdr_out, buf, BTCPC_FRAME_HEADER_SIZE);
    if(memcmp(hdr_out->magic, MAGIC, BTCPC_MAGIC_LEN) != 0) return false;
    if(buf_len < BTCPC_FRAME_HEADER_SIZE + hdr_out->payload_len) return false;
    *payload_out = buf + BTCPC_FRAME_HEADER_SIZE;
    return true;
}

/*
 * btcpc_parse_clock_sync()
 *
 * Extract unix_ms from a BTCPC_MSG_CLOCK_SYNC payload.
 * Returns true on success.
 */
bool btcpc_parse_clock_sync(const uint8_t* payload,
                              uint16_t       payload_len,
                              uint64_t*      unix_ms_out) {
    if(payload_len < sizeof(BtcpcClockSync)) return false;
    BtcpcClockSync cs;
    memcpy(&cs, payload, sizeof(cs));
    *unix_ms_out = cs.unix_ms;
    return true;
}

/*
 * btcpc_parse_gps()
 *
 * Extract GPS coordinates from a BTCPC_MSG_GPS payload.
 * Returns true on success.
 */
bool btcpc_parse_gps(const uint8_t* payload,
                      uint16_t       payload_len,
                      BtcpcGps*      gps_out) {
    if(payload_len < sizeof(BtcpcGps)) return false;
    memcpy(gps_out, payload, sizeof(BtcpcGps));
    return true;
}
