/*
 * ed25519.c — TweetNaCl wrapper for Flipper Zero
 *
 * Provides randombytes() via furi_hal_random_get() (TRNG on STM32WB55),
 * then delegates ed25519 keypair/sign/verify to TweetNaCl.
 *
 * TweetNaCl uses SHA-512 internally (included within tweetnacl.c).
 * No external dependencies beyond furi_hal.
 *
 * Shin Devlin — honemesh.network
 */

#include "ed25519.h"
#include "tweetnacl.h"

#include <furi_hal_random.h>
#include <string.h>

/* ─── randombytes() — required by TweetNaCl ────────────────────────────── */

/*
 * TweetNaCl declares: void randombytes(uint8_t* x, uint64_t xlen);
 * On Flipper, furi_hal_random_get() returns one uint32_t from the TRNG.
 * We fill the buffer word by word.
 */
void randombytes(uint8_t* x, uint64_t xlen) {
    uint64_t remaining = xlen;
    while(remaining >= 4) {
        uint32_t word = furi_hal_random_get();
        x[0] = (uint8_t)(word);
        x[1] = (uint8_t)(word >> 8);
        x[2] = (uint8_t)(word >> 16);
        x[3] = (uint8_t)(word >> 24);
        x += 4;
        remaining -= 4;
    }
    if(remaining > 0) {
        uint32_t word = furi_hal_random_get();
        for(uint64_t i = 0; i < remaining; i++) {
            x[i] = (uint8_t)(word >> (i * 8));
        }
    }
}

/* ─── Public API ────────────────────────────────────────────────────────── */

void hone_ed25519_keypair(uint8_t pk_out[HONE_ED25519_PK_LEN],
                           uint8_t sk_out[HONE_ED25519_SK_LEN]) {
    /*
     * TweetNaCl: crypto_sign_keypair(pk, sk)
     * Generates pk (32 bytes) and sk (64 bytes = seed || pk) using randombytes().
     */
    crypto_sign_keypair(pk_out, sk_out);
}

void hone_ed25519_sign(uint8_t        sig_out[HONE_ED25519_SIG_LEN],
                        const uint8_t* msg,
                        size_t         msg_len,
                        const uint8_t  sk[HONE_ED25519_SK_LEN]) {
    /*
     * TweetNaCl: crypto_sign(signed_msg, &signed_msg_len, msg, msg_len, sk)
     * signed_msg = sig (64 bytes) || msg
     *
     * We only want the 64-byte signature prefix, so we use a stack buffer
     * big enough for the signature itself. For messages <= 448 bytes this
     * fits without heap allocation; for larger messages the caller should
     * call crypto_sign directly with a heap buffer.
     *
     * Here we allocate a small signed-message buffer on the stack.
     * Maximum payload size for this wrapper: HONE_SIGN_MSG_MAX bytes.
     */
#define HONE_SIGN_MSG_MAX 512
    if(msg_len > HONE_SIGN_MSG_MAX) {
        /* Caller must use crypto_sign() directly for large messages */
        return;
    }

    uint8_t  sm[HONE_SIGN_MSG_MAX + HONE_ED25519_SIG_LEN];
    uint64_t smlen = 0;

    crypto_sign(sm, &smlen, msg, (uint64_t)msg_len, sk);

    /* First 64 bytes of sm are the signature */
    memcpy(sig_out, sm, HONE_ED25519_SIG_LEN);
#undef HONE_SIGN_MSG_MAX
}

int hone_ed25519_verify(const uint8_t  sig[HONE_ED25519_SIG_LEN],
                         const uint8_t* msg,
                         size_t         msg_len,
                         const uint8_t  pk[HONE_ED25519_PK_LEN]) {
    /*
     * TweetNaCl: crypto_sign_open(msg_out, &msg_len_out, signed_msg, smlen, pk)
     * We reconstruct the signed message (sig || msg) on the stack.
     */
#define HONE_VERIFY_MSG_MAX 512
    if(msg_len > HONE_VERIFY_MSG_MAX) {
        return -1;
    }

    uint8_t  sm[HONE_VERIFY_MSG_MAX + HONE_ED25519_SIG_LEN];
    uint8_t  m[HONE_VERIFY_MSG_MAX];
    uint64_t mlen = 0;

    memcpy(sm, sig, HONE_ED25519_SIG_LEN);
    memcpy(sm + HONE_ED25519_SIG_LEN, msg, msg_len);

    int rc = crypto_sign_open(m, &mlen, sm, (uint64_t)(msg_len + HONE_ED25519_SIG_LEN), pk);
    return rc;
#undef HONE_VERIFY_MSG_MAX
}
