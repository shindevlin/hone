---
title: HONE Integration Manifest — Self-Updating Consumer Understanding
description: How any repo that uses HONE keeps a current, machine-readable understanding of how to consume the chain
author: Shin Devlin
status: implemented
---

# HONE Integration Manifest

> **Goal:** any repository that uses HONE — Bullship first — should carry a
> **self-updating understanding** of how to make use of HONE. Not a hand-written
> README that rots the moment the API changes, but a generated manifest the repo
> (and the repo's coding agent) can *refresh on demand* and trust to be current.

> **Status: built.** The generator, differ, CI gate, and consumer `sync` are
> implemented in `rust/hone-sdk` (`src/manifest/`, binary `hone`). The
> canonical manifest is committed at the repo root as `hone-manifest.json`
> (188 entries, 307 routes as of HONE 1.2.2). See **§4. Using it today**.

The chain's surface changes: new ledger entries, new routes, new deploy
capabilities (see `SERVICE_HOST_V2_14.md`). A consumer that hard-codes today's
surface breaks silently. This system makes the node the **source of truth** and
gives consumers a one-command refresh.

---

## 1. Two halves

### Half A — the node *emits* a machine-readable capability manifest
A new route on the canonical node:

```
GET /api/integration/manifest        → full manifest (JSON)
GET /api/integration/manifest.md     → same, rendered as Markdown for agents/humans
```

The manifest is **generated from the node's own definitions** — not maintained by
hand — so it can never drift from what the node actually serves:

```jsonc
{
  "hone_manifest_version": 1,
  "chain_id": "hone",
  "node_version": "2.14.0",
  "generated_at_epoch": 1234567,
  "genesis_ts": 1783191600000,

  "endpoints": {
    "http_api": "https://<node>/api",
    "inference": "https://<node>/v1",          // OpenAI-compatible
    "explorer": "https://<node>:4242"
  },

  "inference": {
    "chat_completions": "/v1/chat/completions",
    "models": "/v1/models",
    "pricing": "/v1/pricing",
    "auth": "Bearer <api_key> (optional for public tier)"
  },

  "entry_types": [
    // generated from LedgerEntry (crates/hone-types/src/entry.rs):
    {
      "type": "SensorDataCommit",
      "route": "POST /api/sensor/commit",
      "canonical_signing_fields": ["chain_id","type","sensor_id","owner",
                                   "batch_hash","reading_count","sensor_type","signed_by"],
      "signed_by_bound_to": "owner",
      "server_set_fields": ["epoch","value"]
    }
    // ... one entry per LedgerEntry variant, with its route + signing shape
  ],

  "hosting": {                                  // populated as v2.14 lands
    "runtime_register": "POST /api/runtime/register",
    "app_bundle": "POST /api/bundle/register",  // M3
    "sealed_secret": "POST /api/runtime/secret/grant", // M4
    "ingress": "POST /api/ingress/route",       // M5
    "supports": ["stateless_compute","volumes?","bundles?","sealed_secrets?","ingress?"]
  },

  "invariants": [
    "No local submission without peers — a zero-peer node rejects all submissions.",
    "Every entry is signed; signed_by is bound to the owning account.",
    "The epoch is the block; system entries apply on epoch seal."
  ]
}
```

**Why generated, not written:** the endpoint's handler walks the router's route
table and the `LedgerEntry` enum + `canonical_signing_message` arms to build
`entry_types`. When a new entry or route is added, the manifest updates for free.
A CI check (`manifest_is_current`) fails the build if the committed golden
manifest snapshot diverges from what the code would emit — so the manifest can
never silently rot even for offline consumers.

### Half B — the consumer repo carries a *self-refreshing* pointer
Every repo that uses HONE drops in one small file, `HONE.md` (and, for
agent-driven repos, a line in `AGENTS.md`/`CLAUDE.md` pointing at it):

```markdown
<!-- HONE.md — how THIS repo uses HONE. Self-updating: run the refresh command below. -->
# How <this repo> uses HONE

- HONE node: https://node.honemesh.net
- Refresh this file:  `npx hone-sdk sync-manifest --node https://node.honemesh.net`
  (fetches /api/integration/manifest, regenerates the sections below)
- Last synced: epoch 1234567 (node 2.14.0)

## What we consume
- Inference: /v1/chat/completions (ai-headline, oracle, queue_worker)
- Entries we submit: SensorDataCommit, ...

## Pinned surface (auto-generated — do not edit by hand)
<!-- BEGIN hone:generated -->
... entry signing shapes + routes this repo uses, sliced from the manifest ...
<!-- END hone:generated -->
```

`hone-sdk sync-manifest`:
1. `GET /api/integration/manifest` from the configured node,
2. regenerates the `hone:generated` block (only the entries/endpoints this repo
   declares it uses),
3. bumps "Last synced",
4. exits non-zero if a surface the repo depends on **disappeared or changed
   signing shape** — a loud, early break instead of a silent runtime failure.

Run it in the consumer's CI. Now the repo's understanding of HONE is *tested*
against the live chain surface on every build.

---

## 2. Why this matters for agents

A coding agent working in Bullship's repo currently has to *rediscover* HONE's
API by reading HONE's source (which may not even be checked out). With this:

- The agent reads `HONE.md` → knows the exact routes, signing fields, and
  `signed_by` binding rules **for the node version this repo targets**.
- If the agent is about to submit an entry, it has the canonical signing shape
  verbatim (the same failure mode that bit the Flipper receiver — signing the
  wrong message / hitting a phantom route — is prevented by construction).
- `sync-manifest` in CI means the agent's knowledge and the code's behavior are
  reconciled continuously, not at a point in time.

This is the general fix for the class of bug where a consumer's mental model of
HONE drifts from reality.

---

## 3. Build order

| Step | Deliverable |
|------|-------------|
| S1 | Node route `GET /api/integration/manifest[.md]`, generated from the router + `LedgerEntry` + `canonical_signing_message`. |
| S2 | CI check `manifest_is_current` (golden snapshot vs. generated). |
| S3 | `hone-sdk sync-manifest` command + the `HONE.md` template. |
| S4 | Land `HONE.md` in Bullship (first consumer); wire `sync-manifest` into its CI. |
| S5 | As v2.14 hosting entries land, they appear in `hosting` automatically — Bullship's `HONE.md` picks them up on next sync. |

**S1–S4 are independent of the v2.14 hosting work** and can ship immediately;
they also *document* the hosting surface as it arrives, closing the loop between
`SERVICE_HOST_V2_14.md` and every consumer repo.

---

## 4. Using it today

The system is implemented in `rust/hone-sdk` (module `src/manifest/`, binary
`hone`). Build it once: `cd rust/hone-sdk && cargo build --release --bin hone`.

### For HONE maintainers (in this repo)

```bash
# Regenerate the canonical manifest after changing routes/entries/signing:
hone manifest generate --repo .          # writes hone-manifest.json

# CI runs this — fails if the committed manifest drifted from source:
hone manifest check --repo .             # exit 0 = current, 2 = stale
```

The `.github/workflows/manifest-check.yml` gate enforces that any change to
`api.rs`, `tx.rs`, or `entry.rs` regenerates the manifest in the same commit.
So "what changed in HONE's surface" is always a real `git diff` of
`hone-manifest.json`.

### For consumer repos (Bullship, bots, services)

```bash
# One-time: initialize HONE.md + HONE.lock in your repo.
hone sync --manifest path/to/hone-manifest.json   # or --node https://node.honemesh.net

# In CI, after pulling HONE updates — prints the changelog, exits 2 on breaking:
hone sync --node https://node.honemesh.net
```

`hone sync` regenerates `HONE.md` (human/agent-readable contract + changelog)
and `HONE.lock` (machine baseline). It reports every change as **ADDED /
REMOVED / DEPRECATED / CHANGED** and **exits non-zero on breaking changes** — so
a consumer's CI blocks a deploy that would break against the new surface. List
the entries/routes you use under `uses_entries` / `uses_routes` in `HONE.lock`
to get change alerts scoped to only the surface you depend on.

### The deprecation convention (source-annotated)

To deprecate an entry or route without breaking consumers, annotate the
**source** — the generator lifts it into the manifest, and consumers see a
WARNING (still works, migrate), distinct from a REMOVED (hard break):

```rust
/// @deprecated since=1.3.0 use=SensorDataCommit remove=2.0.0 reason="folded into commit"
/// Legacy single-reading submit. Still accepted; removed in 2.0.
SensorReadingLegacy { .. }
```

All fields optional: `since=`, `use=` (replacement), `remove=` (removal
version), `reason="..."`. The same marker works on a `//` comment above a
`.route(...)` line. This is the whole lifecycle: **added → deprecated (still
works, migrate) → removed** — and every consumer repo learns which stage each
piece of the surface is at, per HONE commit, automatically.

---

*Cross-refs:* `docs/SERVICE_HOST_V2_14.md` (the hosting surface this manifest
exposes), `docs/AGENT_INTEGRATION.md`, `docs/API_CATALOG_LIBRARY.md`.
Implementation: `rust/hone-sdk/src/manifest/`, `.github/workflows/manifest-check.yml`.
