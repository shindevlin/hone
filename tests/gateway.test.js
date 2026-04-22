"use strict";

/**
 * Gateway tests — v2.10.2
 *
 * Unit tests for the resolver (pure data layer) and integration tests
 * that exercise the HTTP gateway via Node's built-in http client.
 * No supertest dependency; same pattern as commerceRoutes.test.js.
 */

const http = require('http');
const express = require('express');

// Mempool stub BEFORE ledger requires it
const mockMempoolSubmit = jest.fn();
jest.mock('../src/p2p/mempool', () => ({
  submit: (...args) => mockMempoolSubmit(...args),
  addTransaction: jest.fn().mockReturnValue(true),
}));

const stateStore = require('../src/chain/stateStore');
const ledger = require('../src/services/ledger');
const resolver = require('../src/gateway/resolver');
const storefront = require('../src/gateway/storefront');
const { createGatewayRouter } = require('../src/gateway/server');
const epochBandwidth = require('../src/services/epochBandwidth');

function makeTestServer() {
  const app = express();
  app.use(express.json());
  app.use(createGatewayRouter());
  const server = http.createServer(app);
  return new Promise((resolve) => {
    server.listen(0, '127.0.0.1', () => {
      resolve({ server, port: server.address().port });
    });
  });
}

function closeServer(server) {
  if (!server) return Promise.resolve();
  return new Promise((resolve) => server.close(resolve));
}

function request(port, method, path) {
  return new Promise((resolve, reject) => {
    const req = http.request(
      { host: '127.0.0.1', port, method, path, headers: { Accept: 'text/html' } },
      (res) => {
        let raw = '';
        res.on('data', (chunk) => (raw += chunk));
        res.on('end', () => resolve({ status: res.statusCode, body: raw, headers: res.headers }));
      }
    );
    req.on('error', reject);
    req.end();
  });
}

async function seedStore(seller, products) {
  stateStore.applyEntry({
    type: 'FAUCET',
    from: 'btcpc_genesis',
    to: seller,
    token: 'BTCPC',
    amount: 10000,
    epoch: 0,
    timestamp: Math.random(),
  });
  await ledger.recordStoreOpen(seller, { name: seller + ' shop' }, 10, 10, 12.25, 1);
  for (const p of products || []) {
    await ledger.recordProductCreate(seller, p, 1);
  }
}

describe('gateway (v2.10.2)', () => {
  beforeEach(() => {
    stateStore.resetAll();
    epochBandwidth.resetAll();
    ledger.flushPendingEntries();
    mockMempoolSubmit.mockReset();
    mockMempoolSubmit.mockReturnValue({ accepted: true });
    ['alice', 'bob', 'carol', 'mallory', 'shindevlin', 'protocol'].forEach(
      a => epochBandwidth.seedForTest(a)
    );
  });

  describe('resolver.parsePath', () => {
    test('recognizes landing', () => {
      expect(resolver.parsePath('/').kind).toBe('landing');
      expect(resolver.parsePath('').kind).toBe('landing');
    });

    test('recognizes store directory', () => {
      expect(resolver.parsePath('/stores').kind).toBe('store_directory');
      expect(resolver.parsePath('/stores/').kind).toBe('store_directory');
    });

    test('recognizes store profile', () => {
      const p = resolver.parsePath('/stores/alice');
      expect(p.kind).toBe('store_profile');
      expect(p.seller).toBe('alice');
    });

    test('recognizes product', () => {
      const p = resolver.parsePath('/stores/alice/widget');
      expect(p.kind).toBe('product');
      expect(p.seller).toBe('alice');
      expect(p.slug).toBe('widget');
      expect(p.product_id).toBe('alice/widget');
    });

    test('rejects invalid slug in product path', () => {
      const p = resolver.parsePath('/stores/alice/HAS-CAPS');
      expect(p.kind).toBe('unknown');
    });

    test('recognizes shortcode', () => {
      const p = resolver.parsePath('/s/fc4719');
      expect(p.kind).toBe('shortcode');
      expect(p.shortcode).toBe('fc4719');
    });

    test('rejects malformed shortcode', () => {
      expect(resolver.parsePath('/s/toolong').kind).toBe('unknown');
      expect(resolver.parsePath('/s/bad!').kind).toBe('unknown');
    });

    test('returns unknown for random paths', () => {
      expect(resolver.parsePath('/random').kind).toBe('unknown');
      expect(resolver.parsePath('/stores/alice/widget/extra').kind).toBe('unknown');
    });

    test('strips query string', () => {
      const p = resolver.parsePath('/stores/alice?page=2');
      expect(p.kind).toBe('store_profile');
      expect(p.seller).toBe('alice');
    });
  });

  describe('resolver.shortcodeFor', () => {
    test('produces deterministic 6-char hex', () => {
      const s1 = resolver.shortcodeFor('alice/widget');
      const s2 = resolver.shortcodeFor('alice/widget');
      expect(s1).toBe(s2);
      expect(s1).toMatch(/^[a-f0-9]{6}$/);
    });

    test('different products produce different shortcodes', () => {
      const a = resolver.shortcodeFor('alice/widget');
      const b = resolver.shortcodeFor('bob/gizmo');
      expect(a).not.toBe(b);
    });
  });

  describe('resolver.resolve (data layer)', () => {
    test('landing returns 200 with empty state', () => {
      const r = resolver.resolve('/');
      expect(r.status).toBe(200);
      expect(r.kind).toBe('landing');
      expect(r.data.recent_stores).toEqual([]);
    });

    test('landing shows seeded stores', async () => {
      await seedStore('alice');
      await seedStore('bob');
      const r = resolver.resolve('/');
      expect(r.data.recent_stores.length).toBe(2);
      expect(r.data.total_stores).toBe(2);
    });

    test('store profile returns products', async () => {
      await seedStore('alice', [
        { product_id: 'alice/widget', title: 'Widget', price: 5, stock: 10 },
        { product_id: 'alice/gizmo', title: 'Gizmo', price: 7, stock: 5 },
      ]);
      const r = resolver.resolve('/stores/alice');
      expect(r.status).toBe(200);
      expect(r.data.store.seller).toBe('alice');
      expect(r.data.product_count).toBe(2);
    });

    test('unknown store is 404', () => {
      const r = resolver.resolve('/stores/nobody');
      expect(r.status).toBe(404);
    });

    test('product resolves with store context', async () => {
      await seedStore('alice', [
        { product_id: 'alice/widget', title: 'Widget', price: 5, stock: 10 },
      ]);
      const r = resolver.resolve('/stores/alice/widget');
      expect(r.status).toBe(200);
      expect(r.data.product.title).toBe('Widget');
      expect(r.data.store.seller).toBe('alice');
      expect(r.data.shortcode).toMatch(/^[a-f0-9]{6}$/);
    });

    test('unknown product is 404', () => {
      const r = resolver.resolve('/stores/alice/nothing');
      expect(r.status).toBe(404);
    });

    test('shortcode resolves to redirect', async () => {
      await seedStore('alice', [
        { product_id: 'alice/widget', title: 'Widget', price: 5, stock: 10 },
      ]);
      const shortcode = resolver.shortcodeFor('alice/widget');
      const r = resolver.resolve('/s/' + shortcode);
      expect(r.status).toBe(302);
      expect(r.kind).toBe('redirect');
      expect(r.data.location).toBe('/stores/alice/widget');
    });
  });

  describe('storefront.esc', () => {
    test('escapes HTML special characters', () => {
      expect(storefront.esc('<script>')).toBe('&lt;script&gt;');
      expect(storefront.esc('"quoted"')).toBe('&quot;quoted&quot;');
      expect(storefront.esc("'single'")).toBe('&#39;single&#39;');
      expect(storefront.esc('A & B')).toBe('A &amp; B');
    });

    test('handles null/undefined gracefully', () => {
      expect(storefront.esc(null)).toBe('');
      expect(storefront.esc(undefined)).toBe('');
    });
  });

  describe('storefront rendering', () => {
    test('renders landing with no stores', () => {
      const html = storefront.renderLanding({ recent_stores: [], total_stores: 0 });
      expect(html).toContain('<!DOCTYPE html>');
      expect(html).toContain('BTCPC');
      expect(html).toContain('No stores open yet');
    });

    test('renders store card with escaped name', () => {
      const html = storefront.renderStoreCard({
        seller: 'alice',
        name: '<script>alert(1)</script>',
        rep_score: 50,
        capacity: 10,
        total_sales: 3,
      });
      expect(html).toContain('&lt;script&gt;');
      expect(html).not.toContain('<script>alert');
    });

    test('renders product page with price and stock', () => {
      const html = storefront.renderProduct({
        product: {
          product_id: 'alice/widget',
          seller: 'alice',
          title: 'Test Widget',
          price: 5.25,
          stock: 42,
          token: 'BTCPC',
          category: 'gadgets',
        },
        store: { seller: 'alice', name: 'Alice Shop' },
        product_reputation: null,
        shortcode: 'abc123',
      });
      expect(html).toContain('Test Widget');
      expect(html).toContain('5.25');
      expect(html).toContain('42 in stock');
      expect(html).toContain('/s/abc123');
    });
  });

  describe('HTTP server integration', () => {
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

    test('GET /health returns 200', async () => {
      const r = await request(port, 'GET', '/health');
      expect(r.status).toBe(200);
      expect(r.body).toContain('ok');
    });

    test('GET / returns landing HTML', async () => {
      const r = await request(port, 'GET', '/');
      expect(r.status).toBe(200);
      expect(r.headers['content-type']).toContain('text/html');
      expect(r.body).toContain('<!DOCTYPE html>');
      expect(r.body).toContain('BTCPC');
    });

    test('GET /stores returns directory', async () => {
      await seedStore('alice');
      const r = await request(port, 'GET', '/stores');
      expect(r.status).toBe(200);
      expect(r.body).toContain('alice');
    });

    test('GET /stores/:seller returns 404 for unknown', async () => {
      const r = await request(port, 'GET', '/stores/nobody');
      expect(r.status).toBe(404);
      expect(r.body).toContain('Not found');
    });

    test('GET /stores/:seller/:slug returns product page', async () => {
      await seedStore('alice', [
        { product_id: 'alice/widget', title: 'Widget', price: 5, stock: 10 },
      ]);
      const r = await request(port, 'GET', '/stores/alice/widget');
      expect(r.status).toBe(200);
      expect(r.body).toContain('Widget');
    });

    test('GET /s/:shortcode redirects to canonical', async () => {
      await seedStore('alice', [
        { product_id: 'alice/widget', title: 'Widget', price: 5, stock: 10 },
      ]);
      const shortcode = resolver.shortcodeFor('alice/widget');
      const r = await request(port, 'GET', '/s/' + shortcode);
      expect(r.status).toBe(302);
      expect(r.headers.location).toBe('/stores/alice/widget');
    });

    test('GET /api/resolve/stores/alice returns JSON', async () => {
      await seedStore('alice');
      const r = await request(port, 'GET', '/api/resolve/stores/alice');
      expect(r.status).toBe(200);
      const parsed = JSON.parse(r.body);
      expect(parsed.kind).toBe('store_profile');
      expect(parsed.data.store.seller).toBe('alice');
    });
  });

  describe('NODE_REGISTER multi-capability (v2.10.2)', () => {
    beforeEach(() => {
      stateStore.resetAll();
      epochBandwidth.resetAll();
      ['shindevlin', 'alice', 'bob'].forEach(a => epochBandwidth.seedForTest(a));
      stateStore.applyEntry({
        type: 'FAUCET',
        from: 'btcpc_genesis',
        to: 'shindevlin',
        token: 'BTCPC',
        amount: 100,
        epoch: 0,
        timestamp: 1,
      });
    });

    test('legacy single node_type string still works', async () => {
      await ledger.recordNodeRegister('shindevlin', 'miner', 'ws://x:1', true, 1);
      const acct = stateStore.getAccount('shindevlin');
      expect(acct.node_types).toEqual(['miner']);
    });

    test('array of node_types declares multi-capability', async () => {
      await ledger.recordNodeRegister(
        'shindevlin',
        ['miner', 'verifier', 'storage_host', 'gateway_op'],
        'ws://x:1',
        true,
        1
      );
      const acct = stateStore.getAccount('shindevlin');
      expect(acct.node_types).toEqual(
        expect.arrayContaining(['miner', 'verifier', 'storage_host', 'gateway_op'])
      );
      expect(acct.node_types.length).toBe(4);
    });

    test('duplicate capabilities are deduped', async () => {
      await ledger.recordNodeRegister(
        'shindevlin',
        ['miner', 'MINER', ' miner ', 'verifier'],
        'ws://x:1',
        true,
        1
      );
      const acct = stateStore.getAccount('shindevlin');
      expect(acct.node_types.length).toBe(2);
      expect(acct.node_types).toEqual(expect.arrayContaining(['miner', 'verifier']));
    });

    test('extra fields are captured (storage capacity, lora region)', async () => {
      await ledger.recordNodeRegister(
        'shindevlin',
        ['storage_host', 'sensor_bridge'],
        'ws://x:1',
        true,
        1,
        {
          storage_capacity_gb: 500,
          lora_region: 'us915',
        }
      );
      const acct = stateStore.getAccount('shindevlin');
      expect(acct.storage_capacity_gb).toBe(500);
      expect(acct.lora_region).toBe('US915');
    });
  });
});
