#pragma once

#include <stdint.h>
#include <stddef.h>

/*
 * ed25519.h — thin wrapper around TweetNaCl for Flipper Zero
 *
 * TweetNaCl (tweetnacl.c/.h) provides the core ed25519 primitives.
 * This wrapper adds:
 *   - randombytes() backed by furi_hal_random_get()
 *   - convenience functions matching the HONE naming convention
 *
 * Key sizes:
 *   Public key : 32 bytes
 *   Secret key : 64 bytes (TweetNaCl expanded: seed || public)
 *   Signature  : 64 bytes
 */

#define HONE_ED25519_PK_LEN  32
#define HONE_ED25519_SK_LEN  64
#define HONE_ED25519_SIG_LEN 64

/*
 * hone_ed25519_keypair()
 *
 * Generate a new ed25519 keypair.
 * pk_out: 32-byte public key  (HONE_ED25519_PK_LEN)
 * sk_out: 64-byte secret key  (HONE_ED25519_SK_LEN)
 *
 * Internally calls randombytes() → furi_hal_random_get() for the seed.
 */
void hone_ed25519_keypair(uint8_t pk_out[HONE_ED25519_PK_LEN],
                           uint8_t sk_out[HONE_ED25519_SK_LEN]);

/*
 * hone_ed25519_sign()
 *
 * Sign `msg_len` bytes of `msg` using `sk` (64-byte secret key).
 * Writes signature to `sig_out` (64 bytes).
 */
void hone_ed25519_sign(uint8_t       sig_out[HONE_ED25519_SIG_LEN],
                        const uint8_t* msg,
                        size_t         msg_len,
                        const uint8_t  sk[HONE_ED25519_SK_LEN]);

/*
 * hone_ed25519_verify()
 *
 * Verify a signature.
 * Returns 0 on success, non-zero on failure.
 */
int hone_ed25519_verify(const uint8_t  sig[HONE_ED25519_SIG_LEN],
                         const uint8_t* msg,
                         size_t         msg_len,
                         const uint8_t  pk[HONE_ED25519_PK_LEN]);
