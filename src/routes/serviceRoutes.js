"use strict";

/**
 * BTCPC Service Hosting HTTP Routes — v2.13-gamma
 * Shin Devlin
 *
 * REST wrapper around serviceRegistry (v2.13-alpha). Sellers deploy
 * stateless services, hosts heartbeat to prove uptime, users start
 * sessions against active hosts. Mirrors the commerce route structure
 * — auth via JWT, chain invariants enforced in the registry, HTTP
 * layer enforces input validation + authenticated principal matching.
 *
 * Route summary:
 *   POST   /api/services                          deploy (deployer = req.user)
 *   PUT    /api/services/:slug                    update runtime (only deployer)
 *   POST   /api/services/:slug/retire             retire (only deployer)
 *   POST   /api/services/:slug/heartbeat          host reports uptime
 *   POST   /api/services/:slug/sessions           user starts session
 *   POST   /api/services/sessions/:sessionId/end  user or host ends session
 *   GET    /api/services                          list (public, paginated)
 *   GET    /api/services/my                       authenticated user's services
 *   GET    /api/services/:slug                    service + hosts + sessions
 *   GET    /api/services/:slug/hosts              heartbeat records
 *   GET    /api/services/:slug/sessions           active sessions
 *   GET    /api/services/sessions/:sessionId      single session
 *
 * Slug format: "<deployer>/<name>" — the slash means :slug must be
 * URL-encoded by the client (same convention as commerce productId).
 *
 * No direct registry writes are exposed — all mutations flow through
 * serviceRegistry functions which validate inputs and maintain the
 * in-memory mirror that (in a later phase) will be driven by ledger
 * entries via stateStore.
 */

const express = require('express');
const router = express.Router();
const crypto = require('crypto');
const { authenticateToken } = require('../middlewares/auth');
const serviceRegistry = require('../services/serviceRegistry');

// ─────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────

const VALID_RUNTIME_TYPES = ['http', 'tcp', 'wasm', 'static'];

function decodeSlug(raw) {
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

function sanitizeRuntime(raw) {
  if (!raw || typeof raw !== 'object') return { error: 'runtime required' };
  var type = sanitizeString(raw.type, 16);
  if (VALID_RUNTIME_TYPES.indexOf(type) < 0) {
    return { error: 'runtime.type must be one of: ' + VALID_RUNTIME_TYPES.join(', ') };
  }
  var cid = sanitizeString(raw.binary_cid, 128);
  if (!/^[a-f0-9]{64}$/.test(cid)) {
    return { error: 'runtime.binary_cid must be 64-char lowercase hex sha256' };
  }

  var runtime = {
    type: type,
    binary_cid: cid,
    entrypoint: sanitizeString(raw.entrypoint, 256) || null,
    command: sanitizeString(raw.command, 256) || null,
  };

  if (Array.isArray(raw.args)) {
    runtime.args = raw.args
      .map(function (a) { return sanitizeString(a, 256); })
      .filter(Boolean)
      .slice(0, 32);
  }

  if (raw.cpu !== undefined) {
    var cpu = parseFloat(raw.cpu);
    if (!Number.isFinite(cpu) || cpu <= 0 || cpu > 256) {
      return { error: 'runtime.cpu must be a positive number <= 256' };
    }
    runtime.cpu = cpu;
  }

  if (raw.ram_mb !== undefined) {
    var ram = parseInt(raw.ram_mb, 10);
    if (!Number.isFinite(ram) || ram <= 0 || ram > 1024 * 1024) {
      return { error: 'runtime.ram_mb must be a positive integer' };
    }
    runtime.ram_mb = ram;
  }

  if (raw.port !== undefined && raw.port !== null) {
    var port = parseInt(raw.port, 10);
    if (!Number.isFinite(port) || port < 1 || port > 65535) {
      return { error: 'runtime.port must be 1-65535' };
    }
    runtime.port = port;
  } else {
    runtime.port = null;
  }

  if (raw.env && typeof raw.env === 'object' && !Array.isArray(raw.env)) {
    var env = {};
    var keys = Object.keys(raw.env).slice(0, 64);
    for (var i = 0; i < keys.length; i++) {
      var k = sanitizeString(keys[i], 64);
      var v = sanitizeString(raw.env[keys[i]], 1024);
      if (k) env[k] = v;
    }
    runtime.env = env;
  } else {
    runtime.env = null;
  }

  return { ok: true, runtime: runtime };
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
// Deploy / update / retire (deployer-only)
// ─────────────────────────────────────────────────────────────────

/**
 * POST /api/services
 * Deploy a new service. The authenticated user becomes the deployer.
 * Body: { name, runtime, min_replicas?, price_per_hour?, price_token? }
 * Slug is computed server-side as "<deployer>/<name>".
 */
router.post('/', authenticateToken, async (req, res) => {
  try {
    const deployer = req.user && req.user.username;
    if (!deployer) return res.status(401).json({ error: 'unauthenticated' });

    const name = sanitizeString(req.body && req.body.name, 63);
    if (!name) return res.status(400).json({ error: 'name required' });

    const rtCheck = sanitizeRuntime(req.body && req.body.runtime);
    if (!rtCheck.ok) return res.status(400).json({ error: rtCheck.error });

    const slug = deployer + '/' + name;

    const options = { epoch: await getCurrentEpoch() };
    if (req.body.min_replicas !== undefined) {
      const mr = parseInt(req.body.min_replicas, 10);
      if (!Number.isFinite(mr) || mr < 1 || mr > 1000) {
        return res.status(400).json({ error: 'min_replicas must be 1-1000' });
      }
      options.min_replicas = mr;
    }
    if (req.body.price_per_hour !== undefined) {
      const pph = parseFloat(req.body.price_per_hour);
      if (!Number.isFinite(pph) || pph < 0) {
        return res.status(400).json({ error: 'price_per_hour must be >= 0' });
      }
      options.price_per_hour = pph;
    }
    if (req.body.price_token !== undefined) {
      options.price_token = sanitizeString(req.body.price_token, 16) || 'BTCPC';
    }

    try {
      const record = serviceRegistry.deploy(deployer, slug, rtCheck.runtime, options);
      return res.status(201).json({ success: true, service: record });
    } catch (err) {
      if (/already/i.test(err.message)) return res.status(409).json({ error: err.message });
      return res.status(422).json({ error: err.message });
    }
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * PUT /api/services/:slug
 * Update a deployed service's runtime spec (binary bump, price change).
 * Only the original deployer can update.
 */
router.put('/:slug', authenticateToken, async (req, res) => {
  try {
    const deployer = req.user && req.user.username;
    if (!deployer) return res.status(401).json({ error: 'unauthenticated' });

    const slug = decodeSlug(req.params.slug);
    if (!slug) return res.status(400).json({ error: 'invalid slug encoding' });

    const existing = serviceRegistry.getService(slug);
    if (!existing) return res.status(404).json({ error: 'service not found' });
    if (existing.deployer !== deployer) {
      return res.status(403).json({ error: 'only the deployer can update this service' });
    }

    const newRuntime = req.body && req.body.runtime
      ? sanitizeRuntime(req.body.runtime)
      : { ok: true, runtime: existing.runtime };
    if (!newRuntime.ok) return res.status(400).json({ error: newRuntime.error });

    const options = { epoch: await getCurrentEpoch() };
    if (req.body.min_replicas !== undefined) {
      const mr = parseInt(req.body.min_replicas, 10);
      if (!Number.isFinite(mr) || mr < 1 || mr > 1000) {
        return res.status(400).json({ error: 'min_replicas must be 1-1000' });
      }
      options.min_replicas = mr;
    }
    if (req.body.price_per_hour !== undefined) {
      const pph = parseFloat(req.body.price_per_hour);
      if (!Number.isFinite(pph) || pph < 0) {
        return res.status(400).json({ error: 'price_per_hour must be >= 0' });
      }
      options.price_per_hour = pph;
    }

    const record = serviceRegistry.deploy(deployer, slug, newRuntime.runtime, options);
    res.json({ success: true, service: record });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * POST /api/services/:slug/retire
 * Retire a service. Only the deployer can retire.
 */
router.post('/:slug/retire', authenticateToken, async (req, res) => {
  try {
    const deployer = req.user && req.user.username;
    if (!deployer) return res.status(401).json({ error: 'unauthenticated' });

    const slug = decodeSlug(req.params.slug);
    if (!slug) return res.status(400).json({ error: 'invalid slug encoding' });

    try {
      const record = serviceRegistry.retire(deployer, slug, await getCurrentEpoch());
      res.json({ success: true, service: record });
    } catch (err) {
      if (/not found/i.test(err.message)) return res.status(404).json({ error: err.message });
      if (/only the deployer/i.test(err.message)) return res.status(403).json({ error: err.message });
      return res.status(422).json({ error: err.message });
    }
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// ─────────────────────────────────────────────────────────────────
// Heartbeat (host-only)
// ─────────────────────────────────────────────────────────────────

/**
 * POST /api/services/:slug/heartbeat
 * Host reports that it is serving this service. Body may include
 * port_addr + state_hash so clients can connect.
 */
router.post('/:slug/heartbeat', authenticateToken, async (req, res) => {
  try {
    const host = req.user && req.user.username;
    if (!host) return res.status(401).json({ error: 'unauthenticated' });

    const slug = decodeSlug(req.params.slug);
    if (!slug) return res.status(400).json({ error: 'invalid slug encoding' });

    const options = { epoch: await getCurrentEpoch() };
    if (req.body && req.body.port_addr !== undefined) {
      options.port_addr = sanitizeString(req.body.port_addr, 128) || null;
    }
    if (req.body && req.body.state_hash !== undefined) {
      options.state_hash = sanitizeString(req.body.state_hash, 128) || null;
    }

    try {
      const record = serviceRegistry.heartbeat(host, slug, options);
      res.json({ success: true, heartbeat: record });
    } catch (err) {
      if (/not found/i.test(err.message)) return res.status(404).json({ error: err.message });
      if (/not active/i.test(err.message)) return res.status(409).json({ error: err.message });
      return res.status(422).json({ error: err.message });
    }
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// ─────────────────────────────────────────────────────────────────
// Sessions
// ─────────────────────────────────────────────────────────────────

/**
 * POST /api/services/:slug/sessions
 * Start a session against a specific host serving the service.
 * Body: { host, max_duration_epochs?, escrow_id? }
 */
router.post('/:slug/sessions', authenticateToken, async (req, res) => {
  try {
    const user = req.user && req.user.username;
    if (!user) return res.status(401).json({ error: 'unauthenticated' });

    const slug = decodeSlug(req.params.slug);
    if (!slug) return res.status(400).json({ error: 'invalid slug encoding' });

    const host = sanitizeString(req.body && req.body.host, 64);
    if (!host) return res.status(400).json({ error: 'host required' });

    const sessionId = 'sess-' + crypto.randomBytes(8).toString('hex');

    const options = { epoch: await getCurrentEpoch() };
    if (req.body.max_duration_epochs !== undefined) {
      const mde = parseInt(req.body.max_duration_epochs, 10);
      if (!Number.isFinite(mde) || mde < 1 || mde > 100000) {
        return res.status(400).json({ error: 'max_duration_epochs must be 1-100000' });
      }
      options.max_duration_epochs = mde;
    }
    if (req.body.escrow_id !== undefined) {
      options.escrow_id = sanitizeString(req.body.escrow_id, 128) || null;
    }

    try {
      const session = serviceRegistry.startSession(user, host, slug, sessionId, options);
      res.status(201).json({ success: true, session: session });
    } catch (err) {
      if (/not found/i.test(err.message)) return res.status(404).json({ error: err.message });
      if (/not active/i.test(err.message)) return res.status(409).json({ error: err.message });
      return res.status(422).json({ error: err.message });
    }
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * POST /api/services/sessions/:sessionId/end
 * End an active session. Either the user or host can end it.
 */
router.post('/sessions/:sessionId/end', authenticateToken, async (req, res) => {
  try {
    const party = req.user && req.user.username;
    if (!party) return res.status(401).json({ error: 'unauthenticated' });

    const sessionId = req.params.sessionId;
    try {
      const session = serviceRegistry.endSession(party, sessionId, await getCurrentEpoch());
      res.json({ success: true, session: session });
    } catch (err) {
      if (/not found/i.test(err.message)) return res.status(404).json({ error: err.message });
      if (/only the user or host/i.test(err.message)) return res.status(403).json({ error: err.message });
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
 * GET /api/services
 * List services. Public, paginated, filterable by status/deployer/type.
 */
router.get('/', (req, res) => {
  const { page, limit } = sanitizePagination(req.query.page, req.query.limit, 200);
  const filter = {};
  if (req.query.status) filter.status = sanitizeString(req.query.status, 16);
  if (req.query.deployer) filter.deployer = sanitizeString(req.query.deployer, 64);
  if (req.query.type) filter.type = sanitizeString(req.query.type, 16);
  const all = serviceRegistry.getAllServices(filter);
  const total = all.length;
  const start = (page - 1) * limit;
  const services = all.slice(start, start + limit);
  res.json({ services, page, limit, total });
});

/**
 * GET /api/services/my
 * List services deployed by the authenticated user.
 */
router.get('/my', authenticateToken, (req, res) => {
  const deployer = req.user && req.user.username;
  if (!deployer) return res.status(401).json({ error: 'unauthenticated' });
  const services = serviceRegistry.getServicesByDeployer(deployer);
  res.json({ services, count: services.length });
});

/**
 * GET /api/services/sessions/:sessionId
 * Get a single session by id. Must come BEFORE /:slug below so that
 * a path like "/sessions/sess-abc" doesn't match :slug.
 */
router.get('/sessions/:sessionId', (req, res) => {
  const session = serviceRegistry.getSession(req.params.sessionId);
  if (!session) return res.status(404).json({ error: 'session not found' });
  res.json({ session });
});

/**
 * GET /api/services/:slug/hosts
 * List active heartbeat records for a service.
 * Query: ?recent_epochs=100
 */
router.get('/:slug/hosts', async (req, res) => {
  const slug = decodeSlug(req.params.slug);
  if (!slug) return res.status(400).json({ error: 'invalid slug encoding' });
  if (!serviceRegistry.getService(slug)) {
    return res.status(404).json({ error: 'service not found' });
  }
  const recentEpochs = parseInt(req.query.recent_epochs, 10) || 100;
  const currentEpoch = await getCurrentEpoch();
  const hosts = serviceRegistry.getActiveHostsForService(slug, currentEpoch, recentEpochs);
  res.json({ hosts, count: hosts.length, current_epoch: currentEpoch });
});

/**
 * GET /api/services/:slug/sessions
 * List active sessions for a service.
 */
router.get('/:slug/sessions', (req, res) => {
  const slug = decodeSlug(req.params.slug);
  if (!slug) return res.status(400).json({ error: 'invalid slug encoding' });
  if (!serviceRegistry.getService(slug)) {
    return res.status(404).json({ error: 'service not found' });
  }
  const sessions = serviceRegistry.getActiveSessions(slug);
  res.json({ sessions, count: sessions.length });
});

/**
 * GET /api/services/:slug
 * Get a service + its active hosts + active sessions.
 */
router.get('/:slug', async (req, res) => {
  const slug = decodeSlug(req.params.slug);
  if (!slug) return res.status(400).json({ error: 'invalid slug encoding' });
  const service = serviceRegistry.getService(slug);
  if (!service) return res.status(404).json({ error: 'service not found' });
  const currentEpoch = await getCurrentEpoch();
  const hosts = serviceRegistry.getActiveHostsForService(slug, currentEpoch, 100);
  const sessions = serviceRegistry.getActiveSessions(slug);
  res.json({ service, hosts, sessions, current_epoch: currentEpoch });
});

module.exports = router;
