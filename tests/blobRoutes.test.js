"use strict";

/**
 * BTCPC-FS HTTP route tests — v2.11.0
 */

const fs = require('fs');
const path = require('path');
const os = require('os');
const http = require('http');
const crypto = require('crypto');

const TEST_ROOT = path.join(os.tmpdir(), 'btcpc-test-blob-routes-' + process.pid);
process.env.BTCPC_BLOB_DIR = TEST_ROOT;

jest.mock('../src/p2p/mempool', () => ({
  submit: jest.fn().mockReturnValue({ accepted: true }),
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

const express = require('express');
const stateStore = require('../src/chain/stateStore');
const ledger = require('../src/services/ledger');
const blobStore = require('../src/services/blobStore');
const blobRoutes = require('../src/routes/blobRoutes');
const epochBandwidth = require('../src/services/epochBandwidth');

function makeTestServer() {
  const app = express();
  app.use('/api/blobs', blobRoutes);
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

function request(port, method, pathStr, body, opts) {
  opts = opts || {};
  return new Promise((resolve, reject) => {
    const headers = Object.assign({}, opts.headers || {});
    if (opts.user) headers['x-test-user'] = opts.user;
    if (body && !headers['Content-Type']) {
      headers['Content-Type'] = Buffer.isBuffer(body)
        ? 'application/octet-stream'
        : 'application/json';
    }
    if (body) {
      const buf = Buffer.isBuffer(body) ? body : Buffer.from(JSON.stringify(body));
      headers['Content-Length'] = buf.length;
    }
    const req = http.request(
      { host: '127.0.0.1', port, method, path: pathStr, headers },
      (res) => {
        const chunks = [];
        res.on('data', (chunk) => chunks.push(chunk));
        res.on('end', () => {
          const raw = Buffer.concat(chunks);
          let parsed = null;
          const ct = (res.headers['content-type'] || '').toLowerCase();
          if (ct.includes('application/json')) {
            try {
              parsed = JSON.parse(raw.toString('utf8'));
            } catch (_) {
              parsed = raw.toString('utf8');
            }
          } else {
            parsed = raw;
          }
          resolve({ status: res.statusCode, body: parsed, headers: res.headers });
        });
      }
    );
    req.on('error', reject);
    if (body) {
      const buf = Buffer.isBuffer(body) ? body : Buffer.from(JSON.stringify(body));
      req.write(buf);
    }
    req.end();
  });
}

describe('blob routes (v2.11.0)', () => {
  let testServer;
  let port;

  beforeAll(async () => {
    const s = await makeTestServer();
    testServer = s.server;
    port = s.port;
  });

  afterAll(async () => {
    await closeServer(testServer);
    if (fs.existsSync(TEST_ROOT)) {
      fs.rmSync(TEST_ROOT, { recursive: true, force: true });
    }
  });

  beforeEach(() => {
    stateStore.resetAll();
    epochBandwidth.resetAll();
    ledger.flushPendingEntries();
    if (fs.existsSync(TEST_ROOT)) {
      fs.rmSync(TEST_ROOT, { recursive: true, force: true });
    }
    epochBandwidth.seedForTest('alice');
    stateStore.applyEntry({
      type: 'ACCOUNT_CREATE',
      to: 'alice',
      epoch: 0,
      account_data: { username: 'alice', public_keys: {}, chain_addresses: {} },
      timestamp: 1,
    });
  });

  describe('POST /api/blobs', () => {
    test('upload returns CID and records chain commit', async () => {
      const content = Buffer.from('hello BTCPC-FS');
      const r = await request(port, 'POST', '/api/blobs', content, { user: 'alice' });
      expect(r.status).toBe(201);
      expect(r.body.cid).toBe(blobStore.computeCid(content));
      expect(r.body.size).toBe(content.length);
      expect(r.body.existed).toBe(false);
      expect(r.body.committed).toBe(true);

      const commit = stateStore.getBlobCommit(r.body.cid);
      expect(commit).toBeTruthy();
      expect(commit.uploader).toBe('alice');
      expect(commit.hosts).toContain('alice');
    });

    test('upload without auth returns 401', async () => {
      const r = await request(port, 'POST', '/api/blobs', Buffer.from('no auth'));
      expect(r.status).toBe(401);
    });

    test('re-upload returns existed=true', async () => {
      const content = Buffer.from('duplicate');
      const first = await request(port, 'POST', '/api/blobs', content, { user: 'alice' });
      const second = await request(port, 'POST', '/api/blobs', content, { user: 'alice' });
      expect(first.body.existed).toBe(false);
      expect(second.body.existed).toBe(true);
      expect(second.body.cid).toBe(first.body.cid);
    });

    test('empty body returns 400', async () => {
      const r = await request(port, 'POST', '/api/blobs', Buffer.alloc(0), { user: 'alice' });
      expect(r.status).toBe(400);
    });

    test('binary content preserved end-to-end', async () => {
      const content = crypto.randomBytes(2048);
      const upload = await request(port, 'POST', '/api/blobs', content, { user: 'alice' });
      const download = await request(port, 'GET', '/api/blobs/' + upload.body.cid);
      expect(Buffer.isBuffer(download.body)).toBe(true);
      expect(download.body.equals(content)).toBe(true);
    });
  });

  describe('GET /api/blobs/:cid', () => {
    test('returns blob bytes with correct headers', async () => {
      const content = Buffer.from('download test');
      const upload = await request(port, 'POST', '/api/blobs', content, { user: 'alice' });
      const download = await request(port, 'GET', '/api/blobs/' + upload.body.cid);
      expect(download.status).toBe(200);
      expect(download.body.equals(content)).toBe(true);
      expect(download.headers['content-type']).toContain('application/octet-stream');
      expect(download.headers['x-btcpc-cid']).toBe(upload.body.cid);
      expect(download.headers['cache-control']).toContain('immutable');
    });

    test('returns 400 for invalid CID format', async () => {
      const r = await request(port, 'GET', '/api/blobs/not-a-cid');
      expect(r.status).toBe(400);
    });

    test('returns 404 for unknown CID', async () => {
      const r = await request(port, 'GET', '/api/blobs/' + 'a'.repeat(64));
      expect(r.status).toBe(404);
    });
  });

  describe('HEAD /api/blobs/:cid', () => {
    test('returns 200 + headers for existing blob', async () => {
      const content = Buffer.from('head test');
      const upload = await request(port, 'POST', '/api/blobs', content, { user: 'alice' });
      const head = await request(port, 'HEAD', '/api/blobs/' + upload.body.cid);
      expect(head.status).toBe(200);
      expect(head.headers['content-length']).toBe(String(content.length));
      expect(head.headers['x-btcpc-cid']).toBe(upload.body.cid);
    });

    test('returns 404 for unknown CID', async () => {
      const head = await request(port, 'HEAD', '/api/blobs/' + 'b'.repeat(64));
      expect(head.status).toBe(404);
    });
  });

  describe('GET /api/blobs/:cid/info', () => {
    test('returns combined local + chain metadata', async () => {
      const content = Buffer.from('info test');
      const upload = await request(port, 'POST', '/api/blobs', content, { user: 'alice' });
      const info = await request(port, 'GET', '/api/blobs/' + upload.body.cid + '/info');
      expect(info.status).toBe(200);
      expect(info.body.local.stored).toBe(true);
      expect(info.body.local.size).toBe(content.length);
      expect(info.body.chain.uploader).toBe('alice');
    });

    test('returns 404 for fully-unknown CID', async () => {
      const info = await request(port, 'GET', '/api/blobs/' + 'c'.repeat(64) + '/info');
      expect(info.status).toBe(404);
    });
  });

  describe('GET /api/blobs (list)', () => {
    test('empty when nothing stored', async () => {
      const r = await request(port, 'GET', '/api/blobs');
      expect(r.status).toBe(200);
      expect(r.body.blobs).toEqual([]);
      expect(r.body.total).toBe(0);
    });

    test('lists uploaded blobs with pagination', async () => {
      for (let i = 0; i < 5; i++) {
        await request(port, 'POST', '/api/blobs', Buffer.from('blob-' + i), { user: 'alice' });
      }
      const r = await request(port, 'GET', '/api/blobs?page=1&limit=3');
      expect(r.body.blobs.length).toBe(3);
      expect(r.body.total).toBe(5);
    });
  });
});
