"use strict";

/**
 * Oracle feed HTTP route tests (v2.16-beta).
 *
 * Follows the serviceRoutes.test.js pattern: ephemeral express server
 * on port 0, JWT bypassed via x-test-user header, no supertest dependency.
 */

const http = require('http');
const express = require('express');

jest.mock('../src/middlewares/auth', () => ({
  authenticateToken: (req, res, next) => {
    const username = req.headers['x-test-user'];
    if (!username) return res.status(401).json({ error: 'unauthenticated' });
    req.user = { username, id: 'test-id-' + username };
    next();
  },
  limiter: (req, res, next) => next(),
}));

jest.mock('../src/p2p/mempool', () => ({
  submit: jest.fn(),
  addTransaction: jest.fn().mockReturnValue(true),
}));

const oracleFeeds = require('../src/services/oracleFeeds');
const oracleRoutes = require('../src/routes/oracleRoutes');

const FEED_SPEC = {
  name: 'btc-usd',
  description: 'BTC/USD spot price',
  unit: 'USD',
  decimals: 2,
  update_interval_epochs: 1,
  min_verifiers: 1,
  max_deviation_bps: 200,
};

function makeTestServer() {
  const app = express();
  app.use(express.json());
  app.use('/api/oracles', oracleRoutes);
  const server = http.createServer(app);
  return new Promise((resolve) => {
    server.listen(0, '127.0.0.1', () => {
      resolve({ server, port: server.address().port });
    });
  });
}

function closeServer(server) {
  return new Promise((resolve) => server.close(resolve));
}

function request(port, method, path, body, user) {
  return new Promise((resolve, reject) => {
    const headers = { 'Content-Type': 'application/json' };
    if (user) headers['x-test-user'] = user;
    const data = body ? JSON.stringify(body) : null;
    if (data) headers['Content-Length'] = Buffer.byteLength(data);
    const req = http.request(
      { host: '127.0.0.1', port, method, path, headers },
      (res) => {
        let raw = '';
        res.on('data', (chunk) => (raw += chunk));
        res.on('end', () => {
          let parsed = null;
          try { parsed = raw ? JSON.parse(raw) : null; } catch (_) { parsed = raw; }
          resolve({ status: res.statusCode, body: parsed });
        });
      }
    );
    req.on('error', reject);
    if (data) req.write(data);
    req.end();
  });
}

describe('oracle feed HTTP routes (v2.16-beta)', () => {
  let testServer;
  let port;

  beforeAll(async () => {
    testServer = await makeTestServer();
    port = testServer.port;
  });

  afterAll(async () => {
    if (testServer) await closeServer(testServer.server);
  });

  beforeEach(() => {
    oracleFeeds.resetForTests();
  });

  // ─────────────────────────────────────────────────────────────────
  // POST /api/oracles/feeds
  // ─────────────────────────────────────────────────────────────────

  describe('POST /api/oracles/feeds', () => {
    it('rejects unauthenticated requests', async () => {
      const res = await request(port, 'POST', '/api/oracles/feeds', FEED_SPEC);
      expect(res.status).toBe(401);
    });

    it('registers a feed owned by the authenticated user', async () => {
      const res = await request(port, 'POST', '/api/oracles/feeds', FEED_SPEC, 'alice');
      expect(res.status).toBe(201);
      expect(res.body.success).toBe(true);
      expect(res.body.feed.feed_id).toBe('alice/btc-usd');
      expect(res.body.feed.owner).toBe('alice');
      expect(res.body.feed.status).toBe('active');
      expect(res.body.feed.min_verifiers).toBe(1);
    });

    it('rejects missing name', async () => {
      const res = await request(port, 'POST', '/api/oracles/feeds', {
        ...FEED_SPEC, name: undefined,
      }, 'alice');
      expect(res.status).toBe(400);
      expect(res.body.error).toMatch(/name required/);
    });

    it('rejects missing description', async () => {
      const res = await request(port, 'POST', '/api/oracles/feeds', {
        ...FEED_SPEC, description: undefined,
      }, 'alice');
      expect(res.status).toBe(400);
      expect(res.body.error).toMatch(/description required/);
    });

    it('rejects invalid spec values from registry', async () => {
      const res = await request(port, 'POST', '/api/oracles/feeds', {
        ...FEED_SPEC, decimals: 99,
      }, 'alice');
      expect(res.status).toBe(422);
      expect(res.body.error).toMatch(/decimals/);
    });

    it('registers a feed with an allowlist verifier', async () => {
      const res = await request(port, 'POST', '/api/oracles/feeds', {
        ...FEED_SPEC, name: 'restricted-feed', verifiers: ['verifier1', 'verifier2'],
      }, 'alice');
      expect(res.status).toBe(201);
      expect(res.body.feed.verifiers).toContain('verifier1');
    });
  });

  // ─────────────────────────────────────────────────────────────────
  // POST /api/oracles/feeds/:id/reports
  // ─────────────────────────────────────────────────────────────────

  describe('POST /api/oracles/feeds/:id/reports', () => {
    beforeEach(async () => {
      await request(port, 'POST', '/api/oracles/feeds', FEED_SPEC, 'alice');
    });

    it('rejects unauthenticated requests', async () => {
      const res = await request(
        port, 'POST',
        '/api/oracles/feeds/' + encodeURIComponent('alice/btc-usd') + '/reports',
        { value: 65000 }
      );
      expect(res.status).toBe(401);
    });

    it('submits a verifier report', async () => {
      const res = await request(
        port, 'POST',
        '/api/oracles/feeds/' + encodeURIComponent('alice/btc-usd') + '/reports',
        { value: 65000, epoch: 1 },
        'verifier1'
      );
      expect(res.status).toBe(201);
      expect(res.body.success).toBe(true);
      expect(res.body.report.verifier).toBe('verifier1');
      expect(res.body.report.value).toBe(65000);
      expect(res.body.report.epoch).toBe(1);
    });

    it('rejects missing value', async () => {
      const res = await request(
        port, 'POST',
        '/api/oracles/feeds/' + encodeURIComponent('alice/btc-usd') + '/reports',
        { epoch: 1 },
        'verifier1'
      );
      expect(res.status).toBe(400);
      expect(res.body.error).toMatch(/value required/);
    });

    it('rejects non-finite value', async () => {
      const res = await request(
        port, 'POST',
        '/api/oracles/feeds/' + encodeURIComponent('alice/btc-usd') + '/reports',
        { value: 'moon', epoch: 1 },
        'verifier1'
      );
      expect(res.status).toBe(400);
    });

    it('returns 404 for unknown feed', async () => {
      const res = await request(
        port, 'POST',
        '/api/oracles/feeds/' + encodeURIComponent('alice/ghost') + '/reports',
        { value: 65000 },
        'verifier1'
      );
      expect(res.status).toBe(404);
    });

    it('returns 409 on duplicate report from same verifier+epoch', async () => {
      await request(
        port, 'POST',
        '/api/oracles/feeds/' + encodeURIComponent('alice/btc-usd') + '/reports',
        { value: 65000, epoch: 5 },
        'verifier1'
      );
      const res = await request(
        port, 'POST',
        '/api/oracles/feeds/' + encodeURIComponent('alice/btc-usd') + '/reports',
        { value: 66000, epoch: 5 },
        'verifier1'
      );
      expect(res.status).toBe(409);
    });

    it('returns 403 when verifier not in allowlist', async () => {
      await request(port, 'POST', '/api/oracles/feeds', {
        ...FEED_SPEC, name: 'restricted', verifiers: ['approved-verifier'],
      }, 'alice');
      const res = await request(
        port, 'POST',
        '/api/oracles/feeds/' + encodeURIComponent('alice/restricted') + '/reports',
        { value: 65000, epoch: 1 },
        'stranger'
      );
      expect(res.status).toBe(403);
    });
  });

  // ─────────────────────────────────────────────────────────────────
  // POST /api/oracles/feeds/:id/finalize
  // ─────────────────────────────────────────────────────────────────

  describe('POST /api/oracles/feeds/:id/finalize', () => {
    beforeEach(async () => {
      await request(port, 'POST', '/api/oracles/feeds', FEED_SPEC, 'alice');
      await request(
        port, 'POST',
        '/api/oracles/feeds/' + encodeURIComponent('alice/btc-usd') + '/reports',
        { value: 65000, epoch: 10 },
        'verifier1'
      );
    });

    it('rejects unauthenticated requests', async () => {
      const res = await request(
        port, 'POST',
        '/api/oracles/feeds/' + encodeURIComponent('alice/btc-usd') + '/finalize',
        { epoch: 10 }
      );
      expect(res.status).toBe(401);
    });

    it('finalizes an epoch and returns the consensus', async () => {
      const res = await request(
        port, 'POST',
        '/api/oracles/feeds/' + encodeURIComponent('alice/btc-usd') + '/finalize',
        { epoch: 10 },
        'anyone'
      );
      expect(res.status).toBe(200);
      expect(res.body.success).toBe(true);
      expect(res.body.finalization.median).toBe(65000);
      expect(res.body.finalization.epoch).toBe(10);
    });

    it('is idempotent — calling finalize twice returns the same record', async () => {
      const r1 = await request(
        port, 'POST',
        '/api/oracles/feeds/' + encodeURIComponent('alice/btc-usd') + '/finalize',
        { epoch: 10 },
        'anyone'
      );
      const r2 = await request(
        port, 'POST',
        '/api/oracles/feeds/' + encodeURIComponent('alice/btc-usd') + '/finalize',
        { epoch: 10 },
        'anyone'
      );
      expect(r1.status).toBe(200);
      expect(r2.status).toBe(200);
      expect(r1.body.finalization.median).toBe(r2.body.finalization.median);
    });

    it('returns 422 when insufficient reports', async () => {
      // Register a feed requiring min_verifiers=3
      await request(port, 'POST', '/api/oracles/feeds', {
        ...FEED_SPEC, name: 'strict-feed', min_verifiers: 3,
      }, 'alice');
      await request(
        port, 'POST',
        '/api/oracles/feeds/' + encodeURIComponent('alice/strict-feed') + '/reports',
        { value: 65000, epoch: 20 },
        'verifier1'
      );
      const res = await request(
        port, 'POST',
        '/api/oracles/feeds/' + encodeURIComponent('alice/strict-feed') + '/finalize',
        { epoch: 20 },
        'anyone'
      );
      expect(res.status).toBe(422);
      expect(res.body.error).toMatch(/insufficient/);
    });
  });

  // ─────────────────────────────────────────────────────────────────
  // POST /api/oracles/feeds/:id/retire
  // ─────────────────────────────────────────────────────────────────

  describe('POST /api/oracles/feeds/:id/retire', () => {
    beforeEach(async () => {
      await request(port, 'POST', '/api/oracles/feeds', FEED_SPEC, 'alice');
    });

    it('retires a feed', async () => {
      const res = await request(
        port, 'POST',
        '/api/oracles/feeds/' + encodeURIComponent('alice/btc-usd') + '/retire',
        {}, 'alice'
      );
      expect(res.status).toBe(200);
      expect(res.body.success).toBe(true);
      expect(res.body.feed.status).toBe('retired');
    });

    it('rejects retire from non-owner', async () => {
      const res = await request(
        port, 'POST',
        '/api/oracles/feeds/' + encodeURIComponent('alice/btc-usd') + '/retire',
        {}, 'bob'
      );
      expect(res.status).toBe(403);
    });

    it('returns 404 for unknown feed', async () => {
      const res = await request(
        port, 'POST',
        '/api/oracles/feeds/' + encodeURIComponent('alice/ghost') + '/retire',
        {}, 'alice'
      );
      expect(res.status).toBe(404);
    });
  });

  // ─────────────────────────────────────────────────────────────────
  // GET /api/oracles/feeds
  // ─────────────────────────────────────────────────────────────────

  describe('GET /api/oracles/feeds', () => {
    beforeEach(async () => {
      await request(port, 'POST', '/api/oracles/feeds', FEED_SPEC, 'alice');
      await request(port, 'POST', '/api/oracles/feeds', {
        ...FEED_SPEC, name: 'eth-usd', description: 'ETH/USD',
      }, 'bob');
    });

    it('lists all feeds paginated', async () => {
      const res = await request(port, 'GET', '/api/oracles/feeds');
      expect(res.status).toBe(200);
      expect(res.body.total).toBe(2);
      expect(res.body.feeds.length).toBe(2);
    });

    it('filters by owner', async () => {
      const res = await request(port, 'GET', '/api/oracles/feeds?owner=alice');
      expect(res.body.total).toBe(1);
      expect(res.body.feeds[0].owner).toBe('alice');
    });

    it('filters by status', async () => {
      await request(
        port, 'POST',
        '/api/oracles/feeds/' + encodeURIComponent('alice/btc-usd') + '/retire',
        {}, 'alice'
      );
      const res = await request(port, 'GET', '/api/oracles/feeds?status=active');
      expect(res.body.total).toBe(1);
      expect(res.body.feeds[0].status).toBe('active');
    });
  });

  // ─────────────────────────────────────────────────────────────────
  // GET /api/oracles/feeds/:id
  // ─────────────────────────────────────────────────────────────────

  describe('GET /api/oracles/feeds/:id', () => {
    beforeEach(async () => {
      await request(port, 'POST', '/api/oracles/feeds', FEED_SPEC, 'alice');
    });

    it('returns feed + current value + reports', async () => {
      // Submit a report and finalize to set a current value
      await request(
        port, 'POST',
        '/api/oracles/feeds/' + encodeURIComponent('alice/btc-usd') + '/reports',
        { value: 65000, epoch: 1 },
        'verifier1'
      );
      await request(
        port, 'POST',
        '/api/oracles/feeds/' + encodeURIComponent('alice/btc-usd') + '/finalize',
        { epoch: 1 }, 'anyone'
      );

      const res = await request(
        port, 'GET',
        '/api/oracles/feeds/' + encodeURIComponent('alice/btc-usd')
      );
      expect(res.status).toBe(200);
      expect(res.body.feed.feed_id).toBe('alice/btc-usd');
      expect(res.body.current_value).toBe(65000);
    });

    it('returns 404 for unknown feed', async () => {
      const res = await request(
        port, 'GET',
        '/api/oracles/feeds/' + encodeURIComponent('alice/ghost')
      );
      expect(res.status).toBe(404);
    });

    it('includes reports for the queried epoch', async () => {
      await request(
        port, 'POST',
        '/api/oracles/feeds/' + encodeURIComponent('alice/btc-usd') + '/reports',
        { value: 65000, epoch: 42 },
        'verifier1'
      );
      const res = await request(
        port, 'GET',
        '/api/oracles/feeds/' + encodeURIComponent('alice/btc-usd') + '?epoch=42'
      );
      expect(res.status).toBe(200);
      expect(res.body.reports.length).toBe(1);
      expect(res.body.reports[0].verifier).toBe('verifier1');
      expect(res.body.query_epoch).toBe(42);
    });
  });
});
