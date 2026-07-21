"use strict";

/**
 * HONE Oracle Feed HTTP Routes — v2.13-delta / v2.16-beta
 * Shin Devlin
 *
 * REST wrapper around oracleFeeds (v2.13-delta). Follows the
 * serviceRoutes pattern — auth via JWT, HTTP layer enforces input
 * validation + principal matching, oracleFeeds enforces chain
 * invariants.
 *
 * Feed ID format: "<owner>/<name>" — URL-encoded in path params.
 *
 * Route summary:
 *   POST   /api/oracles/feeds                register feed (auth: owner)
 *   POST   /api/oracles/feeds/:id/reports    submit report (auth: verifier)
 *   POST   /api/oracles/feeds/:id/finalize   finalize epoch (auth: any, idempotent)
 *   POST   /api/oracles/feeds/:id/retire     retire feed (auth: owner)
 *   GET    /api/oracles/feeds                list feeds (public, filterable)
 *   GET    /api/oracles/feeds/:id            feed + current value + active hosts
 */

const express = require('express');
const router = express.Router();
const { authenticateToken } = require('../middlewares/auth');
const oracleFeeds = require('../services/oracleFeeds');

// ─────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────

function decodeId(raw) {
  try {
    return decodeURIComponent(raw);
  } catch (_) {
    return null;
  }
}

function sanitizeString(val, maxLen) {
  if (typeof val !== 'string') return '';
  var trimmed = val.trim();
  if (trimmed.length > maxLen) trimmed = trimmed.slice(0, maxLen);
  return trimmed;
}

function sanitizePagination(pageRaw, limitRaw, maxLimit) {
  var page = parseInt(pageRaw, 10);
  var limit = parseInt(limitRaw, 10);
  if (!Number.isFinite(page) || page < 1) page = 1;
  if (!Number.isFinite(limit) || limit < 1) limit = 20;
  if (limit > maxLimit) limit = maxLimit;
  return { page: page, limit: limit };
}

async function getCurrentEpoch() {
  try {
    var ledger = require('../services/ledger');
    if (ledger && typeof ledger.getCurrentEpoch === 'function') {
      var e = await ledger.getCurrentEpoch();
      if (Number.isFinite(e)) return e;
    }
  } catch (_) {}
  return 0;
}

// ─────────────────────────────────────────────────────────────────
// Feed management
// ─────────────────────────────────────────────────────────────────

/**
 * POST /api/oracles/feeds
 * Register a new oracle feed. The authenticated user becomes the owner.
 * Body: { name, description, unit, decimals, update_interval_epochs,
 *         min_verifiers, max_deviation_bps, verifiers? }
 * feed_id is computed server-side as "<owner>/<name>".
 */
router.post('/feeds', authenticateToken, async (req, res) => {
  try {
    const owner = req.user && req.user.username;
    if (!owner) return res.status(401).json({ error: 'unauthenticated' });

    const body = req.body || {};
    const name = sanitizeString(body.name, 63);
    if (!name) return res.status(400).json({ error: 'name required' });

    const feedId = owner + '/' + name;

    const spec = {
      description: sanitizeString(body.description, 512),
      unit: sanitizeString(body.unit, 32),
      decimals: body.decimals,
      update_interval_epochs: body.update_interval_epochs,
      min_verifiers: body.min_verifiers,
      max_deviation_bps: body.max_deviation_bps,
    };

    if (!spec.description) return res.status(400).json({ error: 'description required' });
    if (!spec.unit) return res.status(400).json({ error: 'unit required' });

    if (Array.isArray(body.verifiers)) {
      spec.verifiers = body.verifiers
        .filter(function (v) { return typeof v === 'string' && v.length > 0; })
        .map(function (v) { return sanitizeString(v, 64); })
        .slice(0, 1000);
    }

    const options = { epoch: await getCurrentEpoch() };

    try {
      const record = oracleFeeds.registerFeed(owner, feedId, spec, options);
      return res.status(201).json({ success: true, feed: record });
    } catch (err) {
      if (/already/i.test(err.message)) return res.status(409).json({ error: err.message });
      return res.status(422).json({ error: err.message });
    }
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * POST /api/oracles/feeds/:id/reports
 * Submit a verifier report for a feed + epoch.
 * Auth: any authenticated user (registry enforces allowlist if set).
 * Body: { value, epoch? }
 */
router.post('/feeds/:id/reports', authenticateToken, async (req, res) => {
  try {
    const verifier = req.user && req.user.username;
    if (!verifier) return res.status(401).json({ error: 'unauthenticated' });

    const feedId = decodeId(req.params.id);
    if (!feedId) return res.status(400).json({ error: 'invalid feed id encoding' });

    const body = req.body || {};
    if (body.value === undefined || body.value === null) {
      return res.status(400).json({ error: 'value required' });
    }
    const numeric = Number(body.value);
    if (!Number.isFinite(numeric)) {
      return res.status(400).json({ error: 'value must be a finite number' });
    }

    // Use provided epoch or current
    var epoch;
    if (body.epoch !== undefined) {
      epoch = parseInt(body.epoch, 10);
      if (!Number.isFinite(epoch) || epoch < 0) {
        return res.status(400).json({ error: 'epoch must be a non-negative integer' });
      }
    } else {
      epoch = await getCurrentEpoch();
    }

    try {
      const report = oracleFeeds.submitReport(verifier, feedId, numeric, epoch);
      return res.status(201).json({ success: true, report: report });
    } catch (err) {
      if (/not found/i.test(err.message)) return res.status(404).json({ error: err.message });
      if (/not in allowlist/i.test(err.message)) return res.status(403).json({ error: err.message });
      if (/already reported/i.test(err.message)) return res.status(409).json({ error: err.message });
      if (/already finalized/i.test(err.message)) return res.status(409).json({ error: err.message });
      if (/not active/i.test(err.message)) return res.status(409).json({ error: err.message });
      return res.status(422).json({ error: err.message });
    }
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * POST /api/oracles/feeds/:id/finalize
 * Finalize an epoch for a feed — compute median, flag outliers.
 * Auth: any authenticated user (idempotent — calling twice returns same record).
 * Body: { epoch? }
 */
router.post('/feeds/:id/finalize', authenticateToken, async (req, res) => {
  try {
    const caller = req.user && req.user.username;
    if (!caller) return res.status(401).json({ error: 'unauthenticated' });

    const feedId = decodeId(req.params.id);
    if (!feedId) return res.status(400).json({ error: 'invalid feed id encoding' });

    const body = req.body || {};
    var epoch;
    if (body.epoch !== undefined) {
      epoch = parseInt(body.epoch, 10);
      if (!Number.isFinite(epoch) || epoch < 0) {
        return res.status(400).json({ error: 'epoch must be a non-negative integer' });
      }
    } else {
      epoch = await getCurrentEpoch();
    }

    try {
      const finalization = oracleFeeds.finalizeFeedEpoch(feedId, epoch);
      return res.json({ success: true, finalization: finalization });
    } catch (err) {
      if (/not found/i.test(err.message)) return res.status(404).json({ error: err.message });
      if (/insufficient reports/i.test(err.message)) return res.status(422).json({ error: err.message });
      return res.status(422).json({ error: err.message });
    }
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * POST /api/oracles/feeds/:id/retire
 * Retire a feed. Only the owner can retire.
 */
router.post('/feeds/:id/retire', authenticateToken, async (req, res) => {
  try {
    const owner = req.user && req.user.username;
    if (!owner) return res.status(401).json({ error: 'unauthenticated' });

    const feedId = decodeId(req.params.id);
    if (!feedId) return res.status(400).json({ error: 'invalid feed id encoding' });

    const epoch = await getCurrentEpoch();

    try {
      const record = oracleFeeds.retireFeed(owner, feedId, epoch);
      return res.json({ success: true, feed: record });
    } catch (err) {
      if (/not found/i.test(err.message)) return res.status(404).json({ error: err.message });
      if (/only the owner/i.test(err.message)) return res.status(403).json({ error: err.message });
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
 * GET /api/oracles/feeds
 * List feeds. Public, paginated, filterable by owner/status.
 */
router.get('/feeds', (req, res) => {
  const { page, limit } = sanitizePagination(req.query.page, req.query.limit, 200);
  const filter = {};
  if (req.query.owner) filter.owner = sanitizeString(req.query.owner, 64);
  if (req.query.status) filter.status = sanitizeString(req.query.status, 16);
  const all = oracleFeeds.getAllFeeds(filter);
  const total = all.length;
  const start = (page - 1) * limit;
  const feeds = all.slice(start, start + limit);
  res.json({ feeds, page, limit, total });
});

/**
 * GET /api/oracles/feeds/:id
 * Get a feed + its current value + reports for a given epoch.
 * Query: ?epoch= to fetch reports for a specific epoch.
 */
router.get('/feeds/:id', async (req, res) => {
  const feedId = decodeId(req.params.id);
  if (!feedId) return res.status(400).json({ error: 'invalid feed id encoding' });
  const feed = oracleFeeds.getFeed(feedId);
  if (!feed) return res.status(404).json({ error: 'feed not found' });

  const currentEpoch = await getCurrentEpoch();
  const queryEpoch = req.query.epoch !== undefined
    ? parseInt(req.query.epoch, 10)
    : currentEpoch;

  const reports = oracleFeeds.getReportsForEpoch(feedId, queryEpoch);
  const finalization = oracleFeeds.getFinalization(feedId, queryEpoch);
  const currentValue = oracleFeeds.getFeedValue(feedId);

  res.json({
    feed,
    current_value: currentValue,
    current_epoch: currentEpoch,
    query_epoch: queryEpoch,
    reports,
    finalization,
  });
});

module.exports = router;
