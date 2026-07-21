"use strict";

/**
 * HONE Bridge HTTP Routes — v2.16-beta
 * Shin Devlin
 *
 * REST wrapper around bridgeRegistry (v2.16-alpha) and
 * bridgeFeeDistributor (v2.16-alpha). Follows the serviceRoutes
 * pattern — auth via JWT, HTTP layer enforces input validation +
 * principal matching, registry enforces chain invariants.
 *
 * Lock-and-Recycle bridge: no burn, all fees recycle to LPs or
 * smoothing buffer. wHONE hard cap: 4.2M per chain.
 *
 * Route summary:
 *   POST   /api/bridge/wrap               wrap HONE → wHONE (auth: user)
 *   POST   /api/bridge/unwrap             unwrap wHONE → HONE (auth: user)
 *   POST   /api/bridge/fund               fund bridge as LP (auth: funder)
 *   POST   /api/bridge/unlock             request LP unlock (auth: funder)
 *   GET    /api/bridge/chains             list supported chains (public)
 *   GET    /api/bridge/chains/:id         chain config + state (public)
 *   GET    /api/bridge/fees               current fee schedule (public)
 *   GET    /api/bridge/lps               LP leaderboard by weight (public)
 *   GET    /api/bridge/lps/:funder        specific LP position (public)
 */

const express = require('express');
const router = express.Router();
const { authenticateToken } = require('../middlewares/auth');
const bridgeRegistry = require('../services/bridgeRegistry');
const bridgeFeeDistributor = require('../services/bridgeFeeDistributor');
const stateStore = require('../chain/stateStore');

// ─────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────

function sanitizeString(val, maxLen) {
  if (typeof val !== 'string') return '';
  var trimmed = val.trim();
  if (trimmed.length > maxLen) trimmed = trimmed.slice(0, maxLen);
  return trimmed;
}

// ─────────────────────────────────────────────────────────────────
// Wrap / Unwrap (authenticated mutations)
// ─────────────────────────────────────────────────────────────────

/**
 * POST /api/bridge/wrap
 * Wrap HONE → wHONE on a destination chain.
 * Body: { chainId, amount }
 * Fee is charged in HONE (source asset). Net wHONE is credited on-chain.
 */
router.post('/wrap', authenticateToken, async (req, res) => {
  try {
    const user = req.user && req.user.username;
    if (!user) return res.status(401).json({ error: 'unauthenticated' });

    const body = req.body || {};
    const chainId = sanitizeString(body.chainId, 64);
    if (!chainId) return res.status(400).json({ error: 'chainId required' });

    const amount = Number(body.amount);
    if (!Number.isFinite(amount) || amount <= 0) {
      return res.status(400).json({ error: 'amount must be a positive number' });
    }

    const caller = req.user && req.user.username;
    const userBalance = stateStore.getBalance(caller, 'HONE');
    if (userBalance < amount) {
      return res.status(402).json({
        error: 'insufficient HONE balance',
        required: amount,
        current: userBalance,
      });
    }

    try {
      const result = bridgeRegistry.wrap(user, chainId, amount);
      return res.status(201).json({ success: true, wrap: result });
    } catch (err) {
      if (/unknown chainId/i.test(err.message)) return res.status(404).json({ error: err.message });
      if (/cap/i.test(err.message)) return res.status(422).json({ error: err.message });
      return res.status(422).json({ error: err.message });
    }
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * POST /api/bridge/unwrap
 * Unwrap wHONE → HONE. No burn — wHONE transfers to bridge reserve.
 * Body: { chainId, amount }
 * Fee is charged in wHONE (source asset).
 */
router.post('/unwrap', authenticateToken, async (req, res) => {
  try {
    const user = req.user && req.user.username;
    if (!user) return res.status(401).json({ error: 'unauthenticated' });

    const body = req.body || {};
    const chainId = sanitizeString(body.chainId, 64);
    if (!chainId) return res.status(400).json({ error: 'chainId required' });

    const amount = Number(body.amount);
    if (!Number.isFinite(amount) || amount <= 0) {
      return res.status(400).json({ error: 'amount must be a positive number' });
    }

    // TODO v2.16-beta: check wHONE balance once per-user wHONE balances
    // are tracked in stateStore. For now, bridgeRegistry enforces the cap
    // on circulating supply which provides a coarse upper bound.

    try {
      const result = bridgeRegistry.unwrap(user, chainId, amount);
      return res.status(201).json({ success: true, unwrap: result });
    } catch (err) {
      if (/unknown chainId/i.test(err.message)) return res.status(404).json({ error: err.message });
      if (/exceeds circulating/i.test(err.message)) return res.status(422).json({ error: err.message });
      return res.status(422).json({ error: err.message });
    }
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * POST /api/bridge/fund
 * Fund the bridge as an LP. Locks HONE native for a chosen period.
 * Body: { chainId, amount, lockDays }
 * Weight = amount × lock_days (veCRV-style).
 */
router.post('/fund', authenticateToken, async (req, res) => {
  try {
    const funder = req.user && req.user.username;
    if (!funder) return res.status(401).json({ error: 'unauthenticated' });

    const body = req.body || {};
    const chainId = sanitizeString(body.chainId, 64);
    if (!chainId) return res.status(400).json({ error: 'chainId required' });

    const amount = Number(body.amount);
    if (!Number.isFinite(amount) || amount <= 0) {
      return res.status(400).json({ error: 'amount must be a positive number' });
    }

    const lockDays = Number(body.lockDays);
    if (!Number.isFinite(lockDays) || lockDays <= 0) {
      return res.status(400).json({ error: 'lockDays must be a positive number' });
    }

    try {
      const result = bridgeRegistry.fundBridge(funder, chainId, amount, lockDays);
      return res.status(201).json({ success: true, position: result });
    } catch (err) {
      if (/unknown chainId/i.test(err.message)) return res.status(404).json({ error: err.message });
      if (/cap/i.test(err.message)) return res.status(422).json({ error: err.message });
      if (/lockDays/i.test(err.message)) return res.status(400).json({ error: err.message });
      return res.status(422).json({ error: err.message });
    }
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * POST /api/bridge/unlock
 * Request LP unlock. Adds funder to the FIFO withdrawal queue if
 * their lock period has expired. Funders in queue continue earning fees.
 * Body: { chainId }
 */
router.post('/unlock', authenticateToken, async (req, res) => {
  try {
    const funder = req.user && req.user.username;
    if (!funder) return res.status(401).json({ error: 'unauthenticated' });

    const body = req.body || {};
    const chainId = sanitizeString(body.chainId, 64);
    if (!chainId) return res.status(400).json({ error: 'chainId required' });

    try {
      const result = bridgeRegistry.requestUnlock(funder, chainId);
      return res.json({ success: true, unlock: result });
    } catch (err) {
      if (/unknown chainId/i.test(err.message)) return res.status(404).json({ error: err.message });
      if (/no lock found/i.test(err.message)) return res.status(404).json({ error: err.message });
      return res.status(422).json({ error: err.message });
    }
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// ─────────────────────────────────────────────────────────────────
// Read endpoints (public)
// ─────────────────────────────────────────────────────────────────

/**
 * GET /api/bridge/chains
 * List all supported destination chains with config + current state.
 */
router.get('/chains', (req, res) => {
  const chains = bridgeRegistry.getAllChains();
  res.json({ chains, count: chains.length });
});

/**
 * GET /api/bridge/chains/:id
 * Get a single chain config + state.
 */
router.get('/chains/:id', (req, res) => {
  const chainId = req.params.id;
  const chain = bridgeRegistry.getChain(chainId);
  if (!chain) return res.status(404).json({ error: 'chain not found' });
  res.json({ chain });
});

/**
 * GET /api/bridge/fees
 * Current fee schedule: wrap flat rate + tiered unwrap rates.
 */
router.get('/fees', (req, res) => {
  const schedule = {
    wrap: {
      rate_bps: bridgeFeeDistributor.WRAP_FEE_BPS,
      rate_pct: bridgeFeeDistributor.WRAP_FEE_BPS / 100,
      description: 'flat 0.05% on all wraps, charged in HONE',
    },
    unwrap: {
      tiers: [
        {
          label: 'small',
          threshold_max: bridgeFeeDistributor.UNWRAP_SMALL_THRESHOLD,
          rate_bps: bridgeFeeDistributor.UNWRAP_SMALL_BPS,
          rate_pct: bridgeFeeDistributor.UNWRAP_SMALL_BPS / 100,
          description: 'amounts < ' + bridgeFeeDistributor.UNWRAP_SMALL_THRESHOLD + ' HONE',
        },
        {
          label: 'mid',
          threshold_min: bridgeFeeDistributor.UNWRAP_SMALL_THRESHOLD,
          threshold_max: bridgeFeeDistributor.UNWRAP_LARGE_THRESHOLD,
          rate_bps: bridgeFeeDistributor.UNWRAP_MID_BPS,
          rate_pct: bridgeFeeDistributor.UNWRAP_MID_BPS / 100,
          description: bridgeFeeDistributor.UNWRAP_SMALL_THRESHOLD + '-' + bridgeFeeDistributor.UNWRAP_LARGE_THRESHOLD + ' HONE',
        },
        {
          label: 'large',
          threshold_min: bridgeFeeDistributor.UNWRAP_LARGE_THRESHOLD,
          rate_bps: bridgeFeeDistributor.UNWRAP_LARGE_BPS,
          rate_pct: bridgeFeeDistributor.UNWRAP_LARGE_BPS / 100,
          description: 'amounts > ' + bridgeFeeDistributor.UNWRAP_LARGE_THRESHOLD + ' HONE',
        },
      ],
      fee_currency: 'wHONE',
    },
    smoothing_buffer: {
      fraction: bridgeFeeDistributor.SMOOTHING_FRACTION,
      description: 'fraction of fees routed to smoothing buffer (not burned — recycled)',
    },
    note: 'No burn. All fees recycle to LPs or smoothing buffer.',
  };
  res.json({ fees: schedule });
});

/**
 * GET /api/bridge/lps
 * LP leaderboard sorted by current weight (descending).
 * Query: ?chainId= (required to scope by chain)
 */
router.get('/lps', (req, res) => {
  const chainId = sanitizeString(req.query.chainId || '', 64);
  if (!chainId) return res.status(400).json({ error: 'chainId query param required' });
  const chain = bridgeRegistry.getChain(chainId);
  if (!chain) return res.status(404).json({ error: 'chain not found' });

  const allFunders = bridgeRegistry.getAllFunders(chainId);
  // Sort by current_weight descending
  allFunders.sort(function (a, b) { return (b.current_weight || 0) - (a.current_weight || 0); });

  res.json({ chain_id: chainId, lps: allFunders, count: allFunders.length });
});

/**
 * GET /api/bridge/lps/:funder
 * Get a specific LP's position across all chains, or for a specific chain
 * if ?chainId= is provided.
 */
router.get('/lps/:funder', (req, res) => {
  const funder = sanitizeString(req.params.funder, 64);
  if (!funder) return res.status(400).json({ error: 'funder required' });

  const chainId = req.query.chainId ? sanitizeString(req.query.chainId, 64) : null;

  var chains = chainId ? [bridgeRegistry.getChain(chainId)].filter(Boolean) : bridgeRegistry.getAllChains();
  if (chainId && chains.length === 0) return res.status(404).json({ error: 'chain not found' });

  var positions = [];
  chains.forEach(function (chain) {
    var weight = bridgeRegistry.getLPWeight(funder, chain.chain_id);
    var allFunders = bridgeRegistry.getAllFunders(chain.chain_id);
    var position = allFunders.find(function (f) { return f.funder === funder; });
    if (position) {
      positions.push(Object.assign({}, position, { current_weight: weight }));
    }
  });

  if (positions.length === 0) return res.status(404).json({ error: 'no LP position found for ' + funder });
  res.json({ funder, positions, count: positions.length });
});

module.exports = router;
