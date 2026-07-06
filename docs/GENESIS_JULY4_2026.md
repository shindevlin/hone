---
title: HONE Genesis — July 4th, 2026 (Freedom Tech Relaunch)
description: Plan for review — why we reset the chain, the recoverable-keystore fix, and the wallet recreation for every project
author: Shin Devlin
status: DRAFT — for review before any build
supersedes_genesis: 1783191600000 (May 1 2026, "Mayday")
---

# HONE Genesis — July 4th, 2026

> **This is a plan for you to review. Nothing is built or changed yet.** It lays
> out *what* we do, *why*, and *in what order*. Approve it (or edit it) and then
> we build against it.

---

## 1. Why reset the chain

The current chain (genesis `1783191600000`, May 1 2026) carries a **fatal,
unfixable-in-place flaw**: **wallets were created without any recoverable key
storage.**

- The SDK's `Wallet::save_to_file` writes **only public keys** — by design:
  *"No private key, no mnemonic, no seed — those must be kept by the user."*
  (`rust/hone-sdk/src/lib.rs:1445`).
- The mnemonic was shown once (if at all) and discarded. In practice it was
  **never durably delivered** — confirmed: not on either PC, not in Telegram,
  not in Signal.
- Result: **we own accounts we cannot sign for.** `bullship` is the concrete
  example — the account exists, is funded, but its private key is gone. This has
  blocked real work repeatedly.

This is not a bug we patch and move on from. Every account minted under the old
flow is potentially unrecoverable. The honest fix is a **clean genesis** where
**no account can ever be created without a recoverable, encrypted key file** —
and we recreate the accounts we actually operate, this time keeping the keys.

The chain is pre-mainnet / testnet-live, so a reset costs us nothing real and
fixes the deepest structural gap we have.

## 2. Why July 4th, 2026

America's **250th anniversary** (Semiquincentennial). HONE is freedom tech for a
freedom-based country — a sovereign chain, self-custody, no gatekeepers. Anchoring
genesis to July 4 2026 makes the mission the launch.

- **New genesis timestamp: `1783191600000`** = 2026-07-04 12:00:00 PDT (noon Los Angeles)
  (19:00 UTC). Noon Pacific (Los Angeles) on Independence Day.
- Retires the "Mayday" / May 1 anchor (`1783191600000`).
- 64 days after the old genesis — a real, deliberate relaunch, not a slip.

*(Locked to noon Pacific / Los Angeles — Shin's call.)*

## 3. The core fix — recoverable keystore (best-of-all design)

Every wallet creation produces a **portable, encrypted, recoverable key file**,
and the user is shown their recovery phrase once. Three layers of recoverability,
so a single lost thing never means a lost account:

### Layer 1 — Encrypted keystore file (primary)
- `<account>.keystore.json` — the mnemonic/seed encrypted at rest.
- **KDF: Argon2id** (memory-hard, brute-force resistant). *Not* the node's
  existing SHA-256 `derive_key` — that is fine for a hardware-derived key but
  **too weak for a human password.** This is the one place we must not reuse the
  old pattern.
- **Cipher: AES-256-GCM** (authenticated) — same cipher family as
  `secret_store.rs`, so it's already a vetted dependency.
- **Password set by the user at creation.** The password never leaves the device;
  only the ciphertext is ever written or transmitted.
- File format modeled on the battle-tested **Ethereum keystore V3** shape
  (kdf params, salt, iv, ciphertext, mac) so it's inspectable and standard.

### Layer 2 — Recovery phrase, shown once with a confirmation gate
- At creation, the **12/24-word BIP39 mnemonic is displayed**, and the user must
  confirm they've written it down (re-enter a few words) before proceeding.
- This is the ultimate offline backup — recovers the account even with no file.
- Fixes the exact failure we hit: the phrase is **actively delivered**, not
  silently assumed.

### Layer 3 — Optional encrypted relay backup (opt-in)
- User may opt to upload the **encrypted keystore blob** (ciphertext only) to a
  HONE relay/service, so the account survives losing the local file.
- The password never leaves the device; the relay only ever holds ciphertext it
  cannot decrypt.
- Off by default — sovereignty first; convenience is a choice.

### What changes in code
- `Wallet::save_to_file` gains an **encrypted variant**
  (`save_keystore(path, password)`) and a matching `load_keystore(path, password)`.
- The public-only `save_to_file` stays (for publishing identity), but the wallet
  **creation flow always writes a keystore too** — you cannot create a wallet
  that leaves no recoverable file.
- New deps in `hone-sdk`: `argon2`, `aes-gcm` (already used by the node).

## 4. Wallets to recreate (kept locally, this time)

Every account we operate gets recreated under the new flow, with its
`<account>.keystore.json` **saved locally on this machine** (in a dedicated,
gitignored `wallets/` vault) so you can always refer back to it. Proposed set:

| Account | Purpose | Notes |
|---|---|---|
| `shindevlin` | **Protocol founder / root owner** | HIGHEST value — the whitepaper makes this account the owner of `freeport`, `verasens`, `linkgit` (holds their seed phrases). If any wallet must be recoverable, it is this one. All three backup layers on. |
| `natoshisakamoto` | The node / founder account | Node signing key; highest value — keystore + phrase + relay backup all on. |
| `bullship` | Bullship inference billing | The account this whole thread needed. Fresh key, recoverable. You have its **Hive** key already — this is its **HONE** key. |
| `__treasury__`, `__recycle_fund__`, `__testnet_fund__` | System funds | Defined in genesis, not user wallets — no keystore, but documented. |
| `freeport` | Freeport marketplace service | Per-vertical service account. Owner: `shindevlin`. |
| `linkgit` | LinkGit identity layer | Per-vertical service account. Owner: `shindevlin`. |
| `verasens` | Verasens sensor layer | Per-vertical service account. Owner: `shindevlin`. |
| `hone-market`, `hone-relay` | Service accounts | As needed by each running service. |
| (bots) `honebot`, `honewalletbot` | Telegram bots | If they hold accounts. |

**Deliverable:** a local `wallets/` directory with one `*.keystore.json` per
account + an index (`wallets/INDEX.md`, public info only — account name,
pubkeys, which layers are backed up). The mnemonics are shown to you once each
and stored **only** inside the encrypted keystores. Nothing secret is committed
to git (the vault is gitignored).

> On the current 1,194 on-chain accounts: the vast majority are pre-seeded name
> reservations / placeholders, not wallets we own. They do not survive genesis
> and do not need recreation. Only the operated accounts above matter.

## 5. Whitepaper update

`docs/HONE_WHITEPAPER.md` updated to reflect:
- New genesis (July 4 2026, 250th anniversary) and the freedom-tech framing.
- The recoverable-keystore model as a **first-class sovereignty guarantee**:
  self-custody with real recovery, no silent key loss.
- Any constants that reference the old genesis.

## 6. Execution order (after you approve this plan)

1. **Keystore engine** — Argon2id + AES-256-GCM `save_keystore`/`load_keystore`
   in `hone-sdk`, with tests (round-trip, wrong-password rejection, tamper
   detection). *No chain changes yet.*
2. **Wallet-creation flow** — make keystore-writing mandatory; add the
   mnemonic-shown-once confirmation gate; add `hone wallet new` / `import` /
   `unlock` commands.
3. **Local wallet vault** — recreate the operated accounts (§4), write their
   keystores into a gitignored `wallets/`, produce `wallets/INDEX.md`.
4. **Genesis rebuild** — new `genesis.json` with ts `1783191600000`, the new
   account set, system funds, and the constant updated in `config.rs` +
   `CHAIN_CONSTANTS.md` (+ the CI constant-drift gate).
5. **Whitepaper + docs** — §5 updates; retire "Mayday" references or reframe.
6. **Relay backup (Layer 3)** — opt-in encrypted-blob backup endpoint. Can land
   after 1–4 without blocking launch.
7. **Cutover** — stop the old node, launch the new genesis on July 4 2026.

## 7. Decisions locked (from your direction)
- Reset to a **fresh genesis** — every account recoverable from day one.
- **July 4 2026**, freedom-tech framing, whitepaper updated.
- **Every wallet gets a local encrypted file** kept here for reference.
- **Recreate wallets for every project** we've built.
- Keystore = **all three layers** (Argon2id file + shown phrase + optional relay).

## 8. Open questions for your review
1. Genesis time: **noon EDT (recommended)** vs midnight EDT?
2. Any accounts missing from the §4 list that you want wallets for?
3. Should the local `wallets/` vault live in the hone repo (gitignored) or a
   separate path you name?
4. Keep chain IDs (`hone` mainnet / `hone-testnet` testnet), or new IDs to
   cleanly separate v2 from v1?
