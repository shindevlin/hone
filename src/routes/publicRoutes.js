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
 */
router.get('/network', async (req, res) => {
  try {
    const Epoch = require('../models/Epoch');
    const latestEpoch = await Epoch.findOne({}, null, { sort: { epoch_number: -1 } });
    const p2p = require('../p2p/network');
    res.json({
      epoch: latestEpoch?.epoch_number || 0,
      epoch_status: latestEpoch?.status || 'unknown',
      peer_count: typeof p2p.peerCount === 'function' ? p2p.peerCount() : 0,
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
