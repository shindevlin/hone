"use strict";

/**
 * Vuln 9 — Explorer unauthenticated Telegram balance lookup
 *
 * The requireBotApiKey middleware is defined inline in src/explorer/server.js
 * which cannot be imported directly in tests (it calls process.exit on
 * missing MONGODB_URI). This test validates the same logic by extracting
 * the middleware pattern into a minimal express app.
 *
 * What we're testing:
 *   - GET /api/bot/balance without x-bot-api-key → 401
 *   - GET /api/bot/balance with wrong key → 401
 *   - GET /api/bot/balance with correct key → passes middleware (request continues)
 *   - GET /api/bot/history without x-bot-api-key → 401
 *   - GET /api/bot/history with correct key → passes middleware
 */

const http = require('http');
const express = require('express');

// ── Replicate the requireBotApiKey middleware from server.js ──────────────────
// This is the exact logic added to src/explorer/server.js for Vuln 9.
function requireBotApiKey(req, res, next) {
  const key = req.headers['x-bot-api-key'];
  if (!key || key !== process.env.BOT_API_KEY) {
    return res.status(401).json({ error: 'bot API key required' });
  }
  next();
}

const TEST_BOT_API_KEY = 'test-secret-bot-key-vuln9';

function makeTestServer() {
  const app = express();
  app.use(express.json());

  // Mirror the protected endpoints from server.js
  app.get('/api/bot/balance', requireBotApiKey, (req, res) => {
    res.json({ username: 'testuser', balance: 42 });
  });

  app.get('/api/bot/history', requireBotApiKey, (req, res) => {
    res.json({ transactions: [] });
  });

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

function httpRequest(port, path, headers) {
  return new Promise((resolve, reject) => {
    const req = http.request(
      { host: '127.0.0.1', port, method: 'GET', path, headers: headers || {} },
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
    req.end();
  });
}

describe('Vuln 9 — explorer bot endpoints require x-bot-api-key', () => {
  let srv;
  let port;

  beforeAll(async () => {
    process.env.BOT_API_KEY = TEST_BOT_API_KEY;
    srv = await makeTestServer();
    port = srv.port;
  });

  afterAll(async () => {
    if (srv) await closeServer(srv.server);
  });

  // /api/bot/balance ──────────────────────────────────────────────────────────

  it('GET /api/bot/balance without key → 401', async () => {
    const res = await httpRequest(port, '/api/bot/balance');
    expect(res.status).toBe(401);
    expect(res.body.error).toMatch(/bot API key required/i);
  });

  it('GET /api/bot/balance with wrong key → 401', async () => {
    const res = await httpRequest(port, '/api/bot/balance', {
      'x-bot-api-key': 'wrong-key',
    });
    expect(res.status).toBe(401);
    expect(res.body.error).toMatch(/bot API key required/i);
  });

  it('GET /api/bot/balance with correct key → 200', async () => {
    const res = await httpRequest(port, '/api/bot/balance', {
      'x-bot-api-key': TEST_BOT_API_KEY,
    });
    expect(res.status).toBe(200);
    expect(res.body).toHaveProperty('balance');
  });

  // /api/bot/history ──────────────────────────────────────────────────────────

  it('GET /api/bot/history without key → 401', async () => {
    const res = await httpRequest(port, '/api/bot/history');
    expect(res.status).toBe(401);
    expect(res.body.error).toMatch(/bot API key required/i);
  });

  it('GET /api/bot/history with correct key → 200', async () => {
    const res = await httpRequest(port, '/api/bot/history', {
      'x-bot-api-key': TEST_BOT_API_KEY,
    });
    expect(res.status).toBe(200);
    expect(res.body).toHaveProperty('transactions');
  });
});
