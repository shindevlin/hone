#ifndef TWEETNACL_H
#define TWEETNACL_H

/*
 * TweetNaCl — public domain crypto library
 * Daniel J. Bernstein, Bernard van Gastel, Wesley Janssen,
 * Tanja Lange, Peter Schwabe, Sjaak Smetsers
 * https://tweetnacl.cr.yp.to/
 *
 * Selected subset: ed25519 sign/verify (crypto_sign_*)
 *
 * Callers must provide:
 *   void randombytes(uint8_t *x, uint64_t xlen);
 * See crypto/ed25519.c for the Flipper furi_hal_random implementation.
 */

#include <stdint.h>

#define crypto_sign_BYTES        64U
#define crypto_sign_PUBLICKEYBYTES 32U
#define crypto_sign_SECRETKEYBYTES 64U

extern int crypto_sign_keypair(uint8_t *pk, uint8_t *sk);
extern int crypto_sign(uint8_t *sm, uint64_t *smlen,
                       const uint8_t *m, uint64_t mlen,
                       const uint8_t *sk);
extern int crypto_sign_open(uint8_t *m, uint64_t *mlen,
                            const uint8_t *sm, uint64_t smlen,
                            const uint8_t *pk);

#endif /* TWEETNACL_H */
