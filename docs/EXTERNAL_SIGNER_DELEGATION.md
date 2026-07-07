# External Signer Delegation (TON Connect, and other chains)

**Version 0.1 — July 2026**

---

## Position

A HONE holder can opt an **external chain's wallet key** (starting with TON, via
TON Connect) into authorizing HONE transfers directly — no HONE-native signature
required for those transfers, no copy-paste, no separate HONE key to manage day
to day. This is a genuine protocol-level capability: a TON-signed payload becomes
a **valid HONE transfer authorization**, not merely a gate in front of one.

This is deliberately **not** the same thing as [Cross-Chain Identity
Binding](CROSS_CHAIN_IDENTITY_BINDING.md) (`VerifyChainLink`), which proves
*control* of an external wallet for privacy/claim purposes and never grants
spending power. This spec grants spending power, bounded and revocable.

**The root of trust never moves.** The HONE `active` key is always the
authority that creates, bounds, and can instantly kill an external signer grant.
An external key can never bootstrap itself in, expand its own authority, or
survive its own revocation. This is the single design decision that makes the
convenience safe: **a TON wallet compromise costs at most what the active key
allowed it to cost — never more.**

---

## Why this is the right shape (and not a shared-keypair scheme)

The tempting-but-wrong version of this feature is "the TON wallet's key just
*is* a HONE active key" — same key, two chains, no distinction. Rejected,
because:

- HONE's existing role keys (`owner/active/posting/memo/hide/seek`) all derive
  from **one seed** via hardened SLIP-10 specifically so they are mutually
  unlinkable but share a single point of ultimate control (see
  [ADDRESS_SCHEME.md](ADDRESS_SCHEME.md)). An externally-sourced TON key has no
  such ancestry — treating it as interchangeable with a derived `active` key
  would silently change what "compromise the active key" means for every
  holder who never opted in.
- A shared-keypair model has **no natural revocation**: if the TON wallet is
  compromised, lost, or the user just changes their mind, there is no way to
  cut it off without also touching the HONE key it was conflated with.
- It also has no natural scoping: either the TON key can spend everything, or
  the feature doesn't exist. There is no room for "up to 5 HONE/day."

The delegation model fixes all three: **grant → bounded use → unilateral
revoke**, with the `active` key as the only party that can grant or revoke.

---

## The three entries

### 1. `GrantExternalSigner` (active-key authorized)

```
GrantExternalSigner {
  account:            String,        // the HONE account granting the capability
  external_chain:     String,        // "ton" (extensible to others later)
  external_pubkey:     Bytes,        // the TON wallet's Ed25519 pubkey
  label:              Option<String>, // holder-chosen name, e.g. "Tonkeeper — daily driver"
                                       // shown in every notification (§ Attribution)
  caps: {
    per_tx_max:   Option<Cap>,       // null = no per-tx cap
    daily_max:    Option<Cap>,       // null = no daily cap
    allowlist:    Option<Vec<String>>, // null = any recipient
    expires_at:   Option<u64>,       // null = no expiry
  },
  nonce:              u64,           // replay guard, monotonic per (account, external_pubkey)
  signature:          Bytes,         // signed by the account's HONE `active` key
}

// A cap is denominated in EITHER hunits OR USD — the holder picks per cap,
// at grant time. This is the honest tradeoff: HONE's price moves, so "50
// HONE/day" and "$24/day" are NOT the same promise over time.
enum Cap {
  Hunits(u64),   // fixed HONE amount — no oracle dependency, but the real-value
                 // risk drifts as HONE's price changes; holder re-grants to adjust.
  Usd { cents: u64, max_staleness_s: u64 },
                 // converted to hunits AT SIGNING TIME via HONE's existing
                 // price-oracle path (the same BtcHeightReport-style oracle
                 // median already on-chain — see CHAIN_CONSTANTS.md). Tracks
                 // real risk as price moves, but the check now depends on
                 // oracle freshness: if the most recent price sample is older
                 // than max_staleness_s, the transfer is REJECTED (fail closed,
                 // never fail open on a stale/missing price).
}
```

- Authorized **only** by the HONE `active` key — the same tier that already
  gates high-consequence entries. No new trust tier is introduced on the HONE
  side; this is an `active`-key operation, full stop.
- `caps` are chosen **entirely by the user, through their active key** — this
  spec does not mandate a cap model or a default. A holder may grant an
  uncapped signer if they choose to; the point is that it is *their* choice,
  made once, explicitly, by the key that already controls the account — not a
  platform-imposed limit and not something the external key can set for
  itself.
- **Denomination is per-cap, chosen at grant time.** `Hunits` needs no oracle
  and is the simpler, lower-trust-surface default. `Usd` is what a holder
  reaches for when they want a durable "$24/day" promise that survives price
  swings — it costs an oracle-freshness dependency on a security check, made
  explicit and fail-closed (§ above) rather than silently trusted.
- The node stores the grant keyed by `(account, external_chain, external_pubkey)`.
  A later grant for the same triple **replaces** the caps (still active-key
  authorized) — there is no way for the external key to modify its own caps.

### 2. `ExternalSignerTransfer` (external-key authorized, within caps)

```
ExternalSignerTransfer {
  account:            String,        // must have an active, non-expired grant
  external_chain:     String,        // must match a granted chain
  to:                 String,        // recipient (hh1… or name)
  amount_hunits:      u64,
  memo:               Option<String>,
  nonce:              u64,           // replay guard
  ton_signature_envelope: Bytes,     // the TON Connect signature, in TON's native
                                      // sign_data/transaction envelope format —
                                      // NOT a bare HONE-canonical-message signature
}
```

- Verification path: recover the TON wallet's pubkey from
  `ton_signature_envelope` per **TON's own signature format** (this is the one
  genuinely new verifier the node needs — HONE does not otherwise parse a
  foreign chain's signature envelope). Confirm it matches the granted
  `external_pubkey` for `account`. Then check, **in order**:
  1. grant exists, is not expired, is not revoked
  2. for each cap that is `Usd`: fetch the current price-oracle median; if its
     freshness exceeds `max_staleness_s`, **reject the entire transfer** —
     stale/missing price never fails open into "treat as uncapped"
  3. `amount_hunits <= per_tx_max` (converted to hunits at check time if `Usd`; if set)
  4. running daily total for this grant `+ amount_hunits <= daily_max` (converted to
     hunits at check time if `Usd`; if set — the running total accumulates in
     hunits regardless of the cap's denomination, so a `Usd` cap's *hunit*
     budget can drift intra-day with price; this is disclosed, not hidden)
  5. `to` is in `allowlist` (if set)
  6. `nonce` has not been used for this grant (replay guard)
- Any failed check → entry rejected, same as any other invalid entry. No
  partial application.
- This is the entry the Mini App (or any thin client) submits when the user
  taps "send" and approves in their TON wallet — no HONE-native signature, no
  local key custody in the webview, fully compliant with Telegram's TON-Connect
  policy (see [ANDROID_WORLDCLASS_PLAN.md §7b](ANDROID_WORLDCLASS_PLAN.md)).

### 3. `RevokeExternalSigner` (active-key authorized, instant)

```
RevokeExternalSigner {
  account:            String,
  external_chain:     String,
  external_pubkey:     Bytes,
  nonce:              u64,
  signature:          Bytes,         // signed by the account's HONE `active` key
}
```

- **Only** the `active` key can produce this. There is no external-key path to
  revoke or modify its own grant — by construction, a compromised TON key
  cannot protect itself from being cut off.
- Takes effect at the epoch it's sealed in — any `ExternalSignerTransfer`
  using the revoked pubkey in a later epoch is rejected. (Same-epoch races
  resolve by the existing sha256-order `drain_pending_sorted` rule, same as
  every other entry type — no special-casing.)
- Revocation is **immediate and total** — there is no "grace period" during
  which a revoked key still works. If the user wants a cool-down before a
  grant takes effect, that is a `caps.expires_at`/re-grant pattern on the way
  *in*, not a soft-revoke on the way *out*.

---

## Attribution & compromise notification

A holder who sees an unexpected transfer needs to know **which key produced
it** in seconds, not after digging through raw entries — that speed is what
turns "a TON wallet got compromised" into "I revoked it before the second
transfer went out" instead of a drained account.

**On-chain: every transfer is unmistakably labeled.** `ExternalSignerTransfer`
is a **distinct entry type** from a native HONE `Transfer` — it can never be
confused with an active-key-signed send by anything reading chain history. The
entry (and therefore every explorer, wallet, and notification built on it)
always carries:
- `external_chain` + `external_pubkey` (which grant authorized this)
- the grant's `label`, if the holder set one (e.g. *"Tonkeeper — daily
  driver"*) — carried through so a notification can say the *human* name, not
  a raw pubkey the holder has to recognize under pressure
- this is true for every `ExternalSignerTransfer`, not just anomalous ones —
  attribution is not an incident-response feature bolted on after the fact, it
  is a property of the entry type itself

**Push: the holder is told immediately, not on next login.** Every applied
`ExternalSignerTransfer` triggers a push notification (via the existing
node → client channel — the same path the native app's `NodeService`
foreground notification and the Telegram bot already use) to every surface the
holder has registered, with:

> **HONE sent via "Tonkeeper — daily driver"**
> 12.5 HONE → @merchant · epoch 91442
> Not you? [**Revoke this signer now**]

- The **[Revoke this signer now]** action is a **pre-built, one-tap**
  `RevokeExternalSigner` — the notification itself carries enough to construct
  it (account, chain, pubkey), so revocation is one tap away from the alert
  that prompted it, on whichever surface (native app, Mini App, Telegram bot)
  the holder sees first. It still requires the `active` key to actually sign —
  the notification shortcuts the *path* to revoking, it does not weaken *who*
  can revoke (§3 still holds: only the active key, always).
- This applies per-transfer, not just above some anomaly threshold — a holder
  who granted a signer and expects regular small transfers still gets told
  each time, so "this doesn't look like my usual pattern" is a judgment the
  *holder* makes, not one the protocol tries to make for them by filtering
  which transfers are worth mentioning.
- Delivery is best-effort (push can fail, a client can be offline) — the
  **on-chain label is the durable, always-available record**; push is the
  fast path on top of it, not a replacement for it.

---

## Opt-in flow

```
Holder (active key)                  TON Wallet                    Chain
   │                                     │                            │
   │ 1. decide: grant TON wallet X       │                            │
   │    signing power, caps C            │                            │
   │ 2. sign GrantExternalSigner          │                            │
   │    with HONE active key ────────────┼───────────────────────────▶ verify active-key
   │                                     │                            │   sig, store grant
   │                                     │                            │
   │           ... later, a transfer ... │                            │
   │ 3. Mini App builds unsigned         │                            │
   │    ExternalSignerTransfer            │                            │
   │ 4. hand to TON Connect ─────────────▶ 5. user approves in        │
   │                                     │    THEIR TON wallet app     │
   │                                     │    (never in the webview)   │
   │                                     │ 6. signed envelope ─────────┼──▶ verify vs grant
   │                                     │    returned to Mini App     │    + caps, apply
   │◀────────────────────────────────────┼──────────────────────────── │
   │ 7. sees confirmed tx                │                            │
```

Step 4/5 is exactly the TON Connect delegated-signing pattern already
established for TON dApps — the Mini App never touches the TON private key,
mirroring how it must never touch a HONE key either (see
[ANDROID_WORLDCLASS_PLAN.md §7b](ANDROID_WORLDCLASS_PLAN.md)). This spec adds
no new custody risk to the Mini App; it only defines how the *chain* accepts
the result of that existing, compliant pattern as a valid HONE authorization.

---

## What this does NOT do

- It does **not** make the TON key equivalent to a HONE `owner`/`active` key
  for anything other than the `ExternalSignerTransfer` entry — it cannot vote,
  cannot bind other chains, cannot grant/revoke other signers, cannot change
  account policy. Its entire power is "spend, within caps, until revoked."
- It does **not** change genesis, the address scheme, or existing role-key
  derivation. No re-genesis, no re-smoke, no new HONE key material.
- It does **not** apply to founder/triumvirate wallets by default or
  automatically — per the standing rule, any token transfer must have an
  explicit request to Shin or a triumvirate founder wallet via sign-request;
  an `ExternalSignerTransfer` from a founder account is still subject to that
  rule at the caps/allowlist the founders set (e.g. an allowlist of zero
  external recipients, or a per-tx cap of zero, effectively disables it for
  wallets where founders want signing to stay manual).
- It does **not** (v0.1) support chains other than TON — the `external_chain`
  field is a string specifically so this generalizes later (e.g. a
  Ledger-connected EVM key), but only the TON verifier is specified/built here.

---

## Relationship to other specs

- **[Cross-Chain Identity Binding](CROSS_CHAIN_IDENTITY_BINDING.md)** — a
  *different* feature. `VerifyChainLink` proves control for privacy/claim
  purposes and grants no spending power; this spec grants spending power and
  is unrelated to the commitment/claim-ledger mechanism. A holder may use
  either, both, or neither independently.
- **[Sign-Request Protocol](SIGN_REQUEST_PROTOCOL.md)** — `GrantExternalSigner`
  and `RevokeExternalSigner` are active-key operations and can be routed
  through the existing sign-request flow the same as any other sensitive
  active-key action (agent proposes, holder disposes).
- **[Android world-class plan §7b](ANDROID_WORLDCLASS_PLAN.md)** — this is the
  concrete mechanism behind "the Mini App can send, it just never signs
  locally": `ExternalSignerTransfer` via TON Connect is the compliant path.
- **2FA (`SetKeyPolicy`)** — conceptually a sibling: both are ways the
  `active` key configures a bounded capability on top of the account without
  changing the root key. A future version could let `SetKeyPolicy` require
  2FA even for `ExternalSignerTransfer` within caps, for holders who want an
  extra gate on top.

---

## Build phasing

1. **TON signature envelope verifier.** The one genuinely new crypto surface:
   parse and verify a TON Connect `sign_data`/`transaction` envelope against a
   raw Ed25519 pubkey. Build and test this in isolation before wiring entries.
2. **`GrantExternalSigner` / `RevokeExternalSigner`** entries — active-key
   gated, straightforward given existing entry-signing infrastructure. Ship
   with `Hunits` caps only — no oracle dependency yet.
3. **`ExternalSignerTransfer`** entry, `Hunits`-cap path — per-tx, daily
   running total, allowlist, expiry, nonce replay guard, all in hunits.
4. **On-chain attribution** (§ Attribution) — the entry-type distinction and
   `label` field ship in step 3, not bolted on later; every transfer is
   labeled from the first one that lands.
5. **Push notification path** — wire the applied-`ExternalSignerTransfer` →
   notification → one-tap-revoke flow across native app / Mini App / Telegram
   bot. Depends on step 3's entry existing but is otherwise independent of the
   `Usd`-cap work below.
6. **`Usd`-denominated caps** — wires the existing price-oracle median into
   the caps check, fail-closed on staleness (§2 verification order). Deferred
   behind the `Hunits` path landing first since it adds a dependency
   (oracle freshness) to a security-critical check; ship it once the simpler
   path is proven in production.
7. **Mini App integration** — TON Connect wiring in the Telegram Mini App
   (§7b), building unsigned `ExternalSignerTransfer`s and submitting the
   returned envelope.
8. **Native app parity** — expose grant/revoke (with `label`) in the native
   Kotlin Wallet screen (§4) so a holder can manage external signers from
   either surface.

No genesis change at any phase. No new HONE key material. No re-smoke.

---

_The active key is always the root of trust. An external wallet can only ever
spend what the active key explicitly, revocably allowed it to._
