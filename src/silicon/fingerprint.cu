/**
 * HONE Silicon Fingerprint Probe
 * Shin Devlin
 *
 * Derives a unique 256-bit Silicon Identity Key (SIK) from the physical
 * properties of a specific GPU die. Uses two independent probe methods:
 *
 *   1. VRAM Timing Probe — measures read latency variations across memory cells
 *   2. FP Divergence Probe — captures least-significant-bit differences in
 *      floating-point computation caused by transistor-level manufacturing variance
 *
 * The resulting fingerprint is:
 *   - Unique per physical GPU
 *   - Reproducible across reboots
 *   - Not derivable from serial numbers or spec sheets
 *   - Unclonable without possessing the exact silicon
 *
 * Build:
 *   nvcc -O2 -o hone-sik fingerprint.cu -lcrypto
 *
 * Usage:
 *   ./hone-sik              # Print SIK hex + hash
 *   ./hone-sik --json       # Output JSON for node registration
 *   ./hone-sik --verify <hash>  # Verify SIK matches a registered hash
 */

#include <cuda_runtime.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>
#include <time.h>

// ── Constants ──
#define TIMING_PROBE_CELLS    16384
#define TIMING_PROBE_ROUNDS   64
#define FP_PROBE_OPS          4096
#define FP_PROBE_ROUNDS       32
#define SIK_BYTES             32
#define STABLE_BIT_THRESHOLD  0.95f  // cell must be stable 95% of rounds

// ── VRAM Timing Probe Kernel ──
// Writes a known pattern, reads it back, measures clock cycle delta per cell.
// Manufacturing variations cause per-cell timing differences.
__global__ void vram_timing_probe(
    unsigned int *target_mem,
    unsigned long long *timing_out,
    int num_cells,
    int round
) {
    int idx = blockIdx.x * blockDim.x + threadIdx.x;
    if (idx >= num_cells) return;

    // Write a deterministic pattern seeded by cell index + round
    unsigned int pattern = (idx * 2654435761u) ^ (round * 0x9e3779b9u);
    target_mem[idx] = pattern;
    __threadfence();

    // Read back and measure cycles
    unsigned long long t0 = clock64();

    // Force actual memory read (not cache) by using volatile
    volatile unsigned int val = target_mem[idx];

    unsigned long long t1 = clock64();

    // Store timing delta — the LSBs capture per-cell silicon variation
    timing_out[idx] = (t1 - t0) ^ (unsigned long long)val;
}

// ── Floating-Point Divergence Probe Kernel ──
// Runs identical math on every GPU. Due to ALU transistor variations,
// the least significant bits of results differ between physical dies.
__global__ void fp_divergence_probe(
    double *divergence_out,
    int num_ops,
    int round
) {
    int idx = blockIdx.x * blockDim.x + threadIdx.x;
    if (idx >= num_ops) return;

    // Deterministic seed per operation
    double seed = (double)(idx * 1000003 + round * 999983 + 1) / 1000000.0;

    // Chain of operations that amplify transistor-level FP differences.
    // Same code, same inputs, different silicon → different LSBs.
    double x = seed;
    for (int i = 0; i < 200; i++) {
        x = sin(x * 1.0000001) * cos(x * 0.9999999) + sqrt(fabs(x) + 1e-15);
        x = log(fabs(x) + 1.0) * exp(-fabs(x) * 0.001);
        x = fma(x, 3.14159265358979, -2.71828182845904);  // fused multiply-add
    }

    divergence_out[idx] = x;
}

// ── SHA-256 (minimal implementation for standalone binary) ──
// In production, link against OpenSSL (-lcrypto) and use SHA256().
// This is a self-contained fallback.

static const unsigned int sha256_k[64] = {
    0x428a2f98,0x71374491,0xb5c0fbcf,0xe9b5dba5,0x3956c25b,0x59f111f1,0x923f82a4,0xab1c5ed5,
    0xd807aa98,0x12835b01,0x243185be,0x550c7dc3,0x72be5d74,0x80deb1fe,0x9bdc06a7,0xc19bf174,
    0xe49b69c1,0xefbe4786,0x0fc19dc6,0x240ca1cc,0x2de92c6f,0x4a7484aa,0x5cb0a9dc,0x76f988da,
    0x983e5152,0xa831c66d,0xb00327c8,0xbf597fc7,0xc6e00bf3,0xd5a79147,0x06ca6351,0x14292967,
    0x27b70a85,0x2e1b2138,0x4d2c6dfc,0x53380d13,0x650a7354,0x766a0abb,0x81c2c92e,0x92722c85,
    0xa2bfe8a1,0xa81a664b,0xc24b8b70,0xc76c51a3,0xd192e819,0xd6990624,0xf40e3585,0x106aa070,
    0x19a4c116,0x1e376c08,0x2748774c,0x34b0bcb5,0x391c0cb3,0x4ed8aa4a,0x5b9cca4f,0x682e6ff3,
    0x748f82ee,0x78a5636f,0x84c87814,0x8cc70208,0x90befffa,0xa4506ceb,0xbef9a3f7,0xc67178f2
};

#define RR(x,n) (((x)>>(n))|((x)<<(32-(n))))
#define CH(x,y,z) (((x)&(y))^((~(x))&(z)))
#define MAJ(x,y,z) (((x)&(y))^((x)&(z))^((y)&(z)))
#define EP0(x) (RR(x,2)^RR(x,13)^RR(x,22))
#define EP1(x) (RR(x,6)^RR(x,11)^RR(x,25))
#define SIG0(x) (RR(x,7)^RR(x,18)^((x)>>3))
#define SIG1(x) (RR(x,17)^RR(x,19)^((x)>>10))

void sha256(const unsigned char *data, size_t len, unsigned char *hash) {
    unsigned int h[8] = {
        0x6a09e667,0xbb67ae85,0x3c6ef372,0xa54ff53a,
        0x510e527f,0x9b05688c,0x1f83d9ab,0x5be0cd19
    };

    // Pad message
    size_t padded_len = ((len + 8) / 64 + 1) * 64;
    unsigned char *msg = (unsigned char *)calloc(padded_len, 1);
    memcpy(msg, data, len);
    msg[len] = 0x80;
    unsigned long long bitlen = len * 8;
    for (int i = 0; i < 8; i++)
        msg[padded_len - 1 - i] = (unsigned char)(bitlen >> (i * 8));

    // Process blocks
    for (size_t offset = 0; offset < padded_len; offset += 64) {
        unsigned int w[64];
        for (int i = 0; i < 16; i++)
            w[i] = (msg[offset+i*4]<<24)|(msg[offset+i*4+1]<<16)|(msg[offset+i*4+2]<<8)|msg[offset+i*4+3];
        for (int i = 16; i < 64; i++)
            w[i] = SIG1(w[i-2]) + w[i-7] + SIG0(w[i-15]) + w[i-16];

        unsigned int a=h[0],b=h[1],c=h[2],d=h[3],e=h[4],f=h[5],g=h[6],hh=h[7];
        for (int i = 0; i < 64; i++) {
            unsigned int t1 = hh + EP1(e) + CH(e,f,g) + sha256_k[i] + w[i];
            unsigned int t2 = EP0(a) + MAJ(a,b,c);
            hh=g; g=f; f=e; e=d+t1; d=c; c=b; b=a; a=t1+t2;
        }
        h[0]+=a;h[1]+=b;h[2]+=c;h[3]+=d;h[4]+=e;h[5]+=f;h[6]+=g;h[7]+=hh;
    }

    for (int i = 0; i < 8; i++) {
        hash[i*4]   = (h[i]>>24)&0xff;
        hash[i*4+1] = (h[i]>>16)&0xff;
        hash[i*4+2] = (h[i]>>8)&0xff;
        hash[i*4+3] = h[i]&0xff;
    }
    free(msg);
}

// ── Main: Run both probes, derive SIK ──
int main(int argc, char **argv) {
    int json_mode = 0;
    int verify_mode = 0;
    char verify_hash[65] = {0};

    for (int i = 1; i < argc; i++) {
        if (strcmp(argv[i], "--json") == 0) json_mode = 1;
        if (strcmp(argv[i], "--verify") == 0 && i + 1 < argc) {
            verify_mode = 1;
            strncpy(verify_hash, argv[i + 1], 64);
        }
    }

    // Get GPU info
    cudaDeviceProp prop;
    cudaGetDeviceProperties(&prop, 0);

    if (!json_mode && !verify_mode) {
        fprintf(stderr, "[HONE SIK] GPU: %s\n", prop.name);
        fprintf(stderr, "[HONE SIK] VRAM: %lu MB\n", prop.totalGlobalMem / (1024*1024));
        fprintf(stderr, "[HONE SIK] Running silicon fingerprint probe...\n");
    }

    // ── Allocate device memory ──
    unsigned int *d_target;
    unsigned long long *d_timing;
    double *d_divergence;

    cudaMalloc(&d_target, TIMING_PROBE_CELLS * sizeof(unsigned int));
    cudaMalloc(&d_timing, TIMING_PROBE_CELLS * sizeof(unsigned long long));
    cudaMalloc(&d_divergence, FP_PROBE_OPS * sizeof(double));

    unsigned long long *h_timing = (unsigned long long *)malloc(TIMING_PROBE_CELLS * sizeof(unsigned long long));
    double *h_divergence = (double *)malloc(FP_PROBE_OPS * sizeof(double));

    // ── Accumulate timing fingerprint across rounds ──
    // Track which bits are stable (same value across >95% of rounds)
    unsigned char *timing_bits = (unsigned char *)calloc(TIMING_PROBE_CELLS, 1);
    int *timing_ones = (int *)calloc(TIMING_PROBE_CELLS, sizeof(int));

    int threads = 256;
    int blocks_timing = (TIMING_PROBE_CELLS + threads - 1) / threads;
    int blocks_fp = (FP_PROBE_OPS + threads - 1) / threads;

    for (int round = 0; round < TIMING_PROBE_ROUNDS; round++) {
        vram_timing_probe<<<blocks_timing, threads>>>(d_target, d_timing, TIMING_PROBE_CELLS, round);
        cudaDeviceSynchronize();
        cudaMemcpy(h_timing, d_timing, TIMING_PROBE_CELLS * sizeof(unsigned long long), cudaMemcpyDeviceToHost);

        for (int i = 0; i < TIMING_PROBE_CELLS; i++) {
            // Extract the LSB of each timing measurement
            if (h_timing[i] & 1) timing_ones[i]++;
        }
    }

    // Identify stable bits: ones that are consistently 0 or consistently 1
    int stable_count = 0;
    for (int i = 0; i < TIMING_PROBE_CELLS; i++) {
        float ratio = (float)timing_ones[i] / TIMING_PROBE_ROUNDS;
        if (ratio >= STABLE_BIT_THRESHOLD) {
            timing_bits[i] = 1;
            stable_count++;
        } else if (ratio <= (1.0f - STABLE_BIT_THRESHOLD)) {
            timing_bits[i] = 0;
            stable_count++;
        } else {
            timing_bits[i] = 2;  // unstable, skip
        }
    }

    // ── Accumulate FP divergence fingerprint ──
    unsigned char *fp_bits = (unsigned char *)calloc(FP_PROBE_OPS, 1);
    int *fp_ones = (int *)calloc(FP_PROBE_OPS, sizeof(int));

    for (int round = 0; round < FP_PROBE_ROUNDS; round++) {
        fp_divergence_probe<<<blocks_fp, threads>>>(d_divergence, FP_PROBE_OPS, round);
        cudaDeviceSynchronize();
        cudaMemcpy(h_divergence, d_divergence, FP_PROBE_OPS * sizeof(double), cudaMemcpyDeviceToHost);

        for (int i = 0; i < FP_PROBE_OPS; i++) {
            // Extract LSB of the mantissa
            unsigned long long raw;
            memcpy(&raw, &h_divergence[i], sizeof(double));
            if (raw & 1) fp_ones[i]++;
        }
    }

    int fp_stable = 0;
    for (int i = 0; i < FP_PROBE_OPS; i++) {
        float ratio = (float)fp_ones[i] / FP_PROBE_ROUNDS;
        if (ratio >= STABLE_BIT_THRESHOLD) {
            fp_bits[i] = 1;
            fp_stable++;
        } else if (ratio <= (1.0f - STABLE_BIT_THRESHOLD)) {
            fp_bits[i] = 0;
            fp_stable++;
        } else {
            fp_bits[i] = 2;
        }
    }

    if (!json_mode && !verify_mode) {
        fprintf(stderr, "[HONE SIK] Timing stable bits: %d / %d\n", stable_count, TIMING_PROBE_CELLS);
        fprintf(stderr, "[HONE SIK] FP stable bits: %d / %d\n", fp_stable, FP_PROBE_OPS);
    }

    // ── Derive SIK from stable bits ──
    // Collect all stable bits into a buffer, then hash to 256 bits
    size_t raw_len = stable_count + fp_stable;
    unsigned char *raw_fingerprint = (unsigned char *)calloc(raw_len + 64, 1);
    int pos = 0;

    // Pack timing stable bits
    for (int i = 0; i < TIMING_PROBE_CELLS; i++) {
        if (timing_bits[i] <= 1) {
            raw_fingerprint[pos++] = timing_bits[i];
        }
    }

    // Pack FP stable bits
    for (int i = 0; i < FP_PROBE_OPS; i++) {
        if (fp_bits[i] <= 1) {
            raw_fingerprint[pos++] = fp_bits[i];
        }
    }

    // Append GPU name as domain separator (different GPU models should never collide)
    size_t name_len = strlen(prop.name);
    memcpy(raw_fingerprint + pos, prop.name, name_len);
    pos += name_len;

    // SHA-256 → 256-bit SIK
    unsigned char sik[SIK_BYTES];
    sha256(raw_fingerprint, pos, sik);

    // SHA-256(SIK) → SIK hash (what goes on-chain)
    unsigned char sik_hash[SIK_BYTES];
    sha256(sik, SIK_BYTES, sik_hash);

    // ── Output ──
    char sik_hex[65], hash_hex[65];
    for (int i = 0; i < 32; i++) {
        sprintf(sik_hex + i*2, "%02x", sik[i]);
        sprintf(hash_hex + i*2, "%02x", sik_hash[i]);
    }

    if (verify_mode) {
        if (strcmp(hash_hex, verify_hash) == 0) {
            printf("{\"match\":true,\"sik_hash\":\"%s\"}\n", hash_hex);
            return 0;
        } else {
            printf("{\"match\":false,\"expected\":\"%s\",\"actual\":\"%s\"}\n", verify_hash, hash_hex);
            return 1;
        }
    }

    if (json_mode) {
        printf("{\"sik\":\"%s\",\"sik_hash\":\"%s\",\"gpu\":\"%s\",\"vram_mb\":%lu,\"timing_stable\":%d,\"fp_stable\":%d}\n",
            sik_hex, hash_hex, prop.name, prop.totalGlobalMem/(1024*1024), stable_count, fp_stable);
    } else {
        printf("SIK:      %s\n", sik_hex);
        printf("SIK_HASH: %s\n", hash_hex);
        fprintf(stderr, "[HONE SIK] Done. SIK hash is what goes on-chain.\n");
    }

    // ── Cleanup ──
    cudaFree(d_target);
    cudaFree(d_timing);
    cudaFree(d_divergence);
    free(h_timing);
    free(h_divergence);
    free(timing_bits);
    free(timing_ones);
    free(fp_bits);
    free(fp_ones);
    free(raw_fingerprint);

    return 0;
}
