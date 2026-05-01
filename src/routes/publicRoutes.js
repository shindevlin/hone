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
const { createAccountForUser } = require('../services/accountCreation');

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

const publicAuthLimiter = rateLimit({
  windowMs: 15 * 60 * 1000,
  max: 20,
  standardHeaders: true,
  legacyHeaders: false,
  message: { error: 'Too many login attempts. Try again later.' },
});

const publicSignupLimiter = rateLimit({
  windowMs: 60 * 60 * 1000,
  max: 5,
  standardHeaders: true,
  legacyHeaders: false,
  message: { error: 'Too many account creation attempts. Try again later.' },
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
 * GET /public/machine-status — live status of this machine's btcpc processes.
 * No auth required — intended for the desktop app's Node tab.
 */
router.get('/machine-status', async (req, res) => {
  try {
    const os = require('os');
    const fs = require('fs');
    const path = require('path');
    const axios = require('axios');

    const uptimeSec = Math.round(os.uptime());
    const loadAvg = os.loadavg()[0].toFixed(2);
    const freeMem = Math.round(os.freemem() / 1024 / 1024);
    const totalMem = Math.round(os.totalmem() / 1024 / 1024);

    // Detect running btcpc processes by scanning /proc/*/cmdline.
    // Only role names are returned — no PIDs or memory stats.
    const btcpcRoles = {
      'src/index.js': 'api',
      'btcpc-mine': 'mine',
      'btcpc-clock': 'clock',
      'btcpc-storage': 'storage',
      'btcpc-chain-monitor': 'chain-monitor',
      'btcpc-auto-update': 'auto-update',
      'btcpc-gnss-bridge': 'gnss-bridge',
      'btcpc-nebra': 'nebra',
      'btcpc-all': 'all',
      'btcpc-gateway': 'gateway',
      'btcpc-verifier': 'verifier',
    };
    const btcpcBins = Object.keys(btcpcRoles);
    const runningRoles = new Set();
    function matchesBtcpcProcess(parts, binName) {
      if (!parts.length) return false;
      if (parts[0] === binName || path.basename(parts[0]) === binName) return true;
      const exe = path.basename(parts[0]);
      if (exe === 'node' || exe === 'nodejs') {
        return parts.slice(1, 3).some(part => part === binName || path.basename(part) === binName);
      }
      return false;
    }
    try {
      const procs = fs.readdirSync('/proc').filter(f => /^\d+$/.test(f));
      for (const pid of procs) {
        try {
          const cmd = fs.readFileSync('/proc/' + pid + '/cmdline', 'utf8').replace(/\0/g, ' ').trim();
          const parts = cmd.split(/\s+/).filter(Boolean);
          const match = btcpcBins.find(b => matchesBtcpcProcess(parts, b));
          if (match) runningRoles.add(btcpcRoles[match]);
        } catch (_) {}
      }
    } catch (_) {}

    // Check if Ollama is reachable — return boolean only, not the model list.
    let ollamaRunning = false;
    try {
      const ollamaUrl = process.env.OLLAMA_URL || 'http://localhost:11434';
      await axios.get(ollamaUrl + '/api/tags', { timeout: 3000 });
      ollamaRunning = true;
    } catch (_) {}

    // Chain height from block files
    let chainHeight = 0;
    try {
      const blocksDir = path.join(process.cwd(), 'data', 'blocks');
      const files = fs.readdirSync(blocksDir).filter(f => f.startsWith('block-') && f.endsWith('.bin'));
      if (files.length > 0) {
        files.sort();
        chainHeight = parseInt(files[files.length - 1].replace('block-', '').replace('.bin', ''), 10);
      }
    } catch (_) {}

    res.json({
      uptime_sec: uptimeSec,
      load_avg: parseFloat(loadAvg),
      mem_free_mb: freeMem,
      mem_total_mb: totalMem,
      processes: Array.from(runningRoles).map(role => ({ role })),
      ollama: { running: ollamaRunning },
      chain_height: chainHeight,
      timestamp: Date.now(),
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
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

    // Read latest block from disk (source of truth for chain state).
    // Prefer index.json (updated every epoch) over block-*.bin files (pruned after finality).
    const blocksDir = path.join(process.cwd(), 'data', 'blocks');
    let latestEpoch = 0;
    let latestMtimeMs = 0;
    try {
      const indexPath = path.join(blocksDir, 'index.json');
      const indexStat = fs.statSync(indexPath);
      const indexData = JSON.parse(fs.readFileSync(indexPath, 'utf8'));
      const keys = Object.keys(indexData).map(Number).sort((a, b) => a - b);
      if (keys.length > 0) {
        latestEpoch = keys[keys.length - 1];
        latestMtimeMs = indexStat.mtimeMs;
      }
    } catch (_) {
      // Fallback to block files if index missing
      try {
        const files = fs.readdirSync(blocksDir).filter(f => f.startsWith('block-') && f.endsWith('.bin'));
        if (files.length > 0) {
          files.sort();
          const last = files[files.length - 1];
          latestEpoch = parseInt(last.replace('block-', '').replace('.bin', ''), 10);
          const stat = fs.statSync(path.join(blocksDir, last));
          latestMtimeMs = stat.mtimeMs;
        }
      } catch (_2) {}
    }

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

    // Active browser clocks (heartbeated within the last 2 min via HTTP)
    const browserClocks = getActiveBrowserClocks();

    // Live P2P clock nodes — miners acting as clocks + Android phones sending
    // CLOCK_HEARTBEAT via the relay. Both paths end up in clockUptimeByEpoch.
    // These are often NOT in nodeRegistry (which only has on-chain registered nodes).
    const p2pClockAccounts = new Set();
    try {
      const protocol = require('../p2p/protocol');
      if (typeof protocol.getActiveClockNodes === 'function') {
        const p2pClocks = protocol.getActiveClockNodes(latestEpoch);
        p2pClocks.forEach(a => p2pClockAccounts.add(a));
      }
    } catch (_) {}

    // Merge all clock sources into one deduplicated set
    const activeClockAccounts = new Set();
    browserClocks.forEach(c => { if (c.account) activeClockAccounts.add(c.account); });
    p2pClockAccounts.forEach(a => activeClockAccounts.add(a));
    // Also count any nodeRegistry-registered clock nodes
    registered.filter(n => n.type === 'clock').forEach(n => {
      if (n.account) activeClockAccounts.add(n.account);
    });

    // Network is "alive" if the latest block file was written in the last 30 minutes
    const epochAgeMs = latestMtimeMs > 0 ? Date.now() - latestMtimeMs : Infinity;
    const alive = epochAgeMs < 30 * 60 * 1000;

    // Distinct node count — each unique account is one "node" regardless of roles
    const uniqueAccounts = new Set();
    registered.forEach(n => { if (n.account) uniqueAccounts.add(n.account); });
    browserClocks.forEach(c => { if (c.account) uniqueAccounts.add(c.account); });
    p2pClockAccounts.forEach(a => uniqueAccounts.add(a));

    // Role breakdown
    const storageNodes = registered.filter(n => n.type === 'storage').length;
    const gatewayNodes = registered.filter(n => n.type === 'gateway').length;
    const verifierNodes = registered.filter(n => n.type === 'verifier').length;

    res.json({
      epoch: latestEpoch,
      nodes: uniqueAccounts.size,
      peer_count: registered.length + browserClocks.length + p2pClockAccounts.size,
      miners,
      clocks: activeClockAccounts.size,
      storage: storageNodes,
      gateways: gatewayNodes,
      verifiers: verifierNodes,
      browser_clocks: browserClocks.length,
      p2p_clocks: p2pClockAccounts.size,
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

    // Verify account exists on-chain (stateStore is the canonical source
    // of truth). Previously this checked Mongo User.findOne which meant
    // fresh machines or post-mongo-incident state would 404 a valid
    // on-chain account. See feedback_blockchain_source_of_truth.md.
    const stateStore = require('../chain/stateStore');
    const onChainAccount = stateStore.getAccount(account);
    if (!onChainAccount) {
      // Fall back to Mongo only if stateStore hasn't been hydrated yet
      // (e.g., during early replay before the account's block was processed).
      try {
        const User = require('../models/User');
        const mongoUser = await User.findOne({ username: account });
        if (!mongoUser) return res.status(404).json({ error: 'Account not found' });
      } catch (_) {
        return res.status(404).json({ error: 'Account not found' });
      }
    }

    // Optional JWT verification: if present, mark this heartbeat as verified
    // and confirm the JWT subject matches the claimed account.
    let verified = false;
    const authHeader = req.headers.authorization;
    if (authHeader && authHeader.startsWith('Bearer ')) {
      try {
        const jwt = require('jsonwebtoken');
        const token = authHeader.slice(7);
        const decoded = jwt.verify(token, process.env.JWT_SECRET);
        if (decoded && decoded.username === account) {
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

    let credited = false;
    if (verified) {
      // Track this client as an active browser clock only when it is
      // cryptographically linked to the account.
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
        credited = true;
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
    }

    res.json({
      success: true,
      account,
      epoch: epochNumber,
      client_id: clientId,
      verified,
      credited,
      active_browser_clocks: activeBrowserClocks.size,
      next_heartbeat_in: 35,
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * GET /public/leaderboard — top 20 accounts by BTCPC balance.
 * Returns role breakdown (miner, clock, storage, etc.) for each account.
 * No auth required.
 */
router.get('/leaderboard', async (_req, res) => {
  try {
    const stateStore = require('../chain/stateStore');
    const nodeRegistry = require('../chain/nodeRegistry');

    const allAccounts = stateStore.getAllAccounts();
    const entries = allAccounts.map(function(acc) {
      var balance = stateStore.getBalance(acc.username, 'BTCPC');
      return { username: acc.username, balance: balance, created_epoch: acc.created_epoch };
    });

    entries.sort(function(a, b) { return b.balance - a.balance; });
    var top = entries.slice(0, 20);

    if (typeof nodeRegistry.loadFromBlocks === 'function' && nodeRegistry.getNodeCount() === 0) {
      try { nodeRegistry.loadFromBlocks(); } catch (_e) {}
    }
    var registered = nodeRegistry.getRegisteredNodes();

    var leaderboard = top.map(function(entry) {
      var roles = [];
      for (var i = 0; i < registered.length; i++) {
        if (registered[i].account === entry.username && registered[i].type) {
          if (roles.indexOf(registered[i].type) === -1) roles.push(registered[i].type);
        }
      }
      return {
        account: entry.username,
        balance: entry.balance,
        roles: roles,
        created_epoch: entry.created_epoch,
      };
    });

    res.json({
      leaderboard: leaderboard,
      total_accounts: allAccounts.length,
      timestamp: Date.now(),
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * POST /public/signup — create account from PWA (no bot key needed)
 * Body: { username, password }
 * Returns: { success, username, mnemonic, public_keys, balance }
 */
router.post('/signup', publicSignupLimiter, async (req, res) => {
  try {
    const { username, password } = req.body || {};
    if (!username || typeof username !== 'string' || username.length < 3) {
      return res.status(400).json({ error: 'username must be at least 3 characters' });
    }
    if (!password || typeof password !== 'string' || password.length < 8) {
      return res.status(400).json({ error: 'password must be at least 8 characters' });
    }
    const result = await createAccountForUser({
      username,
      password,
      requirePassword: false,
      claimFaucet: true,
    });

    const jwt = require('jsonwebtoken');
    const jwtSecret = process.env.JWT_SECRET || process.env.BTCPC_JWT_SECRET;
    const token = jwtSecret
      ? jwt.sign({ username: result.username, src: 'public' }, jwtSecret, { expiresIn: '30d' })
      : null;

    res.json({
      success: true,
      username: result.username,
      mnemonic: result.mnemonic,
      mnemonic_warning: 'SAVE YOUR MNEMONIC AND PRIVATE KEYS NOW. They will never be shown again.',
      token,
      wallet_export: result.wallet_export,
      wallet_rows: result.wallet_rows,
      public_keys: result.public_keys,
      role_private_keys: result.role_private_keys,
      wallet_balance: result.wallet_balance,
      balance: result.wallet_balance,
      delegated_balance: result.delegated_balance,
      faucet_claimed: result.faucet_claimed,
      faucet_note: result.faucet_note,
      chain_addresses: result.chain_addresses,
      chain_wallets: result.chain_wallets,
    });
  } catch (err) {
    res.status(err.statusCode || 500).json({ error: err.message });
  }
});

/**
 * POST /public/login — authenticate from PWA (no bot key needed)
 * Body: { username, password }
 */
router.post('/login', publicAuthLimiter, async (req, res) => {
  try {
    const { username, password } = req.body || {};
    if (!username || !password) return res.status(400).json({ error: 'username and password required' });

    const secretStore = require('../services/secretStore');
    try { await secretStore.load(); } catch (_) {}

    // Check if user exists first
    const clean = username.trim().toLowerCase();
    const user = secretStore.getUser(clean);
    if (!user) return res.status(404).json({ error: 'account not found' });

    const ok = await secretStore.verifyPassword(clean, password);
    if (!ok) return res.status(401).json({ error: 'wrong password' });

    const jwt = require('jsonwebtoken');
    const jwtSecret = process.env.JWT_SECRET || process.env.BTCPC_JWT_SECRET;
    if (!jwtSecret) return res.status(503).json({ error: 'JWT secret not initialized' });
    const token = jwt.sign({ username: username.trim().toLowerCase() }, jwtSecret, { expiresIn: '30d' });

    const stateStore = require('../chain/stateStore');
    const balance = stateStore.getBalance(username.trim().toLowerCase(), 'BTCPC');

    res.json({ success: true, username: username.trim().toLowerCase(), token, balance });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * POST /public/change-password
 * Body: { username, old_password, new_password }
 */
router.post('/change-password', publicAuthLimiter, async (req, res) => {
  try {
    const { username, old_password, new_password } = req.body || {};
    if (!username || !old_password || !new_password) {
      return res.status(400).json({ error: 'username, old_password, and new_password required' });
    }
    if (new_password.length < 8) {
      return res.status(400).json({ error: 'new password must be at least 8 characters' });
    }

    const secretStore = require('../services/secretStore');
    try { await secretStore.load(); } catch (_) {}

    const clean = username.trim().toLowerCase();
    const user = secretStore.getUser(clean);
    if (!user) return res.status(404).json({ error: 'account not found' });

    const ok = await secretStore.verifyPassword(clean, old_password);
    if (!ok) return res.status(401).json({ error: 'current password is wrong' });

    // Hash new password and update
    const bcrypt = require('bcryptjs');
    const hash = await bcrypt.hash(new_password, 10);
    secretStore.updateUser(clean, { password_hash: hash });

    res.json({ success: true, message: 'Password changed' });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// ---------------------------------------------------------------------------
// Model demand — users request models, miners see what to pull
// ---------------------------------------------------------------------------

const modelRequestLimiter = rateLimit({
  windowMs: 60 * 1000,
  max: 5,
  standardHeaders: true,
  legacyHeaders: false,
  message: { error: 'Too many model requests. Wait a minute.' },
});

/**
 * POST /public/model-request
 * Body: { model, account }
 * Stores demand in memory, broadcasts via P2P, returns total demand.
 */
router.post('/model-request', modelRequestLimiter, async (req, res) => {
  try {
    const objErr = rejectObjectInputs(req.body, ['model', 'account']);
    if (objErr) return res.status(400).json({ error: objErr });

    const model = sanitizeString(req.body.model, 100);
    const account = sanitizeString(req.body.account, 20);

    if (!model || model.length < 2) {
      return res.status(400).json({ error: 'model name required (e.g. "llama3.3:70b")' });
    }

    // Record demand locally
    const { addModelDemand, getModelDemand, createMessage } = require('../p2p/protocol');
    addModelDemand(model, account || 'anonymous');

    // Broadcast via P2P so all nodes track this demand
    try {
      const p2p = require('../p2p/network');
      const msg = createMessage('MODEL_DEMAND', {
        model: model,
        account: account || 'anonymous',
      }, p2p.NODE_ID || 'api-relay');
      if (typeof p2p.broadcast === 'function') p2p.broadcast(msg);
    } catch (_) {}

    // Find current demand for this model
    const allDemand = getModelDemand();
    const thisDemand = allDemand.find(d => d.model === model.trim().toLowerCase());

    res.json({
      success: true,
      model: model.trim().toLowerCase(),
      total_demand: thisDemand ? thisDemand.count : 1,
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * GET /public/model-demand
 * Returns top 20 most-requested models with demand counts.
 * Used by miners to decide what to pull.
 */
router.get('/model-demand', async (_req, res) => {
  try {
    const { getModelDemand } = require('../p2p/protocol');
    const demand = getModelDemand();
    res.json({
      demand: demand,
      timestamp: Date.now(),
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});


// ---------------------------------------------------------------------------
// Device Registry — track all devices under one account
// ---------------------------------------------------------------------------

var deviceRegistry = new Map(); // account → [{ device_id, name, type, roles, last_seen, earnings, model, status }]

/**
 * POST /public/device-heartbeat
 * Each device pings this every 30s with its status.
 */
router.post('/device-heartbeat', async (req, res) => {
  try {
    var body = req.body || {};
    var account = (body.account || '').trim().toLowerCase();
    var deviceId = body.device_id || 'unknown';
    if (!account) return res.status(400).json({ error: 'account required' });

    var devices = deviceRegistry.get(account) || [];
    var existing = devices.find(d => d.device_id === deviceId);

    var update = {
      device_id: deviceId,
      name: body.name || deviceId,
      type: body.type || 'unknown', // phone, desktop, pi, flipper
      roles: body.roles || [],
      model: body.model || null,
      status: body.status || 'active',
      sensors: body.sensors || 0,
      earnings: body.earnings || 0,
      last_seen: Date.now(),
      ip: (req.ip || '').replace('::ffff:', ''),
    };

    if (existing) {
      Object.assign(existing, update);
    } else {
      devices.push(update);
    }
    deviceRegistry.set(account, devices);

    res.json({ success: true, devices: devices.length });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * GET /public/my-devices?account=xxx
 * Returns all devices registered under an account.
 */
router.get('/my-devices', async (req, res) => {
  try {
    var account = ((req.query.account || '') + '').trim().toLowerCase();
    if (!account) return res.status(400).json({ error: 'account required' });

    var devices = deviceRegistry.get(account) || [];
    // Mark stale devices (no heartbeat in 5 min)
    var now = Date.now();
    devices.forEach(d => {
      d.online = (now - d.last_seen) < 300000;
    });

    res.json({ account: account, devices: devices, timestamp: now });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// Clean stale devices every 10 min
setInterval(function() {
  var cutoff = Date.now() - 3600000; // 1 hour
  for (var [acct, devs] of deviceRegistry) {
    deviceRegistry.set(acct, devs.filter(d => d.last_seen > cutoff));
    if (deviceRegistry.get(acct).length === 0) deviceRegistry.delete(acct);
  }
}, 600000);

/**
 * GET /public/android-version
 * Returns the latest Android APK version info.
 * version_code is an integer; app compares with BuildConfig.VERSION_CODE.
 */
const ANDROID_RELEASE = {
  version_code: 3,
  version_name: require('../../package.json').version,
  apk_url: 'https://btcpc.net/public/android-apk',
  changelog: 'Full sensor array (GPS, battery, temperature, humidity + 5 more), model picker from chain registry, storage quota, on-chain credit for all earn activities, auto-restart on update',
  min_sdk: 26,
};

router.get('/android-version', function(req, res) {
  res.json(ANDROID_RELEASE);
});

/**
 * GET /downloads/btcpc-android.apk — serve the latest release APK.
 * File is placed at website/downloads/btcpc-android.apk by the release script.
 */
router.get('/android-apk', function(req, res) {
  const path = require('path');
  const fs = require('fs');
  const apkPath = path.join(process.cwd(), 'website', 'downloads', 'btcpc-android.apk');
  if (!fs.existsSync(apkPath)) {
    return res.status(404).json({ error: 'APK not yet published — check btcpc.net/downloads' });
  }
  res.setHeader('Content-Type', 'application/vnd.android.package-archive');
  res.setHeader('Content-Disposition', 'attachment; filename="btcpc-android.apk"');
  fs.createReadStream(apkPath).pipe(res);
});

// ---------------------------------------------------------------------------
// Onboarding agent — unauthenticated, streaming SSE
// ---------------------------------------------------------------------------

const agentChatLimiter = rateLimit({
  windowMs: 60 * 1000,
  max: 12,
  standardHeaders: true,
  legacyHeaders: false,
  message: { error: 'Too many messages. Wait a minute.' },
});

const AGENT_SYSTEM_PROMPT = `You are the BTCPC setup assistant on btcpc.net. BTCPC is a sovereign blockchain where miners earn by running AI inference via Ollama — no gatekeepers, no cloud.

Keep every reply under 3 sentences. Be direct and actionable. Give exact commands when asked.

Platform setup:
- Windows: download start-windows.bat from btcpc.net, double-click it. Handles Ollama binding and Docker automatically.
- Linux / Ubuntu / WSL: download start.sh from btcpc.net, run: bash start.sh
- Docker only (advanced): set OLLAMA_URL=http://host.docker.internal:11434 in .env, then: docker compose up -d
- Android: install the BTCPC app from btcpc.net/android

Requirements: Docker Desktop (Windows/Mac) or Docker Engine (Linux), plus Ollama installed on the host.
Recommended first model: qwen3:4b (run: ollama pull qwen3:4b)

Mining starts automatically once the node is running and a model is loaded.
Explorer: btcpc.net/explorer — wallet: btcpc.net/app`;

/**
 * POST /public/agent-chat
 * Body: { message: string, platform?: string }
 * Streams the assistant reply as SSE: data: <token>\n\n, then data: [DONE]\n\n
 */
router.post('/agent-chat', agentChatLimiter, async (req, res) => {
  try {
    const objErr = rejectObjectInputs(req.body, ['message', 'platform']);
    if (objErr) return res.status(400).json({ error: objErr });

    const message = sanitizeString(req.body.message, 500);
    const platform = sanitizeString(req.body.platform || '', 40);

    if (!message || message.length < 1) {
      return res.status(400).json({ error: 'message required' });
    }

    const axios = require('axios');
    const OLLAMA_URL = process.env.OLLAMA_URL || 'http://localhost:11434';

    const userContent = platform ? `[User platform: ${platform}]\n${message}` : message;

    const body = {
      model: process.env.BTCPC_MODEL || 'qwen3:4b',
      messages: [
        { role: 'system', content: AGENT_SYSTEM_PROMPT },
        { role: 'user', content: userContent },
      ],
      stream: true,
    };

    res.setHeader('Content-Type', 'text/event-stream');
    res.setHeader('Cache-Control', 'no-cache');
    res.setHeader('Connection', 'keep-alive');
    res.flushHeaders();

    const ollamaRes = await axios.post(`${OLLAMA_URL}/api/chat`, body, {
      responseType: 'stream',
      timeout: 30000,
    });

    ollamaRes.data.on('data', (chunk) => {
      const lines = chunk.toString().split('\n').filter(Boolean);
      for (const line of lines) {
        try {
          const parsed = JSON.parse(line);
          const token = parsed?.message?.content;
          if (token) res.write(`data: ${JSON.stringify(token)}\n\n`);
          if (parsed?.done) res.write('data: [DONE]\n\n');
        } catch (_) {}
      }
    });

    ollamaRes.data.on('end', () => {
      res.write('data: [DONE]\n\n');
      res.end();
    });

    ollamaRes.data.on('error', () => {
      res.write('data: [DONE]\n\n');
      res.end();
    });

    req.on('close', () => ollamaRes.data.destroy());
  } catch (err) {
    if (!res.headersSent) {
      res.status(503).json({ error: 'Agent unavailable — node may be starting up.' });
    } else {
      res.write('data: [DONE]\n\n');
      res.end();
    }
  }
});

module.exports = router;
