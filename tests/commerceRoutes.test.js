"use strict";

/**
 * Commerce HTTP route tests (v2.10.1).
 *
 * Uses Node's built-in http + a lightweight fetch-style client. No
 * supertest dependency. Spins up the commerce router on an ephemeral
 * port, bypasses JWT verification by injecting req.user directly.
 */

const http = require('http');
const express = require('express');

// Mempool stub BEFORE ledger requires it
const mockMempoolSubmit = jest.fn();
jest.mock('../src/p2p/mempool', () => ({
  submit: (...args) => mockMempoolSubmit(...args),
  addTransaction: jest.fn().mockReturnValue(true),
}));

// Bypass JWT auth — inject req.user based on a test header
jest.mock('../src/middlewares/auth', () => ({
  authenticateToken: (req, res, next) => {
    const username = req.headers['x-test-user'];
    if (!username) return res.status(401).json({ error: 'unauthenticated' });
    req.user = { username, id: 'test-id-' + username };
    next();
  },
  limiter: (req, res, next) => next(),
}));

const stateStore = require('../src/chain/stateStore');
const ledger = require('../src/services/ledger');
const commerceRoutes = require('../src/routes/commerceRoutes');

// Helper: make a tiny fetch-like client against an ephemeral express server
function makeTestServer() {
  const app = express();
  app.use(express.json());
  app.use('/api/commerce', commerceRoutes);
  const server = http.createServer(app);
  return new Promise((resolve) => {
    server.listen(0, '127.0.0.1', () => {
      const port = server.address().port;
      resolve({ server, port });
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

describe('commerce HTTP routes (v2.10.1)', () => {
  let testServer;
  let port;

  beforeAll(async () => {
    const s = await makeTestServer();
    testServer = s.server;
    port = s.port;
  });

  afterAll(async () => {
    await closeServer(testServer);
  });

  beforeEach(() => {
    stateStore.resetAll();
    ledger.flushPendingEntries();
    mockMempoolSubmit.mockReset();
    mockMempoolSubmit.mockReturnValue({ accepted: true });
    // Give Alice some BTCPC so she can stake
    stateStore.applyEntry({
      type: 'FAUCET',
      from: 'btcpc_genesis',
      to: 'alice',
      token: 'BTCPC',
      amount: 10000,
      epoch: 0,
      timestamp: 1,
    });
    // Give Bob some BTCPC so he can buy
    stateStore.applyEntry({
      type: 'FAUCET',
      from: 'btcpc_genesis',
      to: 'bob',
      token: 'BTCPC',
      amount: 1000,
      epoch: 0,
      timestamp: 2,
    });
  });

  describe('auth enforcement', () => {
    test('POST /stores requires authentication', async () => {
      const r = await request(port, 'POST', '/api/commerce/stores', { name: 'X', initial_capacity: 5 });
      expect(r.status).toBe(401);
    });

    test('GET /stores is public', async () => {
      const r = await request(port, 'GET', '/api/commerce/stores');
      expect(r.status).toBe(200);
      expect(r.body).toHaveProperty('stores');
    });
  });

  describe('store lifecycle', () => {
    test('POST /stores opens a store and locks stake', async () => {
      const r = await request(
        port,
        'POST',
        '/api/commerce/stores',
        { name: 'Alice Shop', initial_capacity: 10 },
        'alice'
      );
      expect(r.status).toBe(201);
      expect(r.body.success).toBe(true);
      expect(r.body.store.name).toBe('Alice Shop');
      expect(r.body.store.capacity).toBe(10);
      expect(r.body.stake_btcpc).toBe(10); // 1 BTCPC per slot
      expect(stateStore.getStore('alice')).toBeTruthy();
    });

    test('duplicate store open returns 409', async () => {
      await request(
        port,
        'POST',
        '/api/commerce/stores',
        { name: 'Alice Shop', initial_capacity: 5 },
        'alice'
      );
      const r = await request(
        port,
        'POST',
        '/api/commerce/stores',
        { name: 'Alice Shop 2', initial_capacity: 5 },
        'alice'
      );
      expect(r.status).toBe(409);
    });

    test('insufficient balance returns 402', async () => {
      const r = await request(
        port,
        'POST',
        '/api/commerce/stores',
        { name: 'Big Shop', initial_capacity: 10000 }, // requires 10000 BTCPC stake
        'bob' // only has 1000
      );
      expect(r.status).toBe(402);
      expect(r.body.error).toContain('insufficient');
    });

    test('PUT /stores/:seller rejects non-owner', async () => {
      await request(port, 'POST', '/api/commerce/stores', { name: 'A', initial_capacity: 5 }, 'alice');
      const r = await request(port, 'PUT', '/api/commerce/stores/alice', { name: 'HIJACK' }, 'bob');
      expect(r.status).toBe(403);
    });

    test('capacity expansion via POST /stores/:seller/capacity', async () => {
      await request(port, 'POST', '/api/commerce/stores', { name: 'A', initial_capacity: 5 }, 'alice');
      const r = await request(
        port,
        'POST',
        '/api/commerce/stores/alice/capacity',
        { additional_capacity: 10 },
        'alice'
      );
      expect(r.status).toBe(200);
      expect(r.body.store.capacity).toBe(15);
      expect(r.body.capacity_delta).toBe(10);
    });

    test('DELETE /stores/:seller closes the store', async () => {
      await request(port, 'POST', '/api/commerce/stores', { name: 'A', initial_capacity: 5 }, 'alice');
      const r = await request(port, 'DELETE', '/api/commerce/stores/alice', null, 'alice');
      expect(r.status).toBe(200);
      expect(r.body.store.status).toBe('closed');
    });
  });

  describe('products', () => {
    beforeEach(async () => {
      await request(port, 'POST', '/api/commerce/stores', { name: 'A', initial_capacity: 5 }, 'alice');
    });

    test('POST /products creates a product with namespaced ID', async () => {
      const r = await request(
        port,
        'POST',
        '/api/commerce/products',
        { slug: 'widget', title: 'Widget', price: 5, stock: 100 },
        'alice'
      );
      expect(r.status).toBe(201);
      expect(r.body.product.product_id).toBe('alice/widget');
    });

    test('POST /products rejects bad slug format', async () => {
      const badSlugs = ['UPPER', '-leading', 'has space', 'a/slash'];
      for (const slug of badSlugs) {
        const r = await request(
          port,
          'POST',
          '/api/commerce/products',
          { slug, title: 'X', price: 1, stock: 10 },
          'alice'
        );
        expect(r.status).toBe(400);
      }
    });

    test('POST /products rejects non-owner of store', async () => {
      // bob doesn't have a store
      const r = await request(
        port,
        'POST',
        '/api/commerce/products',
        { slug: 'fake', title: 'X', price: 1, stock: 10 },
        'bob'
      );
      expect(r.status).toBe(422);
    });

    test('POST /products rejects when capacity exhausted', async () => {
      // Open tiny store
      await request(port, 'DELETE', '/api/commerce/stores/alice', null, 'alice');
      await request(port, 'POST', '/api/commerce/stores', { name: 'Tiny', initial_capacity: 1 }, 'alice');
      await request(
        port,
        'POST',
        '/api/commerce/products',
        { slug: 'first', title: 'F', price: 1, stock: 10 },
        'alice'
      );
      const r = await request(
        port,
        'POST',
        '/api/commerce/products',
        { slug: 'second', title: 'S', price: 1, stock: 10 },
        'alice'
      );
      expect(r.status).toBe(422);
      expect(r.body.error).toContain('capacity');
    });

    test('GET /products/:id returns product + store', async () => {
      await request(
        port,
        'POST',
        '/api/commerce/products',
        { slug: 'widget', title: 'Widget', price: 5, stock: 100 },
        'alice'
      );
      const r = await request(port, 'GET', '/api/commerce/products/' + encodeURIComponent('alice/widget'));
      expect(r.status).toBe(200);
      expect(r.body.product.title).toBe('Widget');
      expect(r.body.store.seller).toBe('alice');
    });

    test('PUT /products/:id rejects non-seller', async () => {
      await request(
        port,
        'POST',
        '/api/commerce/products',
        { slug: 'widget', title: 'Widget', price: 5, stock: 100 },
        'alice'
      );
      const r = await request(
        port,
        'PUT',
        '/api/commerce/products/' + encodeURIComponent('alice/widget'),
        { title: 'HIJACKED' },
        'bob'
      );
      expect(r.status).toBe(403);
    });
  });

  describe('order lifecycle', () => {
    beforeEach(async () => {
      await request(port, 'POST', '/api/commerce/stores', { name: 'A', initial_capacity: 5 }, 'alice');
      await request(
        port,
        'POST',
        '/api/commerce/products',
        { slug: 'widget', title: 'Widget', price: 5, stock: 100 },
        'alice'
      );
    });

    test('full flow: place → fulfill → deliver', async () => {
      const placed = await request(
        port,
        'POST',
        '/api/commerce/orders',
        { product_id: 'alice/widget', quantity: 2 },
        'bob'
      );
      expect(placed.status).toBe(201);
      const orderId = placed.body.order.order_id;
      expect(placed.body.order.status).toBe('placed');

      const fulfilled = await request(
        port,
        'POST',
        '/api/commerce/orders/' + orderId + '/fulfill',
        { fulfillment_cid: 'QmTrack123' },
        'alice'
      );
      expect(fulfilled.status).toBe(200);
      expect(fulfilled.body.order.status).toBe('fulfilled');

      const delivered = await request(
        port,
        'POST',
        '/api/commerce/orders/' + orderId + '/deliver',
        null,
        'bob'
      );
      expect(delivered.status).toBe(200);
      expect(delivered.body.order.status).toBe('delivered');
    });

    test('seller cannot self-order', async () => {
      const r = await request(
        port,
        'POST',
        '/api/commerce/orders',
        { product_id: 'alice/widget', quantity: 1 },
        'alice'
      );
      expect(r.status).toBe(422);
    });

    test('insufficient balance returns 402', async () => {
      // carol has no BTCPC at all
      const r = await request(
        port,
        'POST',
        '/api/commerce/orders',
        { product_id: 'alice/widget', quantity: 50 },
        'carol'
      );
      expect(r.status).toBe(402);
    });

    test('cancel returns 200 and restores stock', async () => {
      const placed = await request(
        port,
        'POST',
        '/api/commerce/orders',
        { product_id: 'alice/widget', quantity: 3 },
        'bob'
      );
      const orderId = placed.body.order.order_id;
      expect(stateStore.getProduct('alice/widget').stock).toBe(97);

      const cancelled = await request(
        port,
        'POST',
        '/api/commerce/orders/' + orderId + '/cancel',
        null,
        'bob'
      );
      expect(cancelled.status).toBe(200);
      expect(cancelled.body.order.status).toBe('cancelled');
      expect(stateStore.getProduct('alice/widget').stock).toBe(100);
    });

    test('buyer dispute works, seller self-dispute rejected', async () => {
      const placed = await request(
        port,
        'POST',
        '/api/commerce/orders',
        { product_id: 'alice/widget', quantity: 1 },
        'bob'
      );
      const orderId = placed.body.order.order_id;

      const aliceDispute = await request(
        port,
        'POST',
        '/api/commerce/orders/' + orderId + '/dispute',
        { memo: 'fake' },
        'alice'
      );
      expect(aliceDispute.status).toBe(403);

      const bobDispute = await request(
        port,
        'POST',
        '/api/commerce/orders/' + orderId + '/dispute',
        { memo: 'never arrived' },
        'bob'
      );
      expect(bobDispute.status).toBe(200);
      expect(bobDispute.body.order.status).toBe('disputed');
    });
  });

  describe('reputation', () => {
    beforeEach(async () => {
      await request(port, 'POST', '/api/commerce/stores', { name: 'A', initial_capacity: 5 }, 'alice');
    });

    test('POST /reputation/vote creates a vote with stake-weighted weight', async () => {
      const r = await request(
        port,
        'POST',
        '/api/commerce/reputation/vote',
        { target_type: 'store', target_id: 'alice', vote: 1, memo: 'good' },
        'bob'
      );
      expect(r.status).toBe(200);
      expect(r.body.reputation.votes_up).toBeGreaterThan(0);
    });

    test('invalid target_type returns 400', async () => {
      const r = await request(
        port,
        'POST',
        '/api/commerce/reputation/vote',
        { target_type: 'spam', target_id: 'alice', vote: 1 },
        'bob'
      );
      expect(r.status).toBe(400);
    });

    test('GET /reputation/:type/:id returns current score', async () => {
      await request(
        port,
        'POST',
        '/api/commerce/reputation/vote',
        { target_type: 'store', target_id: 'alice', vote: 1 },
        'bob'
      );
      const r = await request(port, 'GET', '/api/commerce/reputation/store/alice');
      expect(r.status).toBe(200);
      expect(r.body.reputation).toBeTruthy();
    });
  });

  describe('bonding curve quote', () => {
    test('GET /quote/capacity returns USD cost', async () => {
      const r = await request(port, 'GET', '/api/commerce/quote/capacity?current=0&additional=10');
      expect(r.status).toBe(200);
      expect(r.body.cost_usd).toBeGreaterThan(0);
      expect(r.body.stake_btcpc).toBe(10);
    });
  });
});
