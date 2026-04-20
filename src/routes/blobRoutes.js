"use strict";

/**
 * BTCPC-FS Blob Routes — v2.11.0
 * Shin Devlin
 *
 * HTTP endpoints for the content-addressed blob store.
 *
 *   POST /api/blobs            upload a blob (auth required)
 *   GET  /api/blobs/:cid       download by CID (public)
 *   HEAD /api/blobs/:cid       check existence + size (public)
 *   GET  /api/blobs            list blobs on this host (public, paginated)
 *   GET  /api/blobs/:cid/info  metadata about a CID including chain commits
 *
 * v2.11.0 scope:
 *   - Uploader is the sole host (no multi-host replication yet)
 *   - No escrow-paid hosting (payment = 0)
 *   - No bandwidth accounting (comes in v2.11.1)
 *   - No challenge-response (comes in v2.11.2)
 */

const express = require('express');
const router = express.Router();
const { authenticateToken } = require('../middlewares/auth');
const blobStore = require('../services/blobStore');
const stateStore = require('../chain/stateStore');
const ledger = require('../services/ledger');

const DEFAULT_DURATION_EPOCHS = 10000;

/**
 * POST /api/blobs
 * Body: raw binary (application/octet-stream or any type)
 */
router.post(
  '/',
  authenticateToken,
  express.raw({ type: '*/*', limit: blobStore.MAX_BLOB_BYTES }),
  async (req, res) => {
    try {
      const uploader = req.user && req.user.username;
      if (!uploader) return res.status(401).json({ error: 'unauthenticated' });

      const body = req.body;
      if (!body || body.length === 0) {
        return res.status(400).json({ error: 'empty body' });
      }
      if (body.length > blobStore.MAX_BLOB_BYTES) {
        return res.status(413).json({
          error: 'blob exceeds max size',
          max_bytes: blobStore.MAX_BLOB_BYTES,
          actual_bytes: body.length,
        });
      }

      const result = blobStore.putBlob(body);

      const epoch = await ledger.getCurrentEpoch();
      await ledger.recordBlobStoreCommit(
        uploader,
        result.cid,
        result.size,
        [uploader],
        DEFAULT_DURATION_EPOCHS,
        0,
        epoch
      );

      res.status(result.existed ? 200 : 201).json({
        cid: result.cid,
        size: result.size,
        existed: result.existed,
        committed: true,
        retrieval_url: '/api/blobs/' + result.cid,
        gateway_url: '/fs/' + result.cid,
      });
    } catch (err) {
      if (err.message && err.message.indexOf('exceeds max') >= 0) {
        return res.status(413).json({ error: err.message });
      }
      res.status(500).json({ error: err.message });
    }
  }
);

/**
 * POST /api/blobs/stream — streaming upload for large files (no memory limit).
 * Body: raw binary stream. Auth required via ?token= or Authorization header.
 */
router.post(
  '/stream',
  authenticateToken,
  async (req, res) => {
    try {
      const uploader = req.user && req.user.username;
      if (!uploader) return res.status(401).json({ error: 'unauthenticated' });
      const result = await blobStore.putBlobStream(req);
      const epoch = await ledger.getCurrentEpoch();
      await ledger.recordBlobStoreCommit(uploader, result.cid, result.size, [uploader], DEFAULT_DURATION_EPOCHS, 0, epoch);
      res.status(result.existed ? 200 : 201).json({
        cid: result.cid,
        size: result.size,
        existed: result.existed,
        committed: true,
        retrieval_url: '/api/blobs/' + result.cid,
        gateway_url: '/fs/' + result.cid,
      });
    } catch (err) {
      res.status(500).json({ error: err.message });
    }
  }
);

/** GET /api/blobs/:cid — stream the blob */
router.get('/:cid', (req, res) => {
  try {
    const cid = req.params.cid;
    if (!blobStore.isValidCid(cid)) {
      return res.status(400).json({ error: 'invalid CID format' });
    }
    if (!blobStore.hasBlob(cid)) {
      return res.status(404).json({ error: 'blob not found on this host', cid });
    }
    const buf = blobStore.getBlob(cid);
    res.set('Content-Type', 'application/octet-stream');
    res.set('Content-Length', String(buf.length));
    res.set('X-BTCPC-CID', cid);
    res.set('Cache-Control', 'public, max-age=31536000, immutable');
    res.send(buf);
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/** HEAD /api/blobs/:cid — existence + size without body */
router.head('/:cid', (req, res) => {
  const cid = req.params.cid;
  if (!blobStore.isValidCid(cid)) {
    return res.status(400).end();
  }
  const stat = blobStore.statBlob(cid);
  if (!stat) {
    return res.status(404).end();
  }
  res.set('Content-Length', String(stat.size));
  res.set('X-BTCPC-CID', cid);
  res.set('X-BTCPC-MTime', String(stat.mtime));
  res.status(200).end();
});

/** GET /api/blobs/:cid/info — local + chain metadata */
router.get('/:cid/info', (req, res) => {
  const cid = req.params.cid;
  if (!blobStore.isValidCid(cid)) {
    return res.status(400).json({ error: 'invalid CID format' });
  }
  const localStat = blobStore.statBlob(cid);
  const chainCommit = stateStore.getBlobCommit(cid);

  if (!localStat && !chainCommit) {
    return res.status(404).json({ error: 'unknown CID', cid });
  }

  res.json({
    cid,
    local: localStat
      ? { stored: true, size: localStat.size, mtime: localStat.mtime }
      : { stored: false },
    chain: chainCommit || null,
  });
});

/** GET /api/blobs — paginated list */
router.get('/', (req, res) => {
  try {
    const page = Math.max(1, parseInt(req.query.page, 10) || 1);
    const limit = Math.max(1, Math.min(200, parseInt(req.query.limit, 10) || 50));
    const all = blobStore.listBlobs();
    const total = all.length;
    const start = (page - 1) * limit;
    const blobs = all.slice(start, start + limit).map((b) => ({
      cid: b.cid,
      size: b.size,
      retrieval_url: '/api/blobs/' + b.cid,
    }));
    res.json({
      blobs,
      page,
      limit,
      total,
      total_bytes: blobStore.totalBytesStored(),
      max_blob_bytes: blobStore.MAX_BLOB_BYTES,
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

module.exports = router;
