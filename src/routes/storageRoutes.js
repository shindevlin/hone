"use strict";

/**
 * BTCPC-FS Storage Routes — v3.1.76
 * Shin Devlin
 *
 * HTTP endpoints used by the btcpc-storage daemon to submit storage
 * heartbeats into the chain via the API server. The daemon runs as a
 * separate process and must go through HTTP rather than calling
 * ledger.recordStorageHeartbeat() directly, because:
 *
 *   1. Direct in-process ledger calls only update the daemon's private
 *      stateStore instance, which the miner never sees.
 *   2. The daemon has no replay context, so its stateStore epoch is
 *      always -1 and any epochCounter it maintains drifts far below
 *      the real chain height (currently 18000+), making hosts appear
 *      stale to getActiveStorageHosts().
 *
 * By routing through the API server, the heartbeat is recorded with
 * the correct current chain epoch and flows through the shared
 * data/pending-entries.jsonl cross-process queue into the miner's
 * next block.
 *
 * Endpoints:
 *   POST /api/storage/heartbeat   — record a storage host heartbeat
 *   GET  /api/storage/hosts       — list active storage hosts (public)
 *   GET  /api/storage/hosts/:host — single host info (public)
 */

const express = require("express");
const router = express.Router();
const stateStore = require("../chain/stateStore");
const ledger = require("../services/ledger");

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
    return res.json({ ok: true, epoch: currentEpoch, count: hosts.length, hosts: hosts });
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
    return res.json({ ok: true, record: record });
  } catch (e) {
    return res.status(500).json({ error: e.message });
  }
});

module.exports = router;
