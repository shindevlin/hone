"use strict";

/**
 * Public routes — no API key required, CORS open.
 * Used by browser-based clock nodes and other zero-config integrations.
 *
 * Security note: rate-limited heavily, broadcasts only — no DB writes
 * other than the ephemeral activity tracker.
 */
const express = require('express');
const router = express.Router();
const rateLimit = require('express-rate-limit');

const {
  rejectObjectInputs, sanitizeString, validAccountName,
} = require('../middlewares/validate');

// Heavy rate limit on public clock endpoints — once every 30s per IP
const clockLimiter = rateLimit({
  windowMs: 30 * 1000,
  max: 1,
  standardHeaders: true,
  legacyHeaders: false,
  message: { error: 'Heartbeat too frequent. Wait 30s between heartbeats.' },
});

// CORS for browser access
router.use((req, res, next) => {
  res.header('Access-Control-Allow-Origin', '*');
  res.header('Access-Control-Allow-Methods', 'GET, POST, OPTIONS');
  res.header('Access-Control-Allow-Headers', 'Content-Type');
  if (req.method === 'OPTIONS') return res.sendStatus(204);
  next();
});

/**
 * GET /public/network — public network status for browser clock UI
 *
 * Reports the height of the chain and whether the network is producing
 * blocks recently. peer_count is derived from the most recent finalized
 * epoch's reported active node count, not the local API's P2P state
 * (which may be 0 even if the network is healthy).
 */
router.get('/network', async (req, res) => {
  try {
    const fs = require('fs');
    const path = require('path');
    const Node = require('../models/Node');

    // Read latest block from disk (source of truth for chain state)
    const blocksDir = path.join(process.cwd(), 'data', 'blocks');
    let latestEpoch = 0;
    let latestMtimeMs = 0;
    try {
      const files = fs.readdirSync(blocksDir).filter(f => f.startsWith('block-') && f.endsWith('.bin'));
      if (files.length > 0) {
        files.sort();
        const last = files[files.length - 1];
        latestEpoch = parseInt(last.replace('block-', '').replace('.bin', ''), 10);
        const stat = fs.statSync(path.join(blocksDir, last));
        latestMtimeMs = stat.mtimeMs;
      }
    } catch (_) {}

    // Count active mining nodes
    let activeNodes = 0;
    try { activeNodes = await Node.countDocuments({ status: 'active' }); } catch (_) {}

    // Network is "alive" if the latest block file was written in the last 30 minutes
    const epochAgeMs = latestMtimeMs > 0 ? Date.now() - latestMtimeMs : Infinity;
    const alive = epochAgeMs < 30 * 60 * 1000;

    res.json({
      epoch: latestEpoch,
      peer_count: activeNodes,
      alive,
      epoch_age_seconds: Math.round(epochAgeMs / 1000),
      timestamp: Date.now(),
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * POST /public/clock-heartbeat
 * Body: { account: "username" }
 * Browser-based clock nodes call this to participate in epoch timing.
 * The server broadcasts a CLOCK_HEARTBEAT P2P message on their behalf.
 *
 * Limited to 1 heartbeat per 30s per IP.
 */
router.post('/clock-heartbeat', clockLimiter, async (req, res) => {
  try {
    const objErr = rejectObjectInputs(req.body, ['account']);
    if (objErr) return res.status(400).json({ error: objErr });

    const account = sanitizeString(req.body.account, 20);
    if (!account || !validAccountName(account)) {
      return res.status(400).json({ error: 'valid account name required' });
    }

    // Verify account exists
    const User = require('../models/User');
    const user = await User.findOne({ username: account });
    if (!user) return res.status(404).json({ error: 'Account not found' });

    // Get current epoch and broadcast heartbeat
    const Epoch = require('../models/Epoch');
    const latestEpoch = await Epoch.findOne({}, null, { sort: { epoch_number: -1 } });
    const epochNumber = latestEpoch?.epoch_number || 0;

    const p2p = require('../p2p/network');
    const { createMessage } = require('../p2p/protocol');
    const heartbeat = createMessage('CLOCK_HEARTBEAT', {
      account,
      epoch_number: epochNumber,
      source: 'browser',
    }, p2p.NODE_ID);

    if (typeof p2p.broadcast === 'function') {
      p2p.broadcast(heartbeat);
    }

    res.json({
      success: true,
      account,
      epoch: epochNumber,
      next_heartbeat_in: 30,
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

module.exports = router;
