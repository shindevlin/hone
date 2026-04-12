"use strict";

/**
 * Key rotation system tests.
 *
 * Tests:
 *  1.  recordKeyRotate updates stateStore active key
 *  2.  recordKeyRotate updates stateStore posting key
 *  3.  recordKeyRotate updates stateStore memo key
 *  4.  Rotating only active does NOT touch memo or posting keys
 *  5.  recordKeyRotate updates owner key when provided
 *  6.  API POST /rotate-keys with correct password returns new keypairs
 *  7.  API POST /rotate-keys with wrong password returns 401
 *  8.  API POST /rotate-keys with missing password returns 400
 *  9.  New keys returned by API are valid secp256k1 keypairs
 * 10.  After rotation, old active key no longer matches on-chain key
 * 11.  API POST /rotate-owner verifies signature against on-chain owner key
 * 12.  API POST /rotate-owner with invalid signature returns 401
 * 13.  API POST /rotate-owner with wrong username returns 404
 * 14.  recordKeyRotate throws when username is missing
 * 15.  KEY_ROTATE entry is idempotent in stateStore (replayed twice keeps last value)
 */

const fs      = require('fs');
const os      = require('os');
const path    = require('path');
const http    = require('http');
const express = require('express');
const crypto  = require('crypto');

// ── Isolate secretStore ──────────────────────────────────────────────────────
const ISOLATED_DIR = fs.mkdtempSync(path.join(os.tmpdir(), 'btcpc-keyrotation-'));
process.env.BTCPC_SECRETS_PATH = path.join(ISOLATED_DIR, 'secrets.json');
process.env.BTCPC_DATA_DIR     = path.join(ISOLATED_DIR, 'data');
process.env.JWT_SECRET         = 'test-jwt-kr';
process.env.BOT_API_KEY        = 'test-bot-key-kr';

// ── Mocks for heavy dependencies ─────────────────────────────────────────────
const _mongoUsers = {};
const mockUser = {
  findOne:        jest.fn(async (q) => q && q.username ? _mongoUsers[q.username] || null : null),
  create:         jest.fn(async (f) => { const d = { ...f, _id: crypto.randomUUID(), save: jest.fn(async function () { _mongoUsers[this.username] = this; }) }; _mongoUsers[f.username] = d; return d; }),
  countDocuments: jest.fn(async () => Object.keys(_mongoUsers).length),
};
jest.mock('../src/models/User', () => mockUser);
jest.mock('../src/models/Project', () => ({ findOne: jest.fn(async () => null), find: jest.fn(async () => []) }));
jest.mock('../src/services/emissionSchedule', () => ({ getBlockReward: jest.fn(() => 243.06) }));
jest.mock('../src/services/telegramVerify', () => ({ startLink: jest.fn(async () => ({})), verifySignedChallenge: jest.fn(async () => ({})) }));
jest.mock('../src/p2p/network', () => ({ broadcast: jest.fn(), NODE_ID: 'test-node-kr' }));
jest.mock('../src/p2p/protocol', () => ({ createMessage: jest.fn(() => ({})) }));
jest.mock('../src/services/pricing', () => ({ getCurrentPricing: jest.fn(async () => ({})) }));
jest.mock('../src/services/modelRegistry', () => ({ getNetworkModels: jest.fn(async () => []), getUnmetDemand: jest.fn(() => []) }));
jest.mock('../src/controllers/dreamController', () => ({ getDreams: jest.fn(async () => []) }));
jest.mock('../src/inference/p2pRouter', () => ({ getJob: jest.fn(async () => null) }));
jest.mock('../src/chain/nodeRegistry', () => ({ getNode: jest.fn(() => null), getRegisteredNodes: jest.fn(() => []) }));

// ── Real modules (isolated) ───────────────────────────────────────────────────
const secretStore = require('../src/services/secretStore');
const stateStore  = require('../src/chain/stateStore');
const ledger      = require('../src/services/ledger');
const secp256k1   = require('secp256k1');

// ── Helpers ───────────────────────────────────────────────────────────────────
function generateKeypair() {
  let privKey;
  do { privKey = crypto.randomBytes(32); } while (!secp256k1.privateKeyVerify(privKey));
  const pubKey = Buffer.from(secp256k1.publicKeyCreate(privKey, true));
  return { privateKey: privKey.toString('hex'), publicKey: pubKey.toString('hex') };
}

// Inject an account directly into stateStore for testing
function injectAccount(username, publicKeys) {
  stateStore.applyEntry({
    type: 'ACCOUNT_CREATE',
    to: username,
    epoch: 0,
    account_data: { public_keys: publicKeys },
  });
}

// ── HTTP test server helpers ──────────────────────────────────────────────────
function makeTestApp() {
  const app = express();
  app.use(express.json());
  app.use('/api/bot', require('../src/routes/botRoutes'));
  return app;
}

function startServer(app) {
  return new Promise((resolve) => {
    const server = http.createServer(app);
    server.listen(0, '127.0.0.1', () => resolve({ server, port: server.address().port }));
  });
}

function closeServer(server) {
  return new Promise((resolve) => server.close(resolve));
}

function request(port, { method = 'GET', path: urlPath, headers = {}, body } = {}) {
  return new Promise((resolve, reject) => {
    const opts = { host: '127.0.0.1', port, method, path: urlPath, headers: { ...headers } };
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

const BOT_HEADERS = { 'x-bot-key': 'test-bot-key-kr' };

// ── Tests ─────────────────────────────────────────────────────────────────────
describe('Key Rotation System', () => {
  let srv, port;

  beforeAll(async () => {
    await secretStore.load();
    const app = makeTestApp();
    const result = await startServer(app);
    srv  = result.server;
    port = result.port;
  });

  afterAll(async () => {
    if (srv) await closeServer(srv);
    try { fs.rmSync(ISOLATED_DIR, { recursive: true }); } catch (_) {}
  });

  // ── Unit tests: ledger.recordKeyRotate + stateStore KEY_ROTATE ────────────

  it('1. recordKeyRotate updates stateStore active key', async () => {
    const kp = generateKeypair();
    const oldKp = generateKeypair();
    injectAccount('test_active_user', { owner: oldKp.publicKey, active: oldKp.publicKey, posting: oldKp.publicKey, memo: oldKp.publicKey });

    await ledger.recordKeyRotate('test_active_user', { active: kp.publicKey }, 'password', 0);

    const acct = stateStore.getAccount('test_active_user');
    expect(acct.public_keys.active).toBe(kp.publicKey);
    // owner unchanged
    expect(acct.public_keys.owner).toBe(oldKp.publicKey);
  });

  it('2. recordKeyRotate updates stateStore posting key', async () => {
    const kp = generateKeypair();
    const oldKp = generateKeypair();
    injectAccount('test_posting_user', { owner: oldKp.publicKey, active: oldKp.publicKey, posting: oldKp.publicKey, memo: oldKp.publicKey });

    await ledger.recordKeyRotate('test_posting_user', { posting: kp.publicKey }, 'password', 0);

    const acct = stateStore.getAccount('test_posting_user');
    expect(acct.public_keys.posting).toBe(kp.publicKey);
  });

  it('3. recordKeyRotate updates stateStore memo key', async () => {
    const kp = generateKeypair();
    const oldKp = generateKeypair();
    injectAccount('test_memo_user', { owner: oldKp.publicKey, active: oldKp.publicKey, posting: oldKp.publicKey, memo: oldKp.publicKey });

    await ledger.recordKeyRotate('test_memo_user', { memo: kp.publicKey }, 'password', 0);

    const acct = stateStore.getAccount('test_memo_user');
    expect(acct.public_keys.memo).toBe(kp.publicKey);
  });

  it('4. rotating only active does NOT touch memo or posting keys', async () => {
    const ownerKp   = generateKeypair();
    const activeKp  = generateKeypair();
    const postingKp = generateKeypair();
    const memoKp    = generateKeypair();
    const newActiveKp = generateKeypair();

    injectAccount('test_partial_user', {
      owner:   ownerKp.publicKey,
      active:  activeKp.publicKey,
      posting: postingKp.publicKey,
      memo:    memoKp.publicKey,
    });

    await ledger.recordKeyRotate('test_partial_user', { active: newActiveKp.publicKey }, 'password', 0);

    const acct = stateStore.getAccount('test_partial_user');
    expect(acct.public_keys.active).toBe(newActiveKp.publicKey);   // changed
    expect(acct.public_keys.posting).toBe(postingKp.publicKey);    // unchanged
    expect(acct.public_keys.memo).toBe(memoKp.publicKey);          // unchanged
    expect(acct.public_keys.owner).toBe(ownerKp.publicKey);        // unchanged
  });

  it('5. recordKeyRotate updates owner key when provided', async () => {
    const oldOwnerKp = generateKeypair();
    const newOwnerKp = generateKeypair();

    injectAccount('test_owner_user', {
      owner:   oldOwnerKp.publicKey,
      active:  oldOwnerKp.publicKey,
      posting: oldOwnerKp.publicKey,
      memo:    oldOwnerKp.publicKey,
    });

    await ledger.recordKeyRotate('test_owner_user', { owner: newOwnerKp.publicKey }, 'owner_signature', 0);

    const acct = stateStore.getAccount('test_owner_user');
    expect(acct.public_keys.owner).toBe(newOwnerKp.publicKey);
  });

  it('14. recordKeyRotate throws when username is missing', async () => {
    await expect(ledger.recordKeyRotate(null, { active: 'abc' }, 'password', 0))
      .rejects.toThrow('username required');
  });

  it('15. KEY_ROTATE entry is idempotent — second replay keeps the latest value', async () => {
    const kpA = generateKeypair();
    const kpB = generateKeypair();
    injectAccount('test_idempotent_user', { owner: kpA.publicKey, active: kpA.publicKey, posting: kpA.publicKey, memo: kpA.publicKey });

    // First rotation
    stateStore.applyEntry({
      type: 'KEY_ROTATE',
      to: 'test_idempotent_user',
      epoch: 1,
      key_rotate_data: { new_public_keys: { active: kpA.publicKey }, authorized_by: 'password', timestamp: Date.now() },
    });
    // Second rotation (different key)
    stateStore.applyEntry({
      type: 'KEY_ROTATE',
      to: 'test_idempotent_user',
      epoch: 2,
      key_rotate_data: { new_public_keys: { active: kpB.publicKey }, authorized_by: 'password', timestamp: Date.now() },
    });

    const acct = stateStore.getAccount('test_idempotent_user');
    expect(acct.public_keys.active).toBe(kpB.publicKey);
  });

  // ── API tests: POST /rotate-keys ──────────────────────────────────────────

  it('6. POST /rotate-keys with correct password returns new keypairs', async () => {
    const ownerKp = generateKeypair();
    injectAccount('api_rotate_user', {
      owner:   ownerKp.publicKey,
      active:  ownerKp.publicKey,
      posting: ownerKp.publicKey,
      memo:    ownerKp.publicKey,
    });
    await secretStore.createUser('api_rotate_user', { password: 'hunter12345' });

    const res = await request(port, {
      method: 'POST', path: '/api/bot/rotate-keys',
      headers: BOT_HEADERS,
      body: { username: 'api_rotate_user', password: 'hunter12345' },
    });

    expect(res.status).toBe(200);
    expect(res.body.success).toBe(true);
    expect(res.body.new_keys).toBeDefined();
    expect(res.body.new_keys.active).toBeDefined();
    expect(res.body.new_keys.posting).toBeDefined();
    expect(res.body.new_keys.memo).toBeDefined();
    expect(res.body.new_keys.active.pub).toBeTruthy();
    expect(res.body.new_keys.active.priv).toBeTruthy();
  });

  it('7. POST /rotate-keys with wrong password returns 401', async () => {
    const ownerKp = generateKeypair();
    injectAccount('api_rotate_user2', {
      owner:   ownerKp.publicKey,
      active:  ownerKp.publicKey,
      posting: ownerKp.publicKey,
      memo:    ownerKp.publicKey,
    });
    await secretStore.createUser('api_rotate_user2', { password: 'correctpassword' });

    const res = await request(port, {
      method: 'POST', path: '/api/bot/rotate-keys',
      headers: BOT_HEADERS,
      body: { username: 'api_rotate_user2', password: 'wrongpassword' },
    });

    expect(res.status).toBe(401);
    expect(res.body.error).toBeTruthy();
  });

  it('8. POST /rotate-keys with missing password returns 400', async () => {
    const res = await request(port, {
      method: 'POST', path: '/api/bot/rotate-keys',
      headers: BOT_HEADERS,
      body: { username: 'api_rotate_user3' },
    });

    expect(res.status).toBe(400);
  });

  it('9. new keys returned by API are valid secp256k1 keypairs', async () => {
    const ownerKp = generateKeypair();
    injectAccount('api_rotate_valid_kp', {
      owner:   ownerKp.publicKey,
      active:  ownerKp.publicKey,
      posting: ownerKp.publicKey,
      memo:    ownerKp.publicKey,
    });
    await secretStore.createUser('api_rotate_valid_kp', { password: 'validkp9876' });

    const res = await request(port, {
      method: 'POST', path: '/api/bot/rotate-keys',
      headers: BOT_HEADERS,
      body: { username: 'api_rotate_valid_kp', password: 'validkp9876' },
    });

    expect(res.status).toBe(200);
    const nk = res.body.new_keys;
    for (const role of ['active', 'posting', 'memo']) {
      const privBuf = Buffer.from(nk[role].priv, 'hex');
      const pubBuf  = Buffer.from(nk[role].pub,  'hex');
      expect(secp256k1.privateKeyVerify(privBuf)).toBe(true);
      // Verify the public key is the correct one derived from the private key
      const derivedPub = Buffer.from(secp256k1.publicKeyCreate(privBuf, true));
      expect(derivedPub.toString('hex')).toBe(nk[role].pub);
    }
  });

  it('10. after rotation, old active key no longer matches on-chain active key', async () => {
    const ownerKp  = generateKeypair();
    const oldActive = generateKeypair();
    injectAccount('api_rotate_oldkey', {
      owner:   ownerKp.publicKey,
      active:  oldActive.publicKey,
      posting: ownerKp.publicKey,
      memo:    ownerKp.publicKey,
    });
    await secretStore.createUser('api_rotate_oldkey', { password: 'oldkey54321' });

    const res = await request(port, {
      method: 'POST', path: '/api/bot/rotate-keys',
      headers: BOT_HEADERS,
      body: { username: 'api_rotate_oldkey', password: 'oldkey54321' },
    });

    expect(res.status).toBe(200);
    const onChain = stateStore.getAccount('api_rotate_oldkey');
    // Old active key must NOT match anymore
    expect(onChain.public_keys.active).not.toBe(oldActive.publicKey);
    // New on-chain key must match what was returned
    expect(onChain.public_keys.active).toBe(res.body.new_keys.active.pub);
  });

  // ── API tests: POST /rotate-owner ─────────────────────────────────────────

  it('11. POST /rotate-owner verifies signature and rotates owner key', async () => {
    const ownerKp    = generateKeypair();
    const newOwnerKp = generateKeypair();
    injectAccount('api_owner_rotate1', {
      owner:   ownerKp.publicKey,
      active:  ownerKp.publicKey,
      posting: ownerKp.publicKey,
      memo:    ownerKp.publicKey,
    });

    // Build valid challenge + signature
    const timestamp     = Date.now().toString();
    const challengeInput = 'api_owner_rotate1' + newOwnerKp.publicKey + timestamp;
    const challenge     = crypto.createHash('sha256').update(challengeInput).digest('hex');
    const innerHash     = crypto.createHash('sha256').update(challenge).digest();
    const privBuf       = Buffer.from(ownerKp.privateKey, 'hex');
    const sigObj        = secp256k1.ecdsaSign(innerHash, privBuf);
    const signature     = Buffer.from(sigObj.signature).toString('hex');

    const res = await request(port, {
      method: 'POST', path: '/api/bot/rotate-owner',
      headers: BOT_HEADERS,
      body: {
        username:             'api_owner_rotate1',
        new_owner_public_key: newOwnerKp.publicKey,
        challenge,
        signature,
      },
    });

    expect(res.status).toBe(200);
    expect(res.body.success).toBe(true);

    const onChain = stateStore.getAccount('api_owner_rotate1');
    expect(onChain.public_keys.owner).toBe(newOwnerKp.publicKey);
  });

  it('12. POST /rotate-owner with invalid signature returns 401', async () => {
    const ownerKp    = generateKeypair();
    const badKp      = generateKeypair(); // wrong key used to sign
    const newOwnerKp = generateKeypair();
    injectAccount('api_owner_rotate2', {
      owner:   ownerKp.publicKey,
      active:  ownerKp.publicKey,
      posting: ownerKp.publicKey,
      memo:    ownerKp.publicKey,
    });

    const timestamp      = Date.now().toString();
    const challengeInput = 'api_owner_rotate2' + newOwnerKp.publicKey + timestamp;
    const challenge      = crypto.createHash('sha256').update(challengeInput).digest('hex');
    const innerHash      = crypto.createHash('sha256').update(challenge).digest();
    // Sign with the WRONG key
    const badPrivBuf    = Buffer.from(badKp.privateKey, 'hex');
    const sigObj        = secp256k1.ecdsaSign(innerHash, badPrivBuf);
    const signature     = Buffer.from(sigObj.signature).toString('hex');

    const res = await request(port, {
      method: 'POST', path: '/api/bot/rotate-owner',
      headers: BOT_HEADERS,
      body: {
        username:             'api_owner_rotate2',
        new_owner_public_key: newOwnerKp.publicKey,
        challenge,
        signature,
      },
    });

    expect(res.status).toBe(401);
    expect(res.body.error).toBeTruthy();
  });

  it('13. POST /rotate-owner with unknown username returns 404', async () => {
    const ownerKp    = generateKeypair();
    const newOwnerKp = generateKeypair();

    // Do NOT inject this account into stateStore
    const timestamp      = Date.now().toString();
    const challengeInput = 'ghost_user_xyz' + newOwnerKp.publicKey + timestamp;
    const challenge      = crypto.createHash('sha256').update(challengeInput).digest('hex');
    const innerHash      = crypto.createHash('sha256').update(challenge).digest();
    const privBuf        = Buffer.from(ownerKp.privateKey, 'hex');
    const sigObj         = secp256k1.ecdsaSign(innerHash, privBuf);
    const signature      = Buffer.from(sigObj.signature).toString('hex');

    const res = await request(port, {
      method: 'POST', path: '/api/bot/rotate-owner',
      headers: BOT_HEADERS,
      body: {
        username:             'ghost_user_xyz',
        new_owner_public_key: newOwnerKp.publicKey,
        challenge,
        signature,
      },
    });

    expect(res.status).toBe(404);
    expect(res.body.error).toBeTruthy();
  });
});
