"use strict";

/**
 * HONE-FS Storage Routes — v3.1.110
 * Shin Devlin
 *
 * Endpoints:
 *   POST /api/storage/heartbeat        — storage host heartbeat (daemon)
 *   GET  /api/storage/hosts            — list active storage hosts
 *   GET  /api/storage/hosts/:host      — single host info
 *
 *   POST /api/storage/files            — store a new encrypted file manifest
 *   GET  /api/storage/files/:id        — get file metadata + encrypted manifest
 *   POST /api/storage/files/:id/grant  — owner adds a grantee (supplies wrapped_dek)
 *   DELETE /api/storage/files/:id/grant/:grantee — owner revokes access
 *   GET  /api/storage/files            — list files owned by ?owner=<account>
 */

const crypto = require("crypto");
const express = require("express");
const router = express.Router();
const stateStore = require("../chain/stateStore");
const ledger = require("../services/ledger");
const storageCrypto = require("../services/storageCrypto");
const { verifySignature } = require("../wallet/keyManager");

// Verify a posting-key signature over a canonical payload string.
function _checkSig(account, payload, signature) {
  var acc = stateStore.getAccount(account);
  if (!acc) return false;
  var pub = (acc.public_keys || {}).posting;
  if (!pub) return false;
  try { return verifySignature(payload, signature, pub); }
  catch (_) { return false; }
}

/**
 * POST /api/storage/heartbeat
 *
 * Body (JSON):
 *   host         {string}   — storage host account name (required)
 *   cids         {string[]} — list of CIDs currently held on disk
 *   capacity_used_gb {number} — bytes used / 1GiB
 *
 * No authentication required — the host identity is in the body.
 * Rate limiting from the global API limiter is sufficient; heartbeats
 * are sent at most once per minute per daemon.
 *
 * Returns: { ok: true, epoch: <recorded epoch>, host: <host> }
 */
router.post("/heartbeat", async (req, res) => {
  try {
    var body = req.body || {};
    var host = body.host;
    if (!host || typeof host !== "string") {
      return res.status(400).json({ error: "host (string) required" });
    }
    var cids = Array.isArray(body.cids) ? body.cids : [];
    var capacityUsedGb = Number(body.capacity_used_gb) || 0;

    // Use the real current chain epoch so the heartbeat timestamp is
    // meaningful to getActiveStorageHosts(currentEpoch, recentEpochs).
    var currentEpoch = stateStore.getChainHeight();
    if (currentEpoch < 0) currentEpoch = 0;

    await ledger.recordStorageHeartbeat(host, cids, capacityUsedGb, currentEpoch);

    return res.json({ ok: true, host: host, epoch: currentEpoch });
  } catch (e) {
    console.error("[storageRoutes] heartbeat error:", e.message);
    return res.status(500).json({ error: e.message });
  }
});

/**
 * GET /api/storage/hosts
 *
 * Returns all storage hosts that have heartbeated within the last
 * `window` epochs (default 200 — roughly 100 minutes at 30s epochs).
 * Query param: ?window=<n>
 */
router.get("/hosts", function (req, res) {
  try {
    var currentEpoch = stateStore.getChainHeight();
    if (currentEpoch < 0) currentEpoch = 0;
    var window = parseInt(req.query.window || "200", 10);
    if (isNaN(window) || window < 1) window = 200;
    var hosts = stateStore.getActiveStorageHosts(currentEpoch, window);
    var redacted = hosts.map(function(h) {
      var r = Object.assign({}, h);
      r.cids_count = Array.isArray(r.cids) ? r.cids.length : 0;
      delete r.cids;
      return r;
    });
    return res.json({ ok: true, epoch: currentEpoch, count: redacted.length, hosts: redacted });
  } catch (e) {
    return res.status(500).json({ error: e.message });
  }
});

/**
 * GET /api/storage/hosts/:host
 * Returns full heartbeat record for a single host.
 */
router.get("/hosts/:host", function (req, res) {
  try {
    var record = stateStore.getStorageHeartbeat(req.params.host);
    if (!record) {
      return res.status(404).json({ error: "host not found", host: req.params.host });
    }
    var redacted = Object.assign({}, record);
    redacted.cids_count = Array.isArray(redacted.cids) ? redacted.cids.length : 0;
    delete redacted.cids;
    return res.json({ ok: true, record: redacted });
  } catch (e) {
    return res.status(500).json({ error: e.message });
  }
});

// ── Private encrypted file storage (v3.1.110+) ──────────────────────────────

/**
 * POST /api/storage/files
 *
 * Store a new private file. The server generates a DEK, encrypts the manifest
 * with it, and wraps the DEK for the owner's memo public key. The plaintext
 * DEK is discarded immediately after wrapping — only the owner can recover it.
 *
 * Body (JSON):
 *   owner     {string}   — account name (must exist on chain)
 *   manifest  {object}   — chunk manifest: { chunk_size, total_size, chunks: [{cid, size, url}] }
 *   signature {string}   — hex posting-key sig over JSON.stringify({owner, storage_id, timestamp})
 *   timestamp {number}   — unix ms (must be within 60s of server time)
 *   grants    {array?}   — optional initial grantees: [{grantee, expires_at}]
 *                          server wraps DEK for each grantee's memo pub key
 *
 * Returns: { ok, storage_id, epoch }
 */
router.post("/files", async (req, res) => {
  try {
    var body = req.body || {};
    var owner = body.owner;
    var manifest = body.manifest;
    var signature = body.signature;
    var timestamp = Number(body.timestamp) || 0;

    if (!owner || typeof owner !== "string") return res.status(400).json({ error: "owner required" });
    if (!manifest || typeof manifest !== "object") return res.status(400).json({ error: "manifest (object) required" });
    if (!signature) return res.status(400).json({ error: "signature required" });
    if (Math.abs(Date.now() - timestamp) > 60000) return res.status(400).json({ error: "timestamp out of range" });

    var acc = stateStore.getAccount(owner);
    if (!acc) return res.status(404).json({ error: "account not found: " + owner });

    var storageId = crypto.randomBytes(16).toString("hex");
    var sigPayload = JSON.stringify({ owner, storage_id: storageId, timestamp });
    if (!_checkSig(owner, sigPayload, signature)) {
      return res.status(401).json({ error: "invalid signature" });
    }

    var ownerMemoPub = (acc.public_keys || {}).memo;
    if (!ownerMemoPub) return res.status(400).json({ error: "owner has no memo public key" });

    var dek = storageCrypto.generateDEK();
    var encryptedManifest = storageCrypto.encryptManifest(manifest, dek);
    var wrappedDekOwner = storageCrypto.wrapDEK(dek, ownerMemoPub);

    var initialGrants = [];
    var rawGrants = Array.isArray(body.grants) ? body.grants : [];
    for (var i = 0; i < rawGrants.length; i++) {
      var g = rawGrants[i];
      if (!g.grantee || typeof g.grantee !== "string") continue;
      var gAcc = stateStore.getAccount(g.grantee);
      var gMemoPub = gAcc && (gAcc.public_keys || {}).memo;
      if (!gMemoPub) continue;
      initialGrants.push({
        grantee: g.grantee,
        wrapped_dek: storageCrypto.wrapDEK(dek, gMemoPub),
        expires_at: g.expires_at || null,
      });
    }

    // Wipe DEK from memory
    dek.fill(0);

    var epoch = stateStore.getChainHeight();
    if (epoch < 0) epoch = 0;
    await ledger.recordFileStore(owner, storageId, encryptedManifest, wrappedDekOwner, initialGrants, manifest.total_size || 0, epoch);

    return res.json({ ok: true, storage_id: storageId, epoch });
  } catch (e) {
    console.error("[storageRoutes] POST /files error:", e.message);
    return res.status(500).json({ error: e.message });
  }
});

/**
 * GET /api/storage/files
 * List files owned by ?owner=<account>
 */
router.get("/files", function (req, res) {
  try {
    var owner = req.query.owner;
    if (!owner) return res.status(400).json({ error: "owner query param required" });
    var files = stateStore.getStoredFilesByOwner(owner);
    return res.json({ ok: true, owner, count: files.length, files: files.map(function(f) {
      return { storage_id: f.storage_id, total_size: f.total_size, stored_epoch: f.stored_epoch, grant_count: f.grants.length };
    })});
  } catch (e) {
    return res.status(500).json({ error: e.message });
  }
});

/**
 * GET /api/storage/files/:id
 *
 * Returns the encrypted manifest + the caller's wrapped_dek (if they have access).
 * Query param: ?requester=<account>
 *
 * The caller decrypts client-side:
 *   1. unwrapDEK(wrapped_dek, myMemoPrivKey) → DEK
 *   2. decryptManifest(encrypted_manifest, DEK) → chunk list
 */
router.get("/files/:id", function (req, res) {
  try {
    var f = stateStore.getStoredFile(req.params.id);
    if (!f) return res.status(404).json({ error: "file not found" });

    var requester = req.query.requester;
    var now = Date.now();

    var wrappedDek = null;
    if (requester === f.owner) {
      wrappedDek = f.wrapped_dek_owner;
    } else if (requester) {
      var grant = f.grants.find(function(g) {
        return g.grantee === requester && (g.expires_at === null || g.expires_at > now);
      });
      if (grant) wrappedDek = grant.wrapped_dek;
    }

    return res.json({
      ok: true,
      storage_id: req.params.id,
      owner: f.owner,
      total_size: f.total_size,
      stored_epoch: f.stored_epoch,
      encrypted_manifest: f.encrypted_manifest,
      wrapped_dek: wrappedDek,
      access: wrappedDek !== null,
    });
  } catch (e) {
    return res.status(500).json({ error: e.message });
  }
});

/**
 * POST /api/storage/files/:id/grant
 *
 * Owner grants access to a new grantee. The owner must supply the wrapped_dek
 * already re-encrypted for the grantee's memo public key — the server never
 * holds the plaintext DEK after initial store.
 *
 * Body:
 *   owner      {string}  — must be file owner
 *   grantee    {string}  — account to grant access
 *   wrapped_dek {object} — { ephem_pub, ciphertext, iv, tag } from client-side wrapDEK
 *   expires_at {number?} — unix ms expiry, omit for permanent
 *   signature  {string}  — posting-key sig over JSON.stringify({owner, storage_id, grantee, timestamp})
 *   timestamp  {number}
 */
router.post("/files/:id/grant", async (req, res) => {
  try {
    var body = req.body || {};
    var owner = body.owner;
    var grantee = body.grantee;
    var wrappedDek = body.wrapped_dek;
    var expiresAt = body.expires_at || null;
    var signature = body.signature;
    var timestamp = Number(body.timestamp) || 0;

    if (!owner || !grantee || !wrappedDek || !signature) {
      return res.status(400).json({ error: "owner, grantee, wrapped_dek, signature required" });
    }
    if (Math.abs(Date.now() - timestamp) > 60000) return res.status(400).json({ error: "timestamp out of range" });

    var f = stateStore.getStoredFile(req.params.id);
    if (!f) return res.status(404).json({ error: "file not found" });
    if (f.owner !== owner) return res.status(403).json({ error: "not the file owner" });

    var sigPayload = JSON.stringify({ owner, storage_id: req.params.id, grantee, timestamp });
    if (!_checkSig(owner, sigPayload, signature)) return res.status(401).json({ error: "invalid signature" });

    if (!stateStore.getAccount(grantee)) return res.status(404).json({ error: "grantee account not found" });

    var epoch = stateStore.getChainHeight();
    if (epoch < 0) epoch = 0;
    await ledger.recordFileGrant(owner, req.params.id, { grantee, wrapped_dek: wrappedDek, expires_at: expiresAt }, epoch);

    return res.json({ ok: true, storage_id: req.params.id, grantee, expires_at: expiresAt, epoch });
  } catch (e) {
    console.error("[storageRoutes] POST /files/:id/grant error:", e.message);
    return res.status(500).json({ error: e.message });
  }
});

/**
 * DELETE /api/storage/files/:id/grant/:grantee
 *
 * Owner revokes a grantee's access. Their wrapped_dek is removed from chain state.
 *
 * Body:
 *   owner     {string} — must be file owner
 *   signature {string} — posting-key sig over JSON.stringify({owner, storage_id, grantee, timestamp})
 *   timestamp {number}
 */
router.delete("/files/:id/grant/:grantee", async (req, res) => {
  try {
    var body = req.body || {};
    var owner = body.owner;
    var grantee = req.params.grantee;
    var signature = body.signature;
    var timestamp = Number(body.timestamp) || 0;

    if (!owner || !signature) return res.status(400).json({ error: "owner, signature required" });
    if (Math.abs(Date.now() - timestamp) > 60000) return res.status(400).json({ error: "timestamp out of range" });

    var f = stateStore.getStoredFile(req.params.id);
    if (!f) return res.status(404).json({ error: "file not found" });
    if (f.owner !== owner) return res.status(403).json({ error: "not the file owner" });

    var sigPayload = JSON.stringify({ owner, storage_id: req.params.id, grantee, timestamp });
    if (!_checkSig(owner, sigPayload, signature)) return res.status(401).json({ error: "invalid signature" });

    var epoch = stateStore.getChainHeight();
    if (epoch < 0) epoch = 0;
    await ledger.recordFileRevoke(owner, req.params.id, grantee, epoch);

    return res.json({ ok: true, storage_id: req.params.id, grantee, revoked: true, epoch });
  } catch (e) {
    console.error("[storageRoutes] DELETE /files/:id/grant/:grantee error:", e.message);
    return res.status(500).json({ error: e.message });
  }
});

// ── Quantum-resistant split storage (v3.1.111+) ──────────────────────────────

/**
 * POST /api/storage/shards
 *
 * Store a file using quantum-resistant split storage.
 *
 * The manifest is encrypted, split into two byte-halves, stored as separate
 * blobs, and the DEK is XOR-split. Shard A pointer (blob_A_cid + S1) is
 * wrapped with the owner's posting public key. Shard B pointer (blob_B_cid
 * + S2) is wrapped with the memo public key. Two unlinkable FILE_SHARD
 * entries are written to chain with no shared metadata.
 *
 * The pair_id that ties them together is returned once and never stored
 * on chain — the client must persist it locally.
 *
 * Body:
 *   owner     {string}   — account name
 *   manifest  {object}   — chunk manifest
 *   signature {string}   — posting-key sig over JSON.stringify({owner, timestamp})
 *   timestamp {number}   — unix ms (within 60s)
 *
 * Returns: { ok, shard_a_id, shard_b_id, pair_id, epoch }
 */
router.post("/shards", async (req, res) => {
  try {
    var body = req.body || {};
    var owner = body.owner;
    var manifest = body.manifest;
    var signature = body.signature;
    var timestamp = Number(body.timestamp) || 0;

    if (!owner || typeof owner !== "string") return res.status(400).json({ error: "owner required" });
    if (!manifest || typeof manifest !== "object") return res.status(400).json({ error: "manifest (object) required" });
    if (!signature) return res.status(400).json({ error: "signature required" });
    if (Math.abs(Date.now() - timestamp) > 60000) return res.status(400).json({ error: "timestamp out of range" });

    var acc = stateStore.getAccount(owner);
    if (!acc) return res.status(404).json({ error: "account not found: " + owner });

    var postingPub = (acc.public_keys || {}).posting;
    var memoPub = (acc.public_keys || {}).memo;
    if (!postingPub) return res.status(400).json({ error: "owner has no posting public key" });
    if (!memoPub) return res.status(400).json({ error: "owner has no memo public key" });

    var sigPayload = JSON.stringify({ owner, timestamp });
    if (!_checkSig(owner, sigPayload, signature)) return res.status(401).json({ error: "invalid signature" });

    // Encrypt manifest, split into two byte-halves
    var dek = storageCrypto.generateDEK();
    var encrypted = storageCrypto.encryptManifest(manifest, dek);
    var encBuf = Buffer.from(JSON.stringify(encrypted));
    var mid = Math.ceil(encBuf.length / 2);
    var halfA = encBuf.slice(0, mid);
    var halfB = encBuf.slice(mid);

    // Store each half as a blob
    var blobStore = require("../services/blobStore");
    var blobA = blobStore.putBlob(halfA);
    var blobB = blobStore.putBlob(halfB);
    if (!blobA || !blobB) {
      dek.fill(0);
      return res.status(500).json({ error: "blob store failed" });
    }

    // XOR-split the DEK
    var shares = storageCrypto.splitDEK(dek);
    var s1 = shares[0];
    var s2 = shares[1];
    dek.fill(0);

    var pairId = crypto.randomBytes(16).toString("hex");

    // Wrap shard A pointer with posting key, shard B pointer with memo key
    var ptrA = storageCrypto.wrapJSON({ blob_cid: blobA.cid, dek_share: s1.toString("hex"), pair_id: pairId, half: "A" }, postingPub);
    var ptrB = storageCrypto.wrapJSON({ blob_cid: blobB.cid, dek_share: s2.toString("hex"), pair_id: pairId, half: "B" }, memoPub);
    s1.fill(0);
    s2.fill(0);

    var shardAId = crypto.randomBytes(16).toString("hex");
    var shardBId = crypto.randomBytes(16).toString("hex");

    var epoch = stateStore.getChainHeight();
    if (epoch < 0) epoch = 0;

    // Write shard A immediately, shard B after a delay so they land in different blocks
    await ledger.recordFileShard(owner, shardAId, ptrA, epoch);
    await new Promise(function(resolve) { setTimeout(resolve, 500); });
    await ledger.recordFileShard(owner, shardBId, ptrB, epoch);

    return res.json({ ok: true, shard_a_id: shardAId, shard_b_id: shardBId, pair_id: pairId, epoch });
  } catch (e) {
    console.error("[storageRoutes] POST /shards error:", e.message);
    return res.status(500).json({ error: e.message });
  }
});

/**
 * GET /api/storage/shards?owner=<account>
 *
 * Returns all shard entries for an owner. Each entry is opaque —
 * only the key holder can decrypt the pointer inside.
 * The client iterates, tries to unwrap each with their posting or memo
 * private key, and uses pair_id to match A+B pairs locally.
 */
router.get("/shards", function (req, res) {
  try {
    var owner = req.query.owner;
    if (!owner) return res.status(400).json({ error: "owner query param required" });
    var shards = stateStore.getShardsByOwner(owner);
    return res.json({ ok: true, owner, count: shards.length, shards });
  } catch (e) {
    return res.status(500).json({ error: e.message });
  }
});

/**
 * GET /api/storage/shards/:id
 * Single shard entry by ID (opaque — caller decrypts locally).
 */
router.get("/shards/:id", function (req, res) {
  try {
    var shard = stateStore.getShard(req.params.id);
    if (!shard) return res.status(404).json({ error: "shard not found" });
    return res.json({ ok: true, shard_id: req.params.id, owner: shard.owner, encrypted_ptr: shard.encrypted_ptr, created_epoch: shard.created_epoch });
  } catch (e) {
    return res.status(500).json({ error: e.message });
  }
});

module.exports = router;
