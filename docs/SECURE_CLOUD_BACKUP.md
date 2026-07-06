# Secure Cloud Backup for HONE

_Added 2026-07-05. How to back up node state, keystores, reports, and secrets to the
cloud **without leaking key material**. Read this before pushing anything sensitive
off-machine._

---

## 0. The one rule

**Encrypt BEFORE it leaves the machine. The cloud only ever sees ciphertext.**

Never rely on "the provider encrypts at rest" — that means the *provider* holds the key
and can read your data (and so can anyone who compromises the provider or your provider
login). For a chain that controls real value, the backup must be encrypted with a key
**you** hold, on **your** machine, so the cloud stores an opaque blob.

---

## 1. Classify what you're backing up (sensitivity tiers)

Different assets need different handling. Do NOT treat them the same.

| Asset | Where | Sensitivity | Backup approach |
|---|---|---|---|
| **BIP-39 mnemonics / raw private keys** | vault zip, `*.keystore.json` secrets | 🔴 CRITICAL — controls funds | Encrypt with a **separate strong passphrase**; ideally also offline (paper/hardware). Cloud copy must be double-encrypted. |
| **Encrypted keystores** (`*.keystore.json`) | wallet dirs | 🟠 already encrypted (Argon2id+AES-GCM) | Safe-ish to store as-is, but still wrap in an outer encryption layer. The Argon2id password is NOT in the file. |
| **RocksDB chain state** (`/var/lib/hone/…`) | node data dir | 🟡 public data, but large & rebuildable | Encrypt (contains no secrets, but avoids leaking peer IPs / timing). Or skip — it re-syncs from peers. |
| **node.env / systemd env** | config | 🔴 CRITICAL — holds `HONE_*` secrets, tokens, mnemonics if present | Encrypt. NEVER commit to git. |
| **Fork/health reports** | `reports/` | 🟢 low, but may contain private node IPs | Scrub IPs or encrypt. |
| **Bot tokens / API keys** | `.env` files | 🔴 CRITICAL | Encrypt; never git (already .gitignored). |

**Rule of thumb:** anything in the 🔴 row, a single plaintext copy in the cloud is a
catastrophic-loss event waiting to happen. The 🟡 chain state is the *least* sensitive
(it's public ledger data and re-syncable) — back it up for fast recovery, not secrecy.

---

## 2. The mechanism: `age` (recommended) or GPG

Use **[`age`](https://github.com/FiloSottile/age)** — modern, small, single-binary,
audited, and **already used by the two-PC comm channel's git-tier secrets** (`channel
secret` uses age), so it's already in your toolchain.

### Encrypt with a passphrase (simplest, symmetric)
```bash
# Encrypt a backup tarball with a passphrase (you'll be prompted).
tar czf - /var/lib/hone/keystores | age -p > hone-keystores-$(date +%F).tar.gz.age

# Decrypt (prompts for the same passphrase)
age -d hone-keystores-2026-07-05.tar.gz.age | tar xzf -
```

### Encrypt to a key (better for automation — no passphrase in scripts)
```bash
# One-time: generate a keypair. Keep the PRIVATE key OFFLINE (this is the recovery key).
age-keygen -o hone-backup.key            # prints the public key: age1....
# Store hone-backup.key OFFLINE (hardware/paper). Put ONLY the public key on the backup host.

# Encrypt to the public key (no secret needed on the machine doing backups):
tar czf - /var/lib/hone | age -r age1YOURPUBLICKEY > hone-state-$(date +%F).tar.age

# Decrypt later on a trusted machine that has the private key:
age -d -i hone-backup.key hone-state-2026-07-05.tar.age | tar xzf -
```

> The public-key approach is best for a scheduled backup: the backup machine can encrypt
> but **cannot decrypt** — so even if the backup host is compromised, the attacker gets
> ciphertext they can't open. The private key lives offline.

---

## 3. Where to put the ciphertext (cloud targets)

Once it's an `.age` blob, the destination barely matters (it's opaque). Pick by
convenience + a second provider for redundancy:

- **Backblaze B2 / AWS S3 / Cloudflare R2** — cheap object storage; use `rclone`.
  You already own a Cloudflare account (playhoy@) — **R2** is a natural fit and keeps it
  in an account you control. R2 has no egress fees.
- **rclone** ties it together and can talk to all of them:
  ```bash
  rclone copy hone-state-2026-07-05.tar.age r2:hone-backups/
  ```
- **A second, different provider** for the 🔴 critical tier (don't put your only key
  backup on the same account as everything else). E.g. state → R2, keystore-blob → B2.

> **Do NOT** back up to a provider using an account whose recovery you can't guarantee,
> and **do NOT** store the decryption key in the same cloud as the ciphertext. That
> defeats the entire scheme.

---

## 4. Recommended layout

```
Tier 🔴 (mnemonics, node.env, bot tokens):
  - age -p (strong unique passphrase) OR age -r (offline private key)
  - stored on TWO providers
  - the mnemonic ALSO offline (paper/steel), never cloud-only
  - passphrase in a password manager (the user's own PW manager), NOT in any script

Tier 🟡 (RocksDB state):
  - age -r (public key) so scheduled backups can't decrypt themselves
  - one provider (R2) is fine; it's re-syncable anyway
  - nightly, keep ~7 dated snapshots, prune older

Tier 🟢 (reports):
  - scrub private IPs first, or age-encrypt; low urgency
```

---

## 5. A concrete scheduled routine (sketch)

`scripts/cloud-backup.sh` (to be built — a natural companion to `update.sh`):
```bash
#!/usr/bin/env bash
set -euo pipefail
DATE=$(date +%F)
PUBKEY="age1YOURPUBLICKEY"          # public key only — safe to have on the host
DEST="r2:hone-backups"

# 1. Encrypt chain state to the public key (host cannot decrypt its own backup)
tar czf - /var/lib/hone/db | age -r "$PUBKEY" > "/tmp/hone-state-$DATE.tar.age"

# 2. Encrypt keystores (already Argon2id-encrypted; this is the OUTER layer)
tar czf - /var/lib/hone/keystores | age -r "$PUBKEY" > "/tmp/hone-keystores-$DATE.tar.age"

# 3. Ship ciphertext to the cloud
rclone copy "/tmp/hone-state-$DATE.tar.age"     "$DEST/state/"
rclone copy "/tmp/hone-keystores-$DATE.tar.age" "$DEST/keystores/"

# 4. Wipe local temp
shred -u "/tmp/hone-state-$DATE.tar.age" "/tmp/hone-keystores-$DATE.tar.age"

# 5. Prune remote (keep last 7 state snapshots)
rclone delete --min-age 7d "$DEST/state/"
```
Wire to a `hone-backup.timer` (daily). **The `node.env` / mnemonic tier is handled
separately and manually** — do NOT automate shipping raw mnemonics, even encrypted,
without a deliberate one-time setup you control.

---

## 6. Hard don'ts (recap)

- ❌ Never upload a **plaintext** mnemonic, private key, `node.env`, or `.env` anywhere.
- ❌ Never store the **decryption key/passphrase in the same cloud** as the ciphertext.
- ❌ Never `git add` a keystore, `.env`, `node.env`, or a report containing private IPs.
  (`.gitignore` + the `opsec-runner.sh` gitleaks scan guard this — keep them.)
- ❌ Never let a scheduled/automated host hold a key that can **decrypt** its own backups
  (use `age -r <public key>`; keep the private key offline).
- ✅ The chain **state** is public and re-syncable — the thing that actually needs
  protecting is the **keys**. A lost node re-syncs; a lost/leaked mnemonic is permanent.

---

## 7. Relationship to existing work

- The **recoverable keystore** (`hone-sdk/src/keystore.rs`, Argon2id+AES-GCM) already
  encrypts the mnemonic at rest — this doc is the **off-machine, redundant** layer on top.
- The **comm-channel** already uses `age` for its git-tier secrets, so `age` is a known,
  in-toolchain choice.
- Backup is one of the four chain-operations routine gaps identified 2026-07-05
  (alongside epoch-advance monitor, cross-node fork check, supply/solvency audit).
