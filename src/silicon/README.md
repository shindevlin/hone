# BTCPC Proof of Silicon — Inference Privacy Architecture

## How SIK Protects Inference Privacy

### The Problem
When a user sends an inference request to a node, the prompt must eventually be processed by the GPU. How do we prevent the node operator from reading it?

### The Solution: SIK-Bound Encryption

```
USER                          NODE (GPU with SIK)
─────                         ──────────────────
                              SIK = f(physical GPU silicon)
                              sik_hash published on-chain

1. User fetches node's
   sik_hash from chain

2. User encrypts prompt:
   session_id = random()
   encrypted_prompt =
     encrypt(prompt,
       KDF(node_public_key + sik_hash + session_id))
                    ──────→
                              3. Node re-derives SIK from GPU
                                 (runs timing + FP probes)

                              4. Derives same session key:
                                 key = KDF(node_private_key + SIK + session_id)

                              5. Decrypts prompt INSIDE locked process:
                                 - mlock() pinned memory
                                 - seccomp blocks ptrace
                                 - MADV_DONTDUMP

                              6. Tokenizes → remaps token IDs → GPU
                                 GPU VRAM contains ONLY remapped integers

                              7. GPU runs inference on remapped tokens

                              8. Locked process de-remaps → encrypts result
                                 with user's public key

                              9. Wipes: session key, plaintext, remap table
                    ←──────
9. User decrypts result
   with their private key
```

### Why This Works

**Without SIK:** An attacker copies the node's software + keys to another machine → decrypts all prompts. The private key alone is sufficient.

**With SIK:** The decryption key requires `KDF(private_key + SIK)`. The SIK can only be derived from the physical GPU. Copy the software to another machine → different GPU → different SIK → wrong key → cannot decrypt.

**Attack scenarios:**

| Attack | Result |
|--------|--------|
| Clone disk to another machine | Can't decrypt — wrong GPU, wrong SIK |
| Read GPU VRAM during inference | See remapped token IDs (meaningless integers) |
| Attach debugger to process | Blocked by seccomp + ptrace prevention |
| Read /proc/pid/mem | Blocked by MADV_DONTDUMP + permissions |
| Modify the binary to log prompts | TPM attestation fails → network rejects node |
| Swap GPU after registration | SIK challenge fails → slashed |
| Custom kernel module | Can bypass software protections — but requires physical access + expertise + risk of full stake slash |

### Compilation

```bash
cd src/silicon
nvcc -O2 -o btcpc-sik fingerprint.cu
```

### Usage

```bash
# Get fingerprint
./btcpc-sik --json

# Verify against registered hash
./btcpc-sik --verify <sik_hash>
```

### Node.js API

```javascript
const silicon = require('./src/silicon');

// Get fingerprint (SIK never leaves this machine)
const fp = await silicon.getFingerprint();
console.log(fp.sik_hash);  // This goes on-chain

// Derive session key for decrypting an inference request
const key = await silicon.deriveSessionKey(requestId);

// Verify this GPU matches a registered hash
const result = await silicon.verify(registeredHash);
```
