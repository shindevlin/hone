"use strict";

/**
 * Bridge HTTP route tests (v2.16-beta).
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

const bridgeRegistry = require('../src/services/bridgeRegistry');
const bridgeRoutes = require('../src/routes/bridgeRoutes');

const TEST_CHAIN = {
  chainId: 'base',
  name: 'Base',
  bridge_reserve_address: '0xdeadbeef',
};

function makeTestServer() {
  const app = express();
  app.use(express.json());
  app.use('/api/bridge', bridgeRoutes);
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

describe('bridge HTTP routes (v2.16-beta)', () => {
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
    bridgeRegistry.resetForTests();
    bridgeRegistry.registerChain(TEST_CHAIN.chainId, {
      name: TEST_CHAIN.name,
      bridge_reserve_address: TEST_CHAIN.bridge_reserve_address,
    });
  });

  // ─────────────────────────────────────────────────────────────────
  // POST /api/bridge/wrap
  // ─────────────────────────────────────────────────────────────────

  describe('POST /api/bridge/wrap', () => {
    it('rejects unauthenticated requests', async () => {
      const res = await request(port, 'POST', '/api/bridge/wrap', {
        chainId: 'base', amount: 100,
      });
      expect(res.status).toBe(401);
    });

    it('wraps BTCPC into wBTCPC', async () => {
      const res = await request(port, 'POST', '/api/bridge/wrap', {
        chainId: 'base', amount: 1000,
      }, 'alice');
      expect(res.status).toBe(201);
      expect(res.body.success).toBe(true);
      expect(res.body.wrap.user).toBe('alice');
      expect(res.body.wrap.chain_id).toBe('base');
      expect(res.body.wrap.gross_amount).toBe(1000);
      expect(res.body.wrap.fee).toBeGreaterThan(0);
      expect(res.body.wrap.net_amount).toBeLessThan(1000);
      expect(res.body.wrap.fee_currency).toBe('BTCPC');
    });

    it('rejects missing chainId', async () => {
      const res = await request(port, 'POST', '/api/bridge/wrap', {
        amount: 100,
      }, 'alice');
      expect(res.status).toBe(400);
      expect(res.body.error).toMatch(/chainId required/);
    });

    it('rejects missing or zero amount', async () => {
      const res = await request(port, 'POST', '/api/bridge/wrap', {
        chainId: 'base', amount: 0,
      }, 'alice');
      expect(res.status).toBe(400);
      expect(res.body.error).toMatch(/positive/);
    });

    it('returns 404 for unknown chain', async () => {
      const res = await request(port, 'POST', '/api/bridge/wrap', {
        chainId: 'unknownchain', amount: 100,
      }, 'alice');
      expect(res.status).toBe(404);
    });
  });

  // ─────────────────────────────────────────────────────────────────
  // POST /api/bridge/unwrap
  // ─────────────────────────────────────────────────────────────────

  describe('POST /api/bridge/unwrap', () => {
    beforeEach(async () => {
      // First wrap to create circulating wBTCPC
      await request(port, 'POST', '/api/bridge/wrap', {
        chainId: 'base', amount: 5000,
      }, 'alice');
    });

    it('rejects unauthenticated requests', async () => {
      const res = await request(port, 'POST', '/api/bridge/unwrap', {
        chainId: 'base', amount: 100,
      });
      expect(res.status).toBe(401);
    });

    it('unwraps wBTCPC back to BTCPC', async () => {
      const res = await request(port, 'POST', '/api/bridge/unwrap', {
        chainId: 'base', amount: 500,
      }, 'alice');
      expect(res.status).toBe(201);
      expect(res.body.success).toBe(true);
      expect(res.body.unwrap.user).toBe('alice');
      expect(res.body.unwrap.chain_id).toBe('base');
      expect(res.body.unwrap.gross_amount).toBe(500);
      expect(res.body.unwrap.fee_currency).toBe('wBTCPC');
      expect(res.body.unwrap.tier).toBe('small'); // 500 < 1000 threshold
    });

    it('applies the small unwrap fee tier for small amounts', async () => {
      const res = await request(port, 'POST', '/api/bridge/unwrap', {
        chainId: 'base', amount: 100,
      }, 'alice');
      expect(res.status).toBe(201);
      expect(res.body.unwrap.tier).toBe('small');
    });

    it('rejects unwrap exceeding circulating supply', async () => {
      const chain = bridgeRegistry.getChain('base');
      const res = await request(port, 'POST', '/api/bridge/unwrap', {
        chainId: 'base', amount: chain.circulating_wbtcpc + 9999,
      }, 'alice');
      expect(res.status).toBe(422);
    });
  });

  // ─────────────────────────────────────────────────────────────────
  // POST /api/bridge/fund
  // ─────────────────────────────────────────────────────────────────

  describe('POST /api/bridge/fund', () => {
    it('rejects unauthenticated requests', async () => {
      const res = await request(port, 'POST', '/api/bridge/fund', {
        chainId: 'base', amount: 10000, lockDays: 90,
      });
      expect(res.status).toBe(401);
    });

    it('funds the bridge as an LP', async () => {
      const res = await request(port, 'POST', '/api/bridge/fund', {
        chainId: 'base', amount: 10000, lockDays: 90,
      }, 'alice');
      expect(res.status).toBe(201);
      expect(res.body.success).toBe(true);
      expect(res.body.position.funder).toBe('alice');
      expect(res.body.position.chain_id).toBe('base');
      expect(res.body.position.amount).toBe(10000);
      expect(res.body.position.weight).toBeGreaterThan(0);
    });

    it('rejects missing chainId', async () => {
      const res = await request(port, 'POST', '/api/bridge/fund', {
        amount: 10000, lockDays: 90,
      }, 'alice');
      expect(res.status).toBe(400);
      expect(res.body.error).toMatch(/chainId required/);
    });

    it('rejects missing amount', async () => {
      const res = await request(port, 'POST', '/api/bridge/fund', {
        chainId: 'base', lockDays: 90,
      }, 'alice');
      expect(res.status).toBe(400);
    });

    it('rejects lock period out of bounds', async () => {
      const res = await request(port, 'POST', '/api/bridge/fund', {
        chainId: 'base', amount: 10000, lockDays: 5,
      }, 'alice');
      expect(res.status).toBe(400);
      expect(res.body.error).toMatch(/lockDays/);
    });
  });

  // ─────────────────────────────────────────────────────────────────
  // POST /api/bridge/unlock
  // ─────────────────────────────────────────────────────────────────

  describe('POST /api/bridge/unlock', () => {
    it('rejects unauthenticated requests', async () => {
      const res = await request(port, 'POST', '/api/bridge/unlock', {
        chainId: 'base',
      });
      expect(res.status).toBe(401);
    });

    it('returns 404 when no lock exists', async () => {
      const res = await request(port, 'POST', '/api/bridge/unlock', {
        chainId: 'base',
      }, 'alice');
      expect(res.status).toBe(404);
      expect(res.body.error).toMatch(/no lock found/);
    });

    it('returns queued false when lock has not expired', async () => {
      // Fund with a fresh lock (not expired)
      await request(port, 'POST', '/api/bridge/fund', {
        chainId: 'base', amount: 5000, lockDays: 90,
      }, 'alice');
      const res = await request(port, 'POST', '/api/bridge/unlock', {
        chainId: 'base',
      }, 'alice');
      expect(res.status).toBe(200);
      expect(res.body.unlock.queued).toBe(false);
    });
  });

  // ─────────────────────────────────────────────────────────────────
  // GET /api/bridge/chains
  // ─────────────────────────────────────────────────────────────────

  describe('GET /api/bridge/chains', () => {
    it('lists supported chains', async () => {
      bridgeRegistry.registerChain('arbitrum', {
        name: 'Arbitrum One',
        bridge_reserve_address: '0xcafe',
      });
      const res = await request(port, 'GET', '/api/bridge/chains');
      expect(res.status).toBe(200);
      expect(res.body.count).toBeGreaterThanOrEqual(1);
      const ids = res.body.chains.map((c) => c.chain_id);
      expect(ids).toContain('base');
    });

    it('returns chain state including circulating wBTCPC', async () => {
      await request(port, 'POST', '/api/bridge/wrap', {
        chainId: 'base', amount: 200,
      }, 'alice');
      const res = await request(port, 'GET', '/api/bridge/chains');
      const base = res.body.chains.find((c) => c.chain_id === 'base');
      expect(base.circulating_wbtcpc).toBeGreaterThan(0);
    });
  });

  // ─────────────────────────────────────────────────────────────────
  // GET /api/bridge/chains/:id
  // ─────────────────────────────────────────────────────────────────

  describe('GET /api/bridge/chains/:id', () => {
    it('returns a single chain config', async () => {
      const res = await request(port, 'GET', '/api/bridge/chains/base');
      expect(res.status).toBe(200);
      expect(res.body.chain.chain_id).toBe('base');
      expect(res.body.chain.name).toBe('Base');
    });

    it('returns 404 for unknown chain', async () => {
      const res = await request(port, 'GET', '/api/bridge/chains/nonexistent');
      expect(res.status).toBe(404);
    });
  });

  // ─────────────────────────────────────────────────────────────────
  // GET /api/bridge/fees
  // ─────────────────────────────────────────────────────────────────

  describe('GET /api/bridge/fees', () => {
    it('returns the fee schedule', async () => {
      const res = await request(port, 'GET', '/api/bridge/fees');
      expect(res.status).toBe(200);
      expect(res.body.fees.wrap.rate_bps).toBe(5);
      expect(res.body.fees.unwrap.tiers).toHaveLength(3);
      expect(res.body.fees.unwrap.tiers[0].label).toBe('small');
      expect(res.body.fees.unwrap.tiers[1].label).toBe('mid');
      expect(res.body.fees.unwrap.tiers[2].label).toBe('large');
      expect(res.body.fees.note).toMatch(/No burn/);
    });
  });

  // ─────────────────────────────────────────────────────────────────
  // GET /api/bridge/lps
  // ─────────────────────────────────────────────────────────────────

  describe('GET /api/bridge/lps', () => {
    beforeEach(async () => {
      await request(port, 'POST', '/api/bridge/fund', {
        chainId: 'base', amount: 10000, lockDays: 365,
      }, 'alice');
      await request(port, 'POST', '/api/bridge/fund', {
        chainId: 'base', amount: 5000, lockDays: 90,
      }, 'bob');
    });

    it('requires chainId query param', async () => {
      const res = await request(port, 'GET', '/api/bridge/lps');
      expect(res.status).toBe(400);
      expect(res.body.error).toMatch(/chainId/);
    });

    it('lists LPs sorted by weight descending', async () => {
      const res = await request(port, 'GET', '/api/bridge/lps?chainId=base');
      expect(res.status).toBe(200);
      expect(res.body.lps.length).toBe(2);
      // alice has more weight (larger amount × longer lock)
      expect(res.body.lps[0].funder).toBe('alice');
    });

    it('returns 404 for unknown chain', async () => {
      const res = await request(port, 'GET', '/api/bridge/lps?chainId=unknownchain');
      expect(res.status).toBe(404);
    });
  });

  // ─────────────────────────────────────────────────────────────────
  // GET /api/bridge/lps/:funder
  // ─────────────────────────────────────────────────────────────────

  describe('GET /api/bridge/lps/:funder', () => {
    beforeEach(async () => {
      await request(port, 'POST', '/api/bridge/fund', {
        chainId: 'base', amount: 10000, lockDays: 90,
      }, 'alice');
    });

    it('returns LP position for a funder', async () => {
      const res = await request(port, 'GET', '/api/bridge/lps/alice?chainId=base');
      expect(res.status).toBe(200);
      expect(res.body.funder).toBe('alice');
      expect(res.body.positions.length).toBe(1);
      expect(res.body.positions[0].amount).toBe(10000);
    });

    it('returns 404 for funder with no position', async () => {
      const res = await request(port, 'GET', '/api/bridge/lps/nobody?chainId=base');
      expect(res.status).toBe(404);
    });
  });
});
