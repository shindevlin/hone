"use strict";

/**
 * BTCPC-FS Serve Proof Routes — v2.11.1
 * Shin Devlin
 *
 * HTTP endpoints for storage hosts to submit bandwidth serve proofs.
 * Separate from blobRoutes.js so upload/download (v2.11.0) and serve
 * proof submission (v2.11.1) can ship independently.
 *
 *   POST /api/blob-serve/:cid          submit a serve proof for one CID
 *   POST /api/blob-serve/flush-all     flush all in-memory accumulated counts
 *   GET  /api/blob-serve/:cid/pending  inspect pending in-memory counter
 *   GET  /api/blob-serve/pending       inspect all pending counters
 *
 * Mount at /api/blob-serve in src/index.js:
 *     app.use("/api/blob-serve", require("./routes/blobServeProofRoutes"));
 *
 * The serve-proof submission flow:
 *   1. Storage host counts bytes served (via blobBandwidth.accumulate
 *      or via external nginx/cloudfront logs)
 *   2. Host POSTs /api/blob-serve/:cid with { bytes_served, request_count }
 *      OR POSTs /api/blob-serve/flush-all to flush the in-process accumulator
 *   3. Chain invariants enforced at the stateStore dispatcher level:
 *      - CID must have a prior BLOB_STORE_COMMIT
 *      - Authenticated host must be in the committed hosts list
 *      - bytes_served must be non-negative
 *   4. Chain records the proof, bytes_served_by_host[host] accumulates
 *   5. At payout time, blobPayouts.settlePayouts distributes pro-rata
 *      based on bytes_served_by_host
 */

const express = require('express');
const router = express.Router();
const { authenticateToken } = require('../middlewares/auth');
const blobStore = require('../services/blobStore');
const stateStore = require('../chain/stateStore');
const ledger = require('../services/ledger');
const blobBandwidth = require('../services/blobBandwidth');

/**
 * POST /api/blob-serve/flush-all
 * Flush ALL pending in-memory serve counts to chain at once.
 * MUST be declared BEFORE POST /:cid so Express matches the literal path
 * first and doesn't treat "flush-all" as a CID parameter.
 */
router.post('/flush-all', authenticateToken, async (req, res) => {
  try {
    const host = req.user && req.user.username;
    if (!host) return res.status(401).json({ error: 'unauthenticated' });
    const result = await blobBandwidth.flushServeProofs(host);
    res.json(result);
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/** GET /api/blob-serve/pending — all pending counts (must be before /:cid/pending) */
router.get('/pending', (req, res) => {
  res.json({
    pending: blobBandwidth.getPending(),
    pending_cid_count: blobBandwidth.pendingCidCount(),
  });
});

/**
 * POST /api/blob-serve/:cid
 * Body: { bytes_served, request_count?, access_log_merkle_root? }
 *
 * Host-submitted serve proof. Can be called explicitly with counts from
 * external log parsing, OR with an empty body to flush the in-process
 * accumulator for this CID.
 */
router.post('/:cid', authenticateToken, express.json(), async (req, res) => {
  try {
    const host = req.user && req.user.username;
    if (!host) return res.status(401).json({ error: 'unauthenticated' });

    const cid = req.params.cid;
    if (!blobStore.isValidCid(cid)) {
      return res.status(400).json({ error: 'invalid CID format' });
    }

    const commit = stateStore.getBlobCommit(cid);
    if (!commit) {
      return res.status(404).json({ error: 'no commit on chain for this CID', cid });
    }
    if (commit.hosts.indexOf(host) === -1) {
      return res.status(403).json({
        error: 'account is not a committed host for this CID',
        host,
        committed_hosts: commit.hosts,
      });
    }

    let bytesServed;
    let requestCount;
    let accessLogMerkleRoot = null;

    if (req.body && typeof req.body.bytes_served === 'number') {
      // Explicit submission — use body values
      bytesServed = req.body.bytes_served;
      requestCount = Number.isFinite(req.body.request_count) ? req.body.request_count : 0;
      accessLogMerkleRoot = typeof req.body.access_log_merkle_root === 'string'
        ? req.body.access_log_merkle_root
        : null;
    } else {
      // No body — flush the in-process accumulator
      const pending = blobBandwidth.getPending(cid);
      bytesServed = pending.bytes;
      requestCount = pending.requests;
      if (bytesServed <= 0) {
        return res.json({
          cid,
          flushed_bytes: 0,
          flushed_requests: 0,
          note: 'nothing to flush',
        });
      }
    }

    if (!Number.isFinite(bytesServed) || bytesServed < 0) {
      return res.status(400).json({ error: 'bytes_served must be a non-negative number' });
    }
    if (bytesServed === 0) {
      return res.json({
        cid,
        flushed_bytes: 0,
        flushed_requests: 0,
        note: 'zero bytes — no-op',
      });
    }

    const epoch = await ledger.getCurrentEpoch();
    await ledger.recordBlobServeProof(
      host,
      cid,
      bytesServed,
      requestCount,
      accessLogMerkleRoot,
      epoch
    );

    // If we pulled from the accumulator, clear it after successful record
    if (!req.body || typeof req.body.bytes_served !== 'number') {
      blobBandwidth.clearPending(cid);
    }

    res.json({
      cid,
      host,
      flushed_bytes: bytesServed,
      flushed_requests: requestCount,
      commit_after: stateStore.getBlobCommit(cid),
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/** GET /api/blob-serve/:cid/pending — pending counts for one CID */
router.get('/:cid/pending', (req, res) => {
  const cid = req.params.cid;
  if (!blobStore.isValidCid(cid)) {
    return res.status(400).json({ error: 'invalid CID format' });
  }
  const pending = blobBandwidth.getPending(cid);
  res.json({ cid, pending });
});

module.exports = router;
