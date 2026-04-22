"use strict";

/**
 * BTCPC-FS serve proof route tests — v2.11.1
 */

const http = require('http');
const express = require('express');

const mockMempoolSubmit = jest.fn();
jest.mock('../src/p2p/mempool', () => ({
  submit: (...args) => mockMempoolSubmit(...args),
  addTransaction: jest.fn().mockReturnValue(true),
}));

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
const blobBandwidth = require('../src/services/blobBandwidth');
const blobServeProofRoutes = require('../src/routes/blobServeProofRoutes');
const epochBandwidth = require('../src/services/epochBandwidth');

const CID_A = 'a'.repeat(64);
const CID_B = 'b'.repeat(64);

function makeTestServer() {
  const app = express();
  app.use('/api/blob-serve', blobServeProofRoutes);
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

function request(port, method, pathStr, body, user) {
  return new Promise((resolve, reject) => {
    const headers = { 'Content-Type': 'application/json' };
    if (user) headers['x-test-user'] = user;
    const data = body ? JSON.stringify(body) : null;
    if (data) headers['Content-Length'] = Buffer.byteLength(data);

    const req = http.request(
      { host: '127.0.0.1', port, method, path: pathStr, headers },
      (res) => {
        let raw = '';
        res.on('data', (chunk) => (raw += chunk));
        res.on('end', () => {
          let parsed = null;
          try {
            parsed = raw ? JSON.parse(raw) : null;
          } catch (_) {
            parsed = raw;
          }
          resolve({ status: res.statusCode, body: parsed });
        });
      }
    );
    req.on('error', reject);
    if (data) req.write(data);
    req.end();
  });
}

describe('blob serve proof routes (v2.11.1)', () => {
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
    epochBandwidth.resetAll();
    ledger.flushPendingEntries();
    blobBandwidth.resetForTests();
    mockMempoolSubmit.mockReset();
    mockMempoolSubmit.mockReturnValue({ accepted: true });
    ['alice', 'bob', 'carol', 'mallory', 'shindevlin'].forEach(a => epochBandwidth.seedForTest(a));
  });

  describe('auth + validation', () => {
    test('POST without auth returns 401', async () => {
      const r = await request(port, 'POST', '/api/blob-serve/' + CID_A, { bytes_served: 100 });
      expect(r.status).toBe(401);
    });

    test('POST with invalid CID format returns 400', async () => {
      const r = await request(port, 'POST', '/api/blob-serve/not-a-cid', { bytes_served: 100 }, 'alice');
      expect(r.status).toBe(400);
    });

    test('POST with no commit on chain returns 404', async () => {
      const r = await request(port, 'POST', '/api/blob-serve/' + CID_A, { bytes_served: 100 }, 'alice');
      expect(r.status).toBe(404);
    });

    test('POST from non-committed host returns 403', async () => {
      await ledger.recordBlobStoreCommit('alice', CID_A, 1000, ['alice'], 100, 0, 1);
      const r = await request(port, 'POST', '/api/blob-serve/' + CID_A, { bytes_served: 100 }, 'mallory');
      expect(r.status).toBe(403);
    });

    test('POST with negative bytes_served returns 400', async () => {
      await ledger.recordBlobStoreCommit('alice', CID_A, 1000, ['alice'], 100, 0, 1);
      const r = await request(port, 'POST', '/api/blob-serve/' + CID_A, { bytes_served: -100 }, 'alice');
      expect(r.status).toBe(400);
    });
  });

  describe('explicit submission (body bytes)', () => {
    test('records BLOB_SERVE_PROOF from explicit body', async () => {
      await ledger.recordBlobStoreCommit('alice', CID_A, 1000, ['alice'], 100, 0, 1);
      const r = await request(
        port,
        'POST',
        '/api/blob-serve/' + CID_A,
        { bytes_served: 5000, request_count: 10 },
        'alice'
      );
      expect(r.status).toBe(200);
      expect(r.body.flushed_bytes).toBe(5000);
      expect(r.body.flushed_requests).toBe(10);

      const commit = stateStore.getBlobCommit(CID_A);
      expect(commit.bytes_served_total).toBe(5000);
      expect(commit.bytes_served_by_host.alice).toBe(5000);
    });

    test('zero bytes returns no-op', async () => {
      await ledger.recordBlobStoreCommit('alice', CID_A, 1000, ['alice'], 100, 0, 1);
      const r = await request(port, 'POST', '/api/blob-serve/' + CID_A, { bytes_served: 0 }, 'alice');
      expect(r.status).toBe(200);
      expect(r.body.flushed_bytes).toBe(0);
      expect(stateStore.getBlobCommit(CID_A).bytes_served_total).toBe(0);
    });
  });

  describe('accumulator flush (no body)', () => {
    test('empty body flushes in-process accumulator', async () => {
      await ledger.recordBlobStoreCommit('alice', CID_A, 1000, ['alice'], 100, 0, 1);
      blobBandwidth.accumulate(CID_A, 1234);
      blobBandwidth.accumulate(CID_A, 5678);

      const r = await request(port, 'POST', '/api/blob-serve/' + CID_A, {}, 'alice');
      expect(r.status).toBe(200);
      expect(r.body.flushed_bytes).toBe(1234 + 5678);

      // Accumulator cleared after successful flush
      expect(blobBandwidth.getPending(CID_A).bytes).toBe(0);

      // Chain reflects the flush
      const commit = stateStore.getBlobCommit(CID_A);
      expect(commit.bytes_served_by_host.alice).toBe(1234 + 5678);
    });

    test('empty accumulator returns nothing-to-flush note', async () => {
      await ledger.recordBlobStoreCommit('alice', CID_A, 1000, ['alice'], 100, 0, 1);
      const r = await request(port, 'POST', '/api/blob-serve/' + CID_A, {}, 'alice');
      expect(r.status).toBe(200);
      expect(r.body.flushed_bytes).toBe(0);
      expect(r.body.note).toContain('nothing to flush');
    });
  });

  describe('flush-all', () => {
    test('POST /flush-all without auth returns 401', async () => {
      const r = await request(port, 'POST', '/api/blob-serve/flush-all');
      expect(r.status).toBe(401);
    });

    test('flushes all pending CIDs where host is committed', async () => {
      await ledger.recordBlobStoreCommit('alice', CID_A, 100, ['alice'], 100, 0, 1);
      await ledger.recordBlobStoreCommit('alice', CID_B, 100, ['alice'], 100, 0, 1);
      blobBandwidth.accumulate(CID_A, 500);
      blobBandwidth.accumulate(CID_B, 800);

      const r = await request(port, 'POST', '/api/blob-serve/flush-all', null, 'alice');
      expect(r.status).toBe(200);
      expect(r.body.flushed).toBe(2);

      expect(stateStore.getBlobCommit(CID_A).bytes_served_total).toBe(500);
      expect(stateStore.getBlobCommit(CID_B).bytes_served_total).toBe(800);
    });
  });

  describe('pending introspection', () => {
    test('GET /:cid/pending returns current accumulator state', async () => {
      blobBandwidth.accumulate(CID_A, 777);
      const r = await request(port, 'GET', '/api/blob-serve/' + CID_A + '/pending');
      expect(r.status).toBe(200);
      expect(r.body.pending.bytes).toBe(777);
    });

    test('GET /pending returns all pending', async () => {
      blobBandwidth.accumulate(CID_A, 100);
      blobBandwidth.accumulate(CID_B, 200);
      const r = await request(port, 'GET', '/api/blob-serve/pending');
      expect(r.status).toBe(200);
      expect(r.body.pending_cid_count).toBe(2);
      expect(r.body.pending[CID_A].bytes).toBe(100);
      expect(r.body.pending[CID_B].bytes).toBe(200);
    });
  });
});
