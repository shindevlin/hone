# BTCPC v3.2.0 — Next Implementation Roadmap

Generated: 2026-04-15
Base version: 3.1.11 (targeting 3.2.0)

## Summary Table

| # | Item | Complexity | Branch Reuse | Status |
|---|------|-----------|--------------|--------|
| 1 | Agent Protocol | L | worktree-agent-ab749035 / worktree-agent-a0d32218 (ECDH session system) | Not started |
| 2 | Sensor Verification | M | None — implement from scratch | Not started |
| 3 | Verifier Commit-Reveal | M | worktree-agent-aefd6121 (P2P sig framework) | Not started |
| 4 | Proof Replay Detection | S | Pattern exists in stateStore.seenEntries | Done (v3.1.11) |
| 5 | Desktop Auto-Update | S | None | Done (v3.1.11) |
| 6 | Emergency Drill Script | S | scripts/emergency-pause.sh for reference | Done (v3.1.11) |
| 7 | MIN_WORK_THRESHOLD Default | S | None | Done (v3.1.11) |
| 8 | Logo Integration | S | website/assets/logo.jpeg already present | Done (v3.1.11) |
| 9 | P2P Noise Protocol Encryption | L | None | Not started |
| 10 | LinkGit Public API (Repos/Issues/Hooks/Tokens) | M | Existing auth/token patterns + API routing | Planned |

---

## 1. Agent Protocol (Complexity: L)

### What it is
Tool-calling sessions where multi-turn inference happens on miners. A client opens a
SESSION_CREATE with a system prompt and an array of tool definitions. During the session,
the miner can emit TOOL_CALL P2P messages; the client responds with TOOL_RESULT messages.
Conversation context is persisted per session_id for the lifetime of the session.

### Why now
The encrypted inference branches (worktree-agent-ab749035, worktree-agent-a0d32218) already
contain a working ECDH + AES-256-GCM session system in src/inference/session.js with
createSession / deriveSessionKey / destroySession and a 5-minute TTL. The session key
infrastructure is complete and can be extended to carry tool-call state without re-doing
the cryptographic plumbing. This is the highest-value feature differentiating BTCPC inference
from commodity API providers.

### Affected files
- `src/inference/session.js` — extend session record to hold { systemPrompt, tools[], history[] }
- `src/inference/api.js` — add POST /v1/agent/session (SESSION_CREATE) and POST /v1/agent/turn
- `src/p2p/protocol.js` — add TOOL_CALL and TOOL_RESULT to MESSAGE_TYPES and handler dispatch

### Sub-phases

**Phase alpha — Session open/close endpoints**
- Extend the session Map entry to include `systemPrompt`, `toolDefinitions`, and `history`.
- Add POST /v1/agent/session: accepts { system_prompt, tools[], client_public_key }; returns
  { session_id, server_public_key } via the existing createSession() flow.
- Add DELETE /v1/agent/session/:id: calls destroySession().
- Unit tests: session creation, TTL expiry, key zeroing on destroy.

**Phase beta — Tool definition schema + TOOL_CALL P2P message**
- Define JSON Schema for tool definitions: { name, description, parameters (JSON Schema object) }.
- Add MESSAGE_TYPES.TOOL_CALL: { session_id, call_id, tool_name, arguments }.
- Add MESSAGE_TYPES.TOOL_RESULT: { session_id, call_id, result, error? }.
- Miner inference handler: when model output contains a tool_use block, package it as TOOL_CALL
  and broadcast to the originating peer address stored in the session record.
- Validate tool names against the tool definitions registered at session open.

**Phase gamma — TOOL_RESULT collection and response routing**
- Add POST /v1/agent/turn: accepts { session_id, encrypted_turn } (reuses session key encrypt/decrypt).
  Decrypts turn, appends to history, runs next inference step, encrypts response.
- On receiving TOOL_RESULT via P2P: look up session_id, append result to history, resume inference.
- Response routing: the miner that owns the session (has the derived key) is the only node that
  can decrypt and continue. Route TOOL_RESULT messages point-to-point to that miner, not broadcast.
- Add session history pruning: keep last N turns within SESSION_TTL_MS to cap memory.

### Branch reuse notes
Cherry-pick src/inference/session.js and src/inference/encrypted.js from
worktree-agent-ab749035. The ECDH + HKDF + AES-256-GCM primitives are production-quality and
require no changes. Only the session data model and API surface need extension.

---

## 2. Sensor Verification (Complexity: M)

### What it is
Cross-validation between sensors reporting the same physical space. Currently
sensorDataBilling.js accepts readings without any cryptographic attestation or
cross-sensor agreement check. This item adds: gateway co-signing on LoRa packets,
a geographic consensus requirement (2+ sensors within 100m must agree within 10%),
and stake slashing for sensors that consistently diverge from consensus.

### Why now
Without verification, any node can submit arbitrary sensor readings and collect billing
rewards. This is the primary economic attack vector on the sensor data market.

### Affected files
- `src/services/sensorDataBilling.js` — add gateway signature validation and cross-validation logic
- `src/chain/stateStore.js` — add sensorReadingsByLocation index and divergence strike counter
- `src/p2p/protocol.js` — add SENSOR_READING message type for cross-peer validation gossip

### Sub-phases

**Phase alpha — Gateway signature requirement**
- Each sensor reading payload must include a `gateway_sig` field: Ed25519 signature by the
  registered gateway over (sensor_id + timestamp + value + nonce).
- Add gateway public key registry lookup in sensorDataBilling.js: reject readings with
  missing or invalid gateway_sig.
- Register gateway keys via src/services/loraGatewayRegistry.js (already exists).

**Phase beta — Cross-validation: 2-of-N geographic consensus**
- On receiving a sensor reading, query stateStore for other readings within 100m in the
  last 60 seconds (use sensor lat/lon metadata already present in the billing schema).
- If 2+ readings exist for the same physical quantity: require all readings to agree within
  10% of median. Readings outside threshold are marked `unconfirmed` and held in a
  pending buffer for up to 5 minutes.
- A reading that gains 2+ corroborating readings within the window is promoted to `confirmed`
  and eligible for billing reward. Lone readings from isolated sensors are still accepted
  but flagged as unverified in the billing record.

**Phase gamma — Slash stake on persistently divergent sensors**
- Track a `divergence_strikes` counter per sensor_id in stateStore.
- Increment on each reading that is rejected by cross-validation or whose gateway_sig fails.
- At 5 strikes within a rolling 24-hour window: call slashing.recordOffense() with offense
  type SENSOR_DIVERGENCE, reducing the sensor owner's staked balance by the configured
  slash percentage.
- Add a reset path: strikes decay by 1 per 6-hour window with no new strikes.

### Branch reuse notes
No relevant branch work. Implement from scratch. The existing slashing.js and
loraGatewayRegistry.js infrastructure provide the hooks needed.

---

## 3. Verifier Commit-Reveal (Complexity: M)

### What it is
A two-phase reveal scheme to prevent verifier collusion (security issue T-04). Currently,
verifiers in src/p2p/protocol.js broadcast their VERIFY_RESPONSE immediately after running
inference verification. This allows the first verifier's verdict to influence later verifiers.
The fix: verifiers first broadcast a commitment (SHA-256 of verdict + nonce), then reveal
within a 15-second window. Verdicts that are not revealed are treated as abstentions and
trigger stake slashing.

### Why now
VERIFY_REQUEST and VERIFY_RESPONSE message types already exist in protocol.js. The verifier
selection and eligibility logic is complete (lines 1452-1530). worktree-agent-aefd6121 has
a P2P signature verification framework and an entry type allowlist that can be adapted to
validate the two-phase message sequence.

### Affected files
- `src/p2p/protocol.js` — add VERIFIER_COMMIT and VERIFIER_REVEAL to MESSAGE_TYPES; add
  handlers; modify verifier dispatch to use two-phase flow
- `src/services/verifier.js` — add commitVerdict() and revealVerdict() functions

### Sub-phases

**Phase alpha — Add VERIFIER_COMMIT and VERIFIER_REVEAL message types**
- Add to MESSAGE_TYPES: VERIFIER_COMMIT, VERIFIER_REVEAL.
- VERIFIER_COMMIT payload: { job_id, epoch, verifier, commitment: sha256(verdict+nonce), sig }.
- VERIFIER_REVEAL payload: { job_id, epoch, verifier, verdict, nonce, sig }.
- In the existing verifier dispatch, replace the immediate VERIFY_RESPONSE broadcast with a
  VERIFIER_COMMIT broadcast. Store { commitment, verdict, nonce } in a local Map keyed by
  (job_id + verifier).
- Schedule a VERIFIER_REVEAL broadcast 15 seconds after commit.

**Phase beta — Aggregation logic**
- On receiving VERIFIER_REVEAL: verify the commitment (sha256(verdict+nonce) === stored commitment).
- Reject reveals with mismatched commitments; log as VERIFIER_EQUIVOCATION.
- Once the reveal window closes (15s after first commit for a given job_id), aggregate
  valid verdicts using the existing majority logic. Pass result to the finalization flow.
- Add per-job reveal tracking to stateStore: { commits: Map<verifier, commitment>, reveals: Map<verifier, verdict> }.

**Phase gamma — Slash on missing reveal**
- After the reveal window expires, identify verifiers who committed but did not reveal.
- Call slashing.recordOffense() with VERIFIER_NO_REVEAL for each non-revealer.
- Apply a lighter slash (e.g., 10% of VERIFIER_EQUIVOCATION) since a missed reveal may be
  a network issue. After 3 consecutive no-reveals, escalate to the full VERIFIER_EQUIVOCATION rate.

### Branch reuse notes
Cherry-pick the P2P signature verification pattern and the entry type allowlist from
worktree-agent-aefd6121. The commitment hash function (SHA-256) is already available via
Node's crypto module used throughout the codebase.

---

## 9. P2P Noise Protocol Encryption (Complexity: L)

### What it is
All P2P messages in src/p2p/protocol.js are currently transmitted as plaintext JSON over
WebSocket. This exposes inference payloads, wallet addresses, and proof data to any network
observer. This item adds transport-layer confidentiality using the Noise_XX handshake pattern
with ChaCha20-Poly1305 AEAD for all subsequent messages.

### Why now
This is a prerequisite for operating on public infrastructure (non-VPN peers). The current
deployment relies on Tailscale WireGuard (see HONE_SEED_PEERS in .env) as a stopgap.
Noise Protocol removes that dependency and allows any peer to connect securely without
a pre-shared VPN.

### Affected files
- `src/p2p/protocol.js` — integrate encryptedTransport on connection; wrap send/receive
- `src/p2p/encryptedTransport.js` (new) — Noise_XX state machine + ChaCha20-Poly1305 helpers

### Sub-phases

**Phase alpha — Noise_XX handshake on peer connect**
- Add `@stablelib/noise` (pure-JS, no native addon — works in Electron and ARM) as a dependency.
- In encryptedTransport.js: implement initiator and responder sides of Noise_XX.
  Noise_XX provides mutual authentication and is the correct pattern when both parties'
  static keys are unknown at connection time (matches BTCPC's open peer model).
- Each node generates a persistent Noise static keypair on first startup, stored in
  src/services/secretStore.js (already provides key persistence).
- In protocol.js: on WebSocket `open` (outbound) or `connection` (inbound), perform the
  Noise_XX handshake before any application messages.

**Phase beta — Wrap all message send/receive with ChaCha20-Poly1305**
- After the handshake, replace the raw `ws.send(JSON.stringify(msg))` pattern with
  `transport.send(ws, msg)` which: serializes to JSON, encrypts with the send CipherState
  (ChaCha20-Poly1305, incrementing nonce), and sends the ciphertext as a binary WebSocket frame.
- In the `ws.on('message')` handler, replace the raw JSON.parse with
  `transport.receive(ws, data)` which: decrypts, then JSON.parses.
- Maintain one CipherState pair per peer connection, stored in the existing peer tracking
  structure in protocol.js.

**Phase gamma — Peer key registry**
- Record each peer's Noise static public key after successful handshake in a persistent
  peer key registry (flat JSON file in data/).
- On reconnection, validate that the peer presents the same static key. A key change is
  logged as a security event; with HONE_NOISE_STRICT_KEY_PIN=true, the connection is
  rejected.
- Export `/admin/peers/keys` endpoint listing known peer keys and their last-seen times.

### Branch reuse notes
No branch has Noise Protocol work. The ECDH and AES primitives in src/inference/session.js
are pattern reference only — the Noise implementation requires a dedicated state machine.
Review the @stablelib/noise package API before starting.

---

## 10. LinkGit Public API (Complexity: M)

### What it is
A first-class LinkGit API surface for repository lifecycle, issues/PR automation,
webhooks, and scoped access tokens so teams can integrate LinkGit into CI/CD and
developer tooling the same way they do with GitHub/Codeberg.

### Why now
LinkGit currently presents as a forge UI, but high-usage teams convert only when
automation primitives are available. API completeness is required for migration,
bot workflows, and ecosystem adoption.

### Affected files
- `src/inference/api.js` or dedicated `src/linkgit/api.js` (new) — REST handlers
- `src/services/secretStore.js` — token generation/storage reuse pattern
- `src/chain/stateStore.js` — API token scopes, webhook config state (if on-chain tracked)

### Sub-phases

**Phase alpha — Core repo + token APIs**
- `POST /api/v1/repos`, `GET /api/v1/repos/:owner/:name`, `PATCH /api/v1/repos/:owner/:name`.
- Scoped personal access tokens (read_repo, write_repo, admin_repo).
- Token issue/revoke/list endpoints with hashed-at-rest storage.

**Phase beta — Issues/PR + webhooks**
- `GET/POST /api/v1/issues`, `GET/POST /api/v1/pulls` core endpoints.
- Webhook registration + delivery signing (`X-LinkGit-Signature`).
- Retry policy + dead-letter visibility for failed webhook deliveries.

**Phase gamma — Org/service automation controls**
- Organization service tokens with narrow scopes and expiry.
- Rate-limit policy per token and per IP with admin override.
- API analytics (anonymized cohorts only) for usage/adoption metrics.

### Branch reuse notes
Leverage existing API auth/session patterns and token secret handling already used
in the codebase to avoid introducing a second credential lifecycle.

---

## Implementation Sequencing

Items 4, 5, 6, 7, 8 are done as of v3.1.11.

Remaining order: **3** (verifier commit-reveal) → **2** (sensor verification) → **1** (agent protocol) → **9** (Noise encryption).

- Items 2 and 3 share src/p2p/protocol.js and src/chain/stateStore.js. Do 3 first (smaller).
- Item 1 (agent protocol) can run in parallel with 2-3 since it primarily touches src/inference/.
- Item 9 (Noise encryption) is a large cross-cutting change. Schedule last, after items 1-3
  are merged and stable.
