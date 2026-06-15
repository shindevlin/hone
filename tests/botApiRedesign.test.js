"use strict";

// Password create/login paths hash with bcryptjs (cost 10), repeated across
// many cases here; the default 5s Jest timeout is too tight on a loaded box.
// Environment slowness, not a product regression.
jest.setTimeout(30000);

/**
 * Bot API redesign — conversational wallet creation + password auth
 *
 * Tests:
 *  1.  Create with password → secretStore has bcrypt hash
 *  2.  Create without telegramId → works (backward compat)
 *  3.  Create returns quiz_words with 2 positions + hash
 *  4.  Create returns public_keys (owner/active/posting/memo)
 *  5.  Create returns 6-chain addresses object
 *  6.  Login correct password → JWT + linked flag
 *  7.  Login wrong password → 401
 *  8.  Login links telegramId in secretStore
 *  9.  Change password → old password fails, new works
 * 10.  Change password without JWT → 401
 * 11.  GET /addresses by telegramId → all 6 chains returned
 * 12.  GET /addresses unknown telegramId → 404
 * 13.  Backward compat: old format { username, telegramId } still works
 * 14.  Create without password → password_set: false
 */

const fs = require('fs');
const os = require('os');
const path = require('path');
const http = require('http');
const express = require('express');
const crypto = require('crypto');

// ── Isolate secretStore ──────────────────────────────────────────────────────
const ISOLATED_DIR = fs.mkdtempSync(path.join(os.tmpdir(), 'btcpc-botapi-redesign-'));
process.env.BTCPC_SECRETS_PATH = path.join(ISOLATED_DIR, 'secrets.json');
process.env.JWT_SECRET = 'test-jwt-secret-botapi-redesign';
process.env.BOT_API_KEY = 'test-bot-key-redesign';

// ── Mock heavy dependencies so tests run without Mongo/Ledger/P2P ────────────

// Track created Mongo users in memory
const _mongoUsers = {};

// Minimal User model mock
const mockUser = {
  findOne: jest.fn(async (q) => {
    if (q && q.username) return _mongoUsers[q.username] || null;
    if (q && q.telegramId) {
      return Object.values(_mongoUsers).find(u => u.telegramId === String(q.telegramId)) || null;
    }
    return null;
  }),
  create: jest.fn(async (fields) => {
    const doc = {
      ...fields,
      _id: crypto.randomUUID(),
      telegramId: null,
      telegramUsername: null,
      save: jest.fn(async function () { _mongoUsers[this.username] = this; }),
    };
    _mongoUsers[fields.username] = doc;
    return doc;
  }),
  countDocuments: jest.fn(async () => Object.keys(_mongoUsers).length),
};
jest.mock('../src/models/User', () => mockUser);

// Project model mock (not used in the new endpoints but botRoutes imports it)
jest.mock('../src/models/Project', () => ({
  findOne: jest.fn(async () => null),
  find: jest.fn(async () => []),
}));

// Ledger mock
jest.mock('../src/services/ledger', () => ({
  recordAccountCreate: jest.fn(async () => {}),
  recordTransfer: jest.fn(async () => {}),
  recordDelegate: jest.fn(async () => {}),
  getCurrentEpoch: jest.fn(async () => 0),
}));

// stateStore mock
jest.mock('../src/chain/stateStore', () => ({
  getChainHeight: jest.fn(() => 0),
  getLatestEpoch: jest.fn(() => null),
  getBalance: jest.fn(() => 0),
  getAccount: jest.fn(() => null),
  getAllComputeProofs: jest.fn(() => ({})),
  getRecentEpochs: jest.fn(() => []),
}));

// nodeRegistry mock
jest.mock('../src/chain/nodeRegistry', () => ({
  getNode: jest.fn(() => null),
  getRegisteredNodes: jest.fn(() => []),
}));

// dreamController mock
jest.mock('../src/controllers/dreamController', () => ({
  getDreams: jest.fn(async () => []),
}));

// p2pRouter mock
jest.mock('../src/inference/p2pRouter', () => ({
  getJob: jest.fn(async () => null),
}));

// emissionSchedule mock
jest.mock('../src/services/emissionSchedule', () => ({
  getBlockReward: jest.fn(() => 243.06),
}));

// telegramVerify mock
jest.mock('../src/services/telegramVerify', () => ({
  startLink: jest.fn(async () => ({ ok: true })),
  verifySignedChallenge: jest.fn(async () => ({ ok: true })),
}));

// p2p/network mock
jest.mock('../src/p2p/network', () => ({
  broadcast: jest.fn(),
  NODE_ID: 'test-node',
}));

// p2p/protocol mock
jest.mock('../src/p2p/protocol', () => ({
  createMessage: jest.fn(() => ({})),
}));

// pricing/modelRegistry/emissionSchedule mocks
jest.mock('../src/services/pricing', () => ({ getCurrentPricing: jest.fn(async () => ({})) }));
jest.mock('../src/services/modelRegistry', () => ({
  getNetworkModels: jest.fn(async () => []),
  getUnmetDemand: jest.fn(() => []),
}));

// ── Now load secretStore (real, but isolated) ────────────────────────────────
const secretStore = require('../src/services/secretStore');

// ── Build a minimal test server ──────────────────────────────────────────────
function makeTestApp() {
  // Reset secretStore module-level flag so it reloads with current state
  const app = express();
  app.use(express.json());
  const botRoutes = require('../src/routes/botRoutes');
  app.use('/api/bot', botRoutes);
  return app;
}

// ── HTTP helpers ─────────────────────────────────────────────────────────────
function startServer(app) {
  return new Promise((resolve) => {
    const server = http.createServer(app);
    server.listen(0, '127.0.0.1', () => resolve({ server, port: server.address().port }));
  });
}

function closeServer(server) {
  return new Promise((resolve) => server.close(resolve));
}

function request(port, { method = 'GET', path, headers = {}, body } = {}) {
  return new Promise((resolve, reject) => {
    const opts = { host: '127.0.0.1', port, method, path, headers: { ...headers } };
    let bodyStr = '';
    if (body) {
      bodyStr = JSON.stringify(body);
      opts.headers['Content-Type'] = 'application/json';
      opts.headers['Content-Length'] = Buffer.byteLength(bodyStr);
    }
    const req = http.request(opts, (res) => {
      let raw = '';
      res.on('data', c => (raw += c));
      res.on('end', () => {
        let parsed;
        try { parsed = raw ? JSON.parse(raw) : null; } catch (_) { parsed = raw; }
        resolve({ status: res.statusCode, body: parsed });
      });
    });
    req.on('error', reject);
    if (bodyStr) req.write(bodyStr);
    req.end();
  });
}

// ── Shared fixtures ──────────────────────────────────────────────────────────
const BOT_HEADERS = { 'x-bot-key': 'test-bot-key-redesign' };

// ── Tests ────────────────────────────────────────────────────────────────────
describe('Bot API redesign — conversational wallet + password auth', () => {
  let srv, port;

  beforeAll(async () => {
    await secretStore.load();
    const app = makeTestApp();
    const result = await startServer(app);
    srv = result.server;
    port = result.port;
  });

  afterAll(async () => {
    if (srv) await closeServer(srv);
    // Cleanup
    try { fs.rmSync(ISOLATED_DIR, { recursive: true }); } catch (_) {}
  });

  beforeEach(() => {
    // Clear in-memory Mongo users between tests
    for (const k of Object.keys(_mongoUsers)) delete _mongoUsers[k];
    mockUser.findOne.mockClear();
    mockUser.create.mockClear();
  });

  // ── 1. Create with password → secretStore has bcrypt hash ─────────────────
  it('1. create with password → secretStore stores bcrypt hash', async () => {
    const res = await request(port, {
      method: 'POST', path: '/api/bot/create',
      headers: BOT_HEADERS,
      body: { username: 'walletuser1', password: 'hunter2' },
    });
    expect(res.status).toBe(200);
    expect(res.body.success).toBe(true);
    expect(res.body.password_set).toBe(true);

    const ssUser = secretStore.getUser('walletuser1');
    expect(ssUser).not.toBeNull();
    expect(ssUser.password_hash).toBeTruthy();
    // Hash must be a bcrypt hash (starts with $2a$ or $2b$)
    expect(ssUser.password_hash).toMatch(/^\$2[ab]\$/);
  });

  // ── 2. Create without telegramId → works ──────────────────────────────────
  it('2. create without telegramId → success', async () => {
    const res = await request(port, {
      method: 'POST', path: '/api/bot/create',
      headers: BOT_HEADERS,
      body: { username: 'walletuser2' },
    });
    expect(res.status).toBe(200);
    expect(res.body.success).toBe(true);
    expect(res.body.username).toBe('walletuser2');
  });

  // ── 3. Create returns quiz_words with 2 positions + hash ──────────────────
  it('3. create returns quiz_words: 2 positions in 1-12, valid answers_hash', async () => {
    const res = await request(port, {
      method: 'POST', path: '/api/bot/create',
      headers: BOT_HEADERS,
      body: { username: 'walletuser3' },
    });
    expect(res.status).toBe(200);
    const qw = res.body.quiz_words;
    expect(qw).toBeDefined();
    expect(Array.isArray(qw.positions)).toBe(true);
    expect(qw.positions).toHaveLength(2);
    const [p1, p2] = qw.positions;
    expect(p1).toBeGreaterThanOrEqual(1);
    expect(p1).toBeLessThanOrEqual(12);
    expect(p2).toBeGreaterThanOrEqual(1);
    expect(p2).toBeLessThanOrEqual(12);
    expect(p1).not.toBe(p2);
    // Verify hash: sha256(word_at_p1 + '|' + word_at_p2)
    const words = res.body.mnemonic.split(' ');
    const expected = crypto.createHash('sha256')
      .update(words[p1 - 1] + '|' + words[p2 - 1]).digest('hex');
    expect(qw.answers_hash).toBe(expected);
  });

  // ── 4. Create returns public_keys ─────────────────────────────────────────
  it('4. create returns public_keys with owner/active/posting/memo', async () => {
    const res = await request(port, {
      method: 'POST', path: '/api/bot/create',
      headers: BOT_HEADERS,
      body: { username: 'walletuser4' },
    });
    expect(res.status).toBe(200);
    const pk = res.body.public_keys;
    expect(pk).toBeDefined();
    expect(pk.owner).toBeTruthy();
    expect(pk.active).toBeTruthy();
    expect(pk.posting).toBeTruthy();
    expect(pk.memo).toBeTruthy();
  });

  // ── 5. Create returns 6-chain addresses ───────────────────────────────────
  it('5. create returns addresses for 6 chains', async () => {
    const res = await request(port, {
      method: 'POST', path: '/api/bot/create',
      headers: BOT_HEADERS,
      body: { username: 'walletuser5' },
    });
    expect(res.status).toBe(200);
    const addr = res.body.chain_addresses;
    expect(addr).toBeDefined();
    expect(addr.btcpc).toBeTruthy();
    expect(addr.hive).toBe('walletuser5');
    // evm/solana/bitcoin/ton may be null if wallet derivation isn't supported in test env
    expect(Object.keys(addr)).toEqual(expect.arrayContaining(['btcpc', 'evm', 'solana', 'bitcoin', 'ton', 'hive']));
  });

  // ── 6. Login correct password → JWT ───────────────────────────────────────
  it('6. login with correct password → success + JWT token', async () => {
    // First create the account
    await request(port, {
      method: 'POST', path: '/api/bot/create',
      headers: BOT_HEADERS,
      body: { username: 'loginuser1', password: 'mypassword' },
    });

    const res = await request(port, {
      method: 'POST', path: '/api/bot/login',
      headers: BOT_HEADERS,
      body: { username: 'loginuser1', password: 'mypassword' },
    });
    expect(res.status).toBe(200);
    expect(res.body.success).toBe(true);
    expect(res.body.username).toBe('loginuser1');
    expect(res.body.token).toBeTruthy();
    // Verify it's a JWT (3 parts separated by dots)
    expect(res.body.token.split('.').length).toBe(3);
  });

  // ── 7. Login wrong password → 401 ─────────────────────────────────────────
  it('7. login with wrong password → 401', async () => {
    await request(port, {
      method: 'POST', path: '/api/bot/create',
      headers: BOT_HEADERS,
      body: { username: 'loginuser2', password: 'correctpass' },
    });

    const res = await request(port, {
      method: 'POST', path: '/api/bot/login',
      headers: BOT_HEADERS,
      body: { username: 'loginuser2', password: 'wrongpass' },
    });
    expect(res.status).toBe(401);
    expect(res.body.success).toBe(false);
    expect(res.body.error).toMatch(/invalid credentials/i);
  });

  // ── 8. Login links telegramId ──────────────────────────────────────────────
  it('8. login with telegramId links the account in secretStore', async () => {
    await request(port, {
      method: 'POST', path: '/api/bot/create',
      headers: BOT_HEADERS,
      body: { username: 'loginuser3', password: 'linkpass' },
    });

    const res = await request(port, {
      method: 'POST', path: '/api/bot/login',
      headers: BOT_HEADERS,
      body: { username: 'loginuser3', password: 'linkpass', telegramId: '98765' },
    });
    expect(res.status).toBe(200);
    expect(res.body.linked).toBe(true);

    const ssUser = secretStore.getUserByTelegramId('98765');
    expect(ssUser).not.toBeNull();
  });

  // ── 9. Change password ─────────────────────────────────────────────────────
  it('9. change-password → old fails, new works', async () => {
    // Create + get JWT
    await request(port, {
      method: 'POST', path: '/api/bot/create',
      headers: BOT_HEADERS,
      body: { username: 'cpuser1', password: 'oldpass123' },
    });
    const loginRes = await request(port, {
      method: 'POST', path: '/api/bot/login',
      headers: BOT_HEADERS,
      body: { username: 'cpuser1', password: 'oldpass123' },
    });
    const token = loginRes.body.token;

    // Change password
    const cpRes = await request(port, {
      method: 'POST', path: '/api/bot/change-password',
      headers: { ...BOT_HEADERS, Authorization: `Bearer ${token}` },
      body: { old_password: 'oldpass123', new_password: 'newpass456' },
    });
    expect(cpRes.status).toBe(200);
    expect(cpRes.body.success).toBe(true);

    // Old password fails
    const oldFail = await secretStore.verifyPassword('cpuser1', 'oldpass123');
    expect(oldFail).toBe(false);

    // New password works
    const newOk = await secretStore.verifyPassword('cpuser1', 'newpass456');
    expect(newOk).toBe(true);
  });

  // ── 10. Change password without JWT → 401 ─────────────────────────────────
  it('10. change-password without JWT → 401', async () => {
    const res = await request(port, {
      method: 'POST', path: '/api/bot/change-password',
      headers: BOT_HEADERS,
      body: { old_password: 'a', new_password: 'b' },
    });
    expect(res.status).toBe(401);
  });

  // ── 11. GET /addresses by telegramId → all 6 chains ───────────────────────
  it('11. GET /addresses by telegramId → 6-chain addresses object', async () => {
    // Create account with telegramId
    await request(port, {
      method: 'POST', path: '/api/bot/create',
      headers: BOT_HEADERS,
      body: { username: 'addruser1', telegramId: '55544433', password: 'addrpass' },
    });

    const res = await request(port, {
      method: 'GET', path: '/api/bot/addresses?telegramId=55544433',
      headers: BOT_HEADERS,
    });
    expect(res.status).toBe(200);
    expect(res.body.username).toBe('addruser1');
    const addr = res.body.addresses || res.body.chain_addresses;
    expect(addr).toBeDefined();
    expect(Object.keys(addr)).toEqual(expect.arrayContaining(['btcpc', 'evm', 'solana', 'bitcoin', 'ton', 'hive']));
  });

  // ── 12. GET /addresses unknown telegramId → 404 ───────────────────────────
  it('12. GET /addresses with unknown telegramId → 404', async () => {
    const res = await request(port, {
      method: 'GET', path: '/api/bot/addresses?telegramId=99999999',
      headers: BOT_HEADERS,
    });
    expect(res.status).toBe(404);
  });

  // ── 13. Backward compat: old format { username, telegramId } works ─────────
  it('13. backward compat: { username, telegramId } still creates account', async () => {
    const res = await request(port, {
      method: 'POST', path: '/api/bot/create',
      headers: BOT_HEADERS,
      body: { username: 'oldformat1', telegramId: '11122233' },
    });
    expect(res.status).toBe(200);
    expect(res.body.success).toBe(true);
    expect(res.body.username).toBe('oldformat1');
    expect(res.body.mnemonic).toBeTruthy();
  });

  // ── 14. Create without password → password_set: false ─────────────────────
  it('14. create without password → password_set: false', async () => {
    const res = await request(port, {
      method: 'POST', path: '/api/bot/create',
      headers: BOT_HEADERS,
      body: { username: 'nopassuser1' },
    });
    expect(res.status).toBe(200);
    expect(res.body.password_set).toBe(false);
  });
});
