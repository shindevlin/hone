/* Minimal SHA-256 + HMAC-SHA-256 — public domain (Brad Conte / original) */
#pragma once
#include <stdint.h>
#include <stddef.h>

typedef struct {
    uint8_t  data[64];
    uint32_t datalen;
    uint64_t bitlen;
    uint32_t state[8];
} SHA256_CTX;

void sha256_init(SHA256_CTX *ctx);
void sha256_update(SHA256_CTX *ctx, const uint8_t *data, size_t len);
void sha256_final(SHA256_CTX *ctx, uint8_t hash[32]);

/* HMAC-SHA256: out must be 32 bytes */
void hmac_sha256(const uint8_t *key, size_t key_len,
                 const uint8_t *msg, size_t msg_len,
                 uint8_t out[32]);
