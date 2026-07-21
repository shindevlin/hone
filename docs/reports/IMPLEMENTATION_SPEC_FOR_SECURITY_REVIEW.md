# HONE Implementation Spec for Security Review

Date: 2026-04-28

This document is a handoff for a second model.

It lists the code I would write, the files I would touch, and the exact insertion points for the highest-risk fixes identified in the review:

1. Remove the private authorization bypass.
2. Authenticate storage heartbeats.
3. Make storage file creation signable and deterministic.
4. Tighten clock heartbeat witness validation.
5. Enforce non-negative spendable balances during replay and finality hydration.

Do not treat this as implemented code. It is a change spec that another model should review for correctness and security before coding.

## Scope and Order

Implement in this order:

1. Storage auth and file-creation flow.
2. Clock heartbeat trust model.
3. Private authorization service.
4. Replay and finality balance validation.
5. Tests and documentation updates.

The reason for this order is that the storage route and heartbeat fixes are concrete protocol issues, while private authorization is currently a stub and should be either made real or made explicitly unavailable.

## 4. Enforce Non-Negative Spendable Balances

### Goal

The replay path currently accepts negative HONE balances from persisted state. That is a chain integrity issue, not just a UI bug. A wallet must never re-enter the active state with a spendable balance below zero, even if the on-disk snapshot is malformed or stale.

### Files to change

- [`src/chain/stateStore.js`](/mnt/btcpc-storage/repos/hone/src/chain/stateStore.js)
- [`src/chain/replay.js`](/mnt/btcpc-storage/repos/hone/src/chain/replay.js)
- [`src/chain/blockStore.js`](/mnt/btcpc-storage/repos/hone/src/chain/blockStore.js)
- [`src/p2p/chainSync.js`](/mnt/btcpc-storage/repos/hone/src/p2p/chainSync.js)
- [`tests/replay.test.js`](/mnt/btcpc-storage/repos/hone/tests/replay.test.js)
- [`tests/stateStore.test.js`](/mnt/btcpc-storage/repos/hone/tests/stateStore.test.js)

### Insertion point

Patch the finality hydration path in `src/chain/stateStore.js` and the startup replay path in `src/chain/replay.js`.

### Proposed code shape

```js
function _assertNonNegativeBalances() {
  // Scan all HONE balances after replay or finality hydration.
  // If any non-system account is negative, fail hard rather than carrying
  // a corrupt spendable state forward.
}

function hydrateFromFinality(snapshot) {
  // existing hydration logic
  // ...
  _assertNonNegativeBalances();
}
```

In `src/chain/replay.js`, after loading finality and replaying blocks:

```js
if (!stateStore.assertNonNegativeBalances || !stateStore.assertNonNegativeBalances()) {
  throw new Error("Corrupt chain state: negative spendable balance detected");
}
```

### Review notes

- This should fail closed.
- If a snapshot contains a negative balance, do not silently clamp it to zero without a migration plan.
- The correct fix is to identify the entry stream that created the negative state and reject or repair it at the source, then add replay-time validation so the corruption cannot persist.
- The security review should also inspect every direct `balances.set(...)` path and any snapshot import path that bypasses `_debit()`.

## 1. Fix Private Authorization

### Goal

The current service returns `verified: true` for every request. That is a security bypass. Replace it with a strict policy-driven flow or disable the route until the feature exists.

### Files to change

- [`src/services/privateAuthorization.js`](/mnt/btcpc-storage/repos/hone/src/services/privateAuthorization.js)
- [`src/routes/walletRoutes.js`](/mnt/btcpc-storage/repos/hone/src/routes/walletRoutes.js)
- [`src/controllers/walletController.js`](/mnt/btcpc-storage/repos/hone/src/controllers/walletController.js)
- [`src/routes/botRoutes.js`](/mnt/btcpc-storage/repos/hone/src/routes/botRoutes.js)
- [`src/explorer/server.js`](/mnt/btcpc-storage/repos/hone/src/explorer/server.js)
- [`tests/privateAuthorization.test.js`](/mnt/btcpc-storage/repos/hone/tests/privateAuthorization.test.js)
- `tests/walletController.test.js`

### Insertion point

Replace the entire stub in `src/services/privateAuthorization.js`.

### Proposed code shape

```js
"use strict";

const crypto = require("crypto");
const stateStore = require("../chain/stateStore");
const { sanitizeString } = require("../middlewares/validate");

const CHALLENGE_TTL_MS = 10 * 60 * 1000;
const policies = new Map();
const enrollmentChallenges = new Map();
const transferChallenges = new Map();

function _id() {
  return crypto.randomBytes(16).toString("hex");
}

function _now() {
  return Date.now();
}

function _cleanup(map) {
  const now = _now();
  for (const [id, rec] of map.entries()) {
    if (!rec || rec.used || rec.expiresAt <= now) map.delete(id);
  }
}

function _getAccountPublicKey(username, role) {
  const acc = stateStore.getAccount ? stateStore.getAccount(username) : null;
  return acc && acc.public_keys ? acc.public_keys[role] || null : null;
}

async function getPolicy(user) {
  const username = sanitizeString(user || "", 80);
  const policy = policies.get(username);
  return policy || { user: username, enabled: false, threshold: 0, enrolled_devices: [] };
}

async function setPolicy(user, { threshold, enabled }) {
  const username = sanitizeString(user || "", 80);
  const policy = {
    user: username,
    enabled: !!enabled,
    threshold: Math.max(0, Number(threshold) || 0),
    enrolled_devices: (policies.get(username) && policies.get(username).enrolled_devices) || [],
  };
  policies.set(username, policy);
  return policy;
}

async function requestEnrollment(user, chain, label, address) {
  const username = sanitizeString(user || "", 80);
  const challengeId = _id();
  const challenge = {
    challengeId,
    user: username,
    chain: sanitizeString(chain || "", 40),
    label: sanitizeString(label || "", 120),
    address: sanitizeString(address || "", 200),
    message: [
      "HONE PRIVATE AUTH ENROLLMENT",
      "challenge_id=" + challengeId,
      "user=" + username,
      "chain=" + sanitizeString(chain || "", 40),
      "label=" + sanitizeString(label || "", 120),
      "address=" + sanitizeString(address || "", 200),
    ].join("\\n"),
    createdAt: _now(),
    expiresAt: _now() + CHALLENGE_TTL_MS,
    used: false,
  };
  enrollmentChallenges.set(challengeId, challenge);
  return challenge;
}

async function verifyEnrollment(challengeId, signature) {
  _cleanup(enrollmentChallenges);
  const rec = enrollmentChallenges.get(challengeId);
  if (!rec) return { success: false, error: "challenge not found or expired" };
  if (rec.used) return { success: false, error: "challenge already used" };

  // Verify against the account's active key first, then memo if the product
  // wants a different factor. Exact policy should be decided by review.
  const activePub = _getAccountPublicKey(rec.user, "active");
  const memoPub = _getAccountPublicKey(rec.user, "memo");
  const key = activePub || memoPub;
  if (!key) return { success: false, error: "no public key registered for enrollment" };

  // signature verification should be delegated to existing wallet crypto helper
  // used elsewhere in the codebase.
  const valid = require("../wallet/keyManager").verifySignature(rec.message, signature, key);
  if (!valid) return { success: false, error: "invalid enrollment signature" };

  rec.used = true;
  enrollmentChallenges.delete(challengeId);

  const policy = policies.get(rec.user) || { user: rec.user, enabled: false, threshold: 0, enrolled_devices: [] };
  policy.enrolled_devices = policy.enrolled_devices || [];
  policy.enrolled_devices.push({
    chain: rec.chain,
    label: rec.label,
    address: rec.address,
    enrolledAt: _now(),
  });
  policies.set(rec.user, policy);

  return { success: true, user: rec.user, chain: rec.chain, label: rec.label, address: rec.address };
}

async function requestTransferAuthorization(user, transferDetails) {
  const username = sanitizeString(user || "", 80);
  const challengeId = _id();
  const challenge = {
    challengeId,
    user: username,
    transfer: {
      to: sanitizeString((transferDetails && (transferDetails.to || transferDetails.toAddress || transferDetails.recipient)) || "", 200),
      amount: Number(transferDetails && transferDetails.amount) || 0,
      token: sanitizeString((transferDetails && transferDetails.token) || "HONE", 20) || "HONE",
      memo: sanitizeString((transferDetails && transferDetails.memo) || "", 500) || "",
    },
    message: [
      "HONE PRIVATE AUTH TRANSFER",
      "challenge_id=" + challengeId,
      "user=" + username,
      "to=" + (transferDetails && (transferDetails.to || transferDetails.toAddress || transferDetails.recipient) || ""),
      "amount=" + String((transferDetails && transferDetails.amount) || ""),
      "token=" + ((transferDetails && transferDetails.token) || "HONE"),
      "memo=" + ((transferDetails && transferDetails.memo) || ""),
    ].join("\\n"),
    createdAt: _now(),
    expiresAt: _now() + CHALLENGE_TTL_MS,
    used: false,
  };
  transferChallenges.set(challengeId, challenge);
  return challenge;
}

async function verifyTransferAuthorization(account, transferData, privateAuth) {
  const username = sanitizeString(account || "", 80);
  const policy = await getPolicy(username);
  if (!policy.enabled) {
    return { requestId: _id(), threshold: 0, approvalCount: 0, factors: [], verified: true, disabled: true };
  }

  _cleanup(transferChallenges);
  const challengeId = sanitizeString(privateAuth && (privateAuth.challengeId || privateAuth.challenge_id) || "", 120);
  const signature = sanitizeString(privateAuth && (privateAuth.signature || "") || "", 512);
  const challenge = challengeId ? transferChallenges.get(challengeId) : null;
  if (!challenge) return { verified: false, error: "challenge not found or expired" };
  if (challenge.user !== username) return { verified: false, error: "challenge does not belong to account" };

  const expected = challenge.transfer;
  const actual = {
    to: sanitizeString(transferData && (transferData.to || transferData.toAddress || transferData.recipient) || "", 200),
    amount: Number(transferData && transferData.amount) || 0,
    token: sanitizeString((transferData && transferData.token) || "HONE", 20) || "HONE",
    memo: sanitizeString((transferData && transferData.memo) || "", 500) || "",
  };
  if (
    expected.to !== actual.to ||
    expected.amount !== actual.amount ||
    expected.token !== actual.token ||
    expected.memo !== actual.memo
  ) {
    return { verified: false, error: "transfer mismatch" };
  }

  const activePub = _getAccountPublicKey(username, "active");
  if (!activePub) return { verified: false, error: "no active key registered" };
  const ok = require("../wallet/keyManager").verifySignature(challenge.message, signature, activePub);
  if (!ok) return { verified: false, error: "invalid signature" };

  challenge.used = true;
  transferChallenges.delete(challengeId);
  return { requestId: _id(), threshold: policy.threshold, approvalCount: 1, factors: ["active"], verified: true };
}

module.exports = {
  getPolicy,
  setPolicy,
  requestEnrollment,
  verifyEnrollment,
  requestTransferAuthorization,
  verifyTransferAuthorization,
};
```

### Review notes

- This implementation intentionally keeps the policy state in memory only.
- If that is not acceptable, the next version should persist policies in chain state or a dedicated durable store.
- If the product does not need private auth yet, the safer alternative is to remove the routes that depend on it instead of shipping a stub.

## 2. Authenticate Storage Heartbeats

### Goal

Prevent arbitrary callers from spoofing storage host liveness.

### Files to change

- [`src/routes/storageRoutes.js`](/mnt/btcpc-storage/repos/hone/src/routes/storageRoutes.js)
- [`src/services/ledger.js`](/mnt/btcpc-storage/repos/hone/src/services/ledger.js)
- [`src/chain/stateStore.js`](/mnt/btcpc-storage/repos/hone/src/chain/stateStore.js)
- [`tests/storageRoutes.test.js`](/mnt/btcpc-storage/repos/hone/tests/storageRoutes.test.js)
- `tests/storageHeartbeat.test.js`

### Insertion point

Replace the `POST /heartbeat` handler in `src/routes/storageRoutes.js`.

### Proposed code shape

```js
router.post("/heartbeat", async (req, res) => {
  try {
    const body = req.body || {};
    const host = body.host;
    const timestamp = Number(body.timestamp) || 0;
    const signature = body.signature || "";
    const cids = Array.isArray(body.cids) ? body.cids : [];
    const capacityUsedGb = Number(body.capacity_used_gb) || 0;

    if (!host || typeof host !== "string") {
      return res.status(400).json({ error: "host (string) required" });
    }
    if (!signature) {
      return res.status(401).json({ error: "signature required" });
    }
    if (Math.abs(Date.now() - timestamp) > 60000) {
      return res.status(400).json({ error: "timestamp out of range" });
    }

    const acc = stateStore.getAccount(host);
    if (!acc) return res.status(404).json({ error: "host account not found" });

    const postingPub = (acc.public_keys || {}).posting;
    if (!postingPub) return res.status(400).json({ error: "host has no posting public key" });

    const payload = JSON.stringify({
      host,
      timestamp,
      cids,
      capacity_used_gb: capacityUsedGb,
    });

    if (!_checkSig(host, payload, signature)) {
      return res.status(401).json({ error: "invalid signature" });
    }

    const currentEpoch = Math.max(stateStore.getChainHeight(), 0);
    await ledger.recordStorageHeartbeat(host, cids, capacityUsedGb, currentEpoch);

    return res.json({ ok: true, host, epoch: currentEpoch });
  } catch (e) {
    console.error("[storageRoutes] heartbeat error:", e.message);
    return res.status(500).json({ error: e.message });
  }
});
```

### Review notes

- The signature must bind `host`, `timestamp`, `cids`, and `capacity_used_gb`.
- If `cids` changes too often for signing, sign a stable digest instead and store that digest in the heartbeat record.
- This route should probably be rate limited even after authentication.

## 3. Fix Storage File Creation

### Goal

Make the signed payload deterministic and client-computable.

### Problem in current flow

The server creates `storageId` and then expects the client to sign a payload containing that ID. That cannot work unless a separate client path predicts the ID.

### Files to change

- [`src/routes/storageRoutes.js`](/mnt/btcpc-storage/repos/hone/src/routes/storageRoutes.js)
- [`src/services/ledger.js`](/mnt/btcpc-storage/repos/hone/src/services/ledger.js)
- [`src/chain/stateStore.js`](/mnt/btcpc-storage/repos/hone/src/chain/stateStore.js)
- [`tests/storageRoutes.test.js`](/mnt/btcpc-storage/repos/hone/tests/storageRoutes.test.js)

### Insertion point

Replace the `POST /files` handler in `src/routes/storageRoutes.js`.

### Proposed API change

Require the client to supply `storage_id` in the request body. The server validates the supplied ID and signs the same value back into the ledger.

### Proposed code shape

```js
router.post("/files", async (req, res) => {
  try {
    const body = req.body || {};
    const owner = body.owner;
    const storageId = body.storage_id;
    const manifest = body.manifest;
    const signature = body.signature;
    const timestamp = Number(body.timestamp) || 0;

    if (!owner || typeof owner !== "string") return res.status(400).json({ error: "owner required" });
    if (!storageId || typeof storageId !== "string") return res.status(400).json({ error: "storage_id required" });
    if (!manifest || typeof manifest !== "object") return res.status(400).json({ error: "manifest (object) required" });
    if (!signature) return res.status(400).json({ error: "signature required" });
    if (Math.abs(Date.now() - timestamp) > 60000) return res.status(400).json({ error: "timestamp out of range" });

    const acc = stateStore.getAccount(owner);
    if (!acc) return res.status(404).json({ error: "account not found: " + owner });

    const postingPub = (acc.public_keys || {}).posting;
    const memoPub = (acc.public_keys || {}).memo;
    if (!postingPub) return res.status(400).json({ error: "owner has no posting public key" });
    if (!memoPub) return res.status(400).json({ error: "owner has no memo public key" });

    const sigPayload = JSON.stringify({ owner, storage_id: storageId, timestamp });
    if (!_checkSig(owner, sigPayload, signature)) {
      return res.status(401).json({ error: "invalid signature" });
    }

    const dek = storageCrypto.generateDEK();
    const encryptedManifest = storageCrypto.encryptManifest(manifest, dek);
    const wrappedDekOwner = storageCrypto.wrapDEK(dek, memoPub);
    const initialGrants = [];

    // existing grant wrapping logic stays here

    dek.fill(0);

    const epoch = Math.max(stateStore.getChainHeight(), 0);
    await ledger.recordFileStore(owner, storageId, encryptedManifest, wrappedDekOwner, initialGrants, manifest.total_size || 0, epoch);

    return res.json({ ok: true, storage_id: storageId, epoch });
  } catch (e) {
    console.error("[storageRoutes] POST /files error:", e.message);
    return res.status(500).json({ error: e.message });
  }
});
```

### Alternative if API compatibility must be preserved

If the client API must not change, then the server should generate `storageId` before the client signs it. That requires a two-step flow:

1. `POST /files/init` returns `storage_id`.
2. Client signs payload using that `storage_id`.
3. `POST /files/commit` stores the file.

That is more invasive but more explicit.

## 4. Tighten Clock Heartbeat Witness Validation

### Goal

Keep the anti-self-credit rule, but make the witness chain of custody clearer and harder to spoof.

### Files to change

- [`src/p2p/protocol.js`](/mnt/btcpc-storage/repos/hone/src/p2p/protocol.js)
- [`src/chain/blockProposal.js`](/mnt/btcpc-storage/repos/hone/src/chain/blockProposal.js)
- [`tests/clockConsensus.test.js`](/mnt/btcpc-storage/repos/hone/tests/clockConsensus.test.js)
- `tests/p2pSyncReplay.test.js`

### Insertion points

1. Modify `handleClockHeartbeat()` in `src/p2p/protocol.js`.
2. Modify the proposer-side filter in `src/chain/blockProposal.js`.

### Proposed code shape for the heartbeat handler

```js
function handleClockHeartbeat(peer, msg, ctx) {
  const data = msg.data || {};
  const account = data.account || msg.nodeId;
  const claimedEpoch = data.epoch_number || 0;
  const source = data.source || "p2p";

  // Require a valid posting-key signature if supplied.
  // Heartbeats without a valid signature should not contribute to reward eligibility.
  if (!data.signature) {
    return;
  }

  const hbVerifyData = {
    account: data.account,
    epoch_number: data.epoch_number,
    timestamp: data.timestamp,
    source: source,
  };

  const hbSigOk = messageAuth.verifyAccountSignature(account, hbVerifyData, data.signature, "posting");
  if (!hbSigOk) {
    return;
  }

  const GENESIS_TS = 1776236400000;
  const EPOCH_MS = 30000;
  const timeDerivedEpoch = Math.floor((Date.now() - GENESIS_TS) / EPOCH_MS);
  const fileEpoch = Math.max(_currentEpochCache > 0 ? _currentEpochCache : 0, timeDerivedEpoch);

  recordPeerEpoch(msg.nodeId || account, claimedEpoch);
  recordNodeActivity(msg.nodeId, account, fileEpoch);

  // Only the relay that forwarded the signed heartbeat counts as a witness.
  recordHeartbeatWitness(account, fileEpoch, msg.nodeId || peer.nodeId || "unknown");
  ctx.broadcast(msg, peer.address);
}
```

### Proposed code shape for proposer-side filtering

```js
if (typeof protocol.getHeartbeatWitnesses === "function") {
  activeClocks = rawClocks.filter(function (clock) {
    if (clock !== proposerAccount) return true;

    const witnesses = protocol.getHeartbeatWitnesses(clock, epochNumber);
    if (!witnesses || witnesses.size === 0) return false;

    // Remove trivial self-echoes if the witness set contains only the proposer.
    if (witnesses.size === 1 && witnesses.has(proposerAccount)) return false;

    return true;
  });
}
```

### Review notes

- The safest version is to require a valid signature for heartbeat participation.
- If unsigned heartbeats must stay allowed for operational reasons, they should not affect rewards.
- The review should verify that witness collection cannot be trivially self-generated through local replay.

## 5. Tests To Add or Update

### Storage

- Reject unauthenticated heartbeats.
- Accept signed heartbeats.
- Reject `/files` when `storage_id` is missing.
- Accept `/files` when `storage_id` is signed and stable.

### Private authorization

- Reject transfer when policy is enabled and signature is missing.
- Accept transfer with valid challenge and signature.
- Reject reused challenges.

### Clock consensus

- Reject proposer self-credit when there are zero witnesses.
- Accept clock participation with a valid witness set.
- Reject unsigned heartbeat rewards.

## 6. What Another Model Should Review Before Implementing

Before coding, another model should validate these questions:

1. Should private auth be a real feature now, or should its public routes be removed until the implementation exists?
2. Should storage heartbeats be signed with posting keys or a dedicated host key?
3. Should `storage_id` be client-generated, or should the API be changed to a two-step create flow?
4. Should clock heartbeats be rejected unless signed, or merely excluded from reward logic?

Those decisions affect the final shape of the patch.
