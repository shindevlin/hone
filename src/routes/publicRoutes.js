"use strict";

/**
 * Public routes — no API key required, CORS open.
 * Used by browser-based clock nodes and other zero-config integrations.
 *
 * Security note: rate-limited heavily, broadcasts only — no DB writes
 * other than the ephemeral activity tracker.
 */
const express = require('express');
const crypto = require('crypto');
const router = express.Router();
const rateLimit = require('express-rate-limit');

const {
  rejectObjectInputs, sanitizeString, validAccountName,
} = require('../middlewares/validate');

// In-memory tracker of active browser-clock heartbeats.
// Map<clientId, { account, lastHeartbeat, ip }>
// Active = heartbeat within the last 2 minutes.
const activeBrowserClocks = new Map();
const CLOCK_ACTIVE_WINDOW_MS = 2 * 60 * 1000;

function pruneActiveClocks() {
  const now = Date.now();
  for (const [id, entry] of activeBrowserClocks) {
    if (now - entry.lastHeartbeat > CLOCK_ACTIVE_WINDOW_MS) {
      activeBrowserClocks.delete(id);
    }
  }
}

function getActiveBrowserClocks() {
  pruneActiveClocks();
  return Array.from(activeBrowserClocks.values());
}

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
    const nodeRegistry = require('../chain/nodeRegistry');

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

    // Registered nodes from the P2P-synchronized registry (source of truth)
    let registered = [];
    let miners = 0;
    let registeredClocks = 0;
    try {
      // Lazy-load registry from blocks if empty (API server may not have loaded yet)
      if (nodeRegistry.getNodeCount() === 0 && typeof nodeRegistry.loadFromBlocks === 'function') {
        try { nodeRegistry.loadFromBlocks(); } catch (_) {}
      }
      registered = nodeRegistry.getRegisteredNodes();
      miners = registered.filter(n => n.type === 'miner').length;
      registeredClocks = registered.filter(n => n.type === 'clock').length;
    } catch (_) {}

    // Active browser clocks (heartbeated within the last 2 min)
    const browserClocks = getActiveBrowserClocks();
    const activeClockAccounts = new Set(browserClocks.map(c => c.account));

    // Network is "alive" if the latest block file was written in the last 30 minutes
    const epochAgeMs = latestMtimeMs > 0 ? Date.now() - latestMtimeMs : Infinity;
    const alive = epochAgeMs < 30 * 60 * 1000;

    res.json({
      epoch: latestEpoch,
      peer_count: registered.length + browserClocks.length,
      miners,
      clocks: registeredClocks + browserClocks.length,
      browser_clocks: browserClocks.length,
      active_clock_accounts: Array.from(activeClockAccounts),
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
    const objErr = rejectObjectInputs(req.body, ['account', 'client_id', 'verified']);
    if (objErr) return res.status(400).json({ error: objErr });

    const account = sanitizeString(req.body.account, 20);
    if (!account || !validAccountName(account)) {
      return res.status(400).json({ error: 'valid account name required' });
    }

    // Verify account exists
    const User = require('../models/User');
    const user = await User.findOne({ username: account });
    if (!user) return res.status(404).json({ error: 'Account not found' });

    // Optional JWT verification: if present, mark this heartbeat as verified
    // and confirm the JWT subject matches the claimed account.
    let verified = false;
    const authHeader = req.headers.authorization;
    if (authHeader && authHeader.startsWith('Bearer ')) {
      try {
        const jwt = require('jsonwebtoken');
        const token = authHeader.slice(7);
        const decoded = jwt.verify(token, process.env.JWT_SECRET);
        if (decoded && (decoded.username === account || decoded.id === user._id.toString())) {
          verified = true;
        }
      } catch (_) {
        // Invalid JWT — just don't mark verified, don't reject the heartbeat
      }
    }

    // Client uniqueness: caller provides a stable client_id (stored in localStorage),
    // or we generate one and return it. Same client_id within the active window =
    // same node, no double-counting. Different client_id = different participant.
    let clientId = sanitizeString(req.body.client_id, 64);
    if (!clientId || !/^[a-f0-9]{32,64}$/.test(clientId)) {
      clientId = crypto.randomBytes(16).toString('hex');
    }

    // Read latest epoch from blocks
    const fs = require('fs');
    const path = require('path');
    let epochNumber = 0;
    try {
      const blocksDir = path.join(process.cwd(), 'data', 'blocks');
      const files = fs.readdirSync(blocksDir).filter(f => f.startsWith('block-') && f.endsWith('.bin'));
      if (files.length > 0) {
        files.sort();
        epochNumber = parseInt(files[files.length - 1].replace('block-', '').replace('.bin', ''), 10);
      }
    } catch (_) {}

    // Track this client as an active browser clock (for the dashboard)
    activeBrowserClocks.set(clientId, {
      account,
      lastHeartbeat: Date.now(),
      ip: req.ip,
      verified,
    });
    pruneActiveClocks();

    // ALSO record into the P2P clock tracker so the miner sees this clock
    // when computing rewards. Without this, browser/HTTP clocks don't earn.
    try {
      const { recordNodeActivity } = require('../p2p/protocol');
      recordNodeActivity(clientId, account, epochNumber);
    } catch (_) {}

    // Broadcast P2P heartbeat
    try {
      const p2p = require('../p2p/network');
      const { createMessage } = require('../p2p/protocol');
      const heartbeat = createMessage('CLOCK_HEARTBEAT', {
        account,
        epoch_number: epochNumber,
        source: 'browser',
        client_id: clientId,
      }, p2p.NODE_ID || 'browser-relay');
      if (typeof p2p.broadcast === 'function') p2p.broadcast(heartbeat);
    } catch (_) {}

    res.json({
      success: true,
      account,
      epoch: epochNumber,
      client_id: clientId,
      verified,
      active_browser_clocks: activeBrowserClocks.size,
      next_heartbeat_in: 35,
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

module.exports = router;
