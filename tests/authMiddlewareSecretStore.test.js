"use strict";

/**
 * auth.js (middleware) secretStore-first — D.5-delta tests
 *
 * Verifies:
 *   1. JWT resolved via secretStore when user lives there (no Mongo)
 *   2. JWT resolved via Mongo fallback when secretStore misses
 *   3. Raw JWT claims used when neither source is available
 *   4. req.user always has a normalised { id, username, email } shape
 */

const fs = require('fs');
const os = require('os');
const path = require('path');
const jwt = require('jsonwebtoken');

// Isolate secretStore per test run
const ISOLATED_DIR = fs.mkdtempSync(path.join(os.tmpdir(), 'hone-auth-mw-delta-'));
process.env.HONE_SECRETS_PATH = path.join(ISOLATED_DIR, 'secrets.json');
process.env.JWT_SECRET = 'test-secret-auth-mw';

// Mock User model
const mockMongoUsers = {};
const mockUserModel = {
  findById: jest.fn(async (id) => {
    for (const u of Object.values(mockMongoUsers)) {
      if (String(u._id) === String(id)) return u;
    }
    return null;
  }),
};
jest.mock('../src/models/User', () => mockUserModel);

const secretStore = require('../src/services/secretStore');

// We require auth AFTER mocks so it picks up the mock User
const { authenticateToken } = require('../src/middlewares/auth');

function makeToken(payload) {
  return jwt.sign(payload, process.env.JWT_SECRET, { expiresIn: '1h' });
}

function makeReqRes(token) {
  const req = {
    headers: { authorization: token ? `Bearer ${token}` : undefined },
    user: null,
  };
  const res = {
    statusCode: 200,
    body: null,
    status(code) { this.statusCode = code; return this; },
    json(obj) { this.body = obj; return this; },
  };
  const next = jest.fn(() => {});
  return { req, res, next };
}

describe('auth middleware — secretStore-first (D.5-delta)', () => {
  beforeEach(async () => {
    secretStore.resetForTests();
    await secretStore.load();
    for (const k of Object.keys(mockMongoUsers)) delete mockMongoUsers[k];
    mockUserModel.findById.mockClear();
  });

  afterAll(() => {
    try { fs.rmSync(ISOLATED_DIR, { recursive: true, force: true }); } catch (_) {}
  });

  it('resolves user from secretStore via username claim — no Mongo', async () => {
    const { user_id } = await secretStore.createUser('alice', { email: 'alice@example.com' });
    const ssUser = secretStore.getUser('alice');
    const token = makeToken({ id: ssUser.user_id, username: 'alice' });

    const { req, res, next } = makeReqRes(token);
    await authenticateToken(req, res, next);

    expect(next).toHaveBeenCalled();
    expect(req.user).toBeTruthy();
    expect(req.user.username).toBe('alice');
    expect(req.user.id).toBe(ssUser.user_id);
    expect(req.user.email).toBe('alice@example.com');
    // Mongo was not consulted
    expect(mockUserModel.findById).not.toHaveBeenCalled();
  });

  it('resolves user from secretStore via user_id lookup', async () => {
    await secretStore.createUser('bob', { email: 'bob@example.com' });
    const ssUser = secretStore.getUser('bob');
    // Token has id but no username (simulates older tokens)
    const token = makeToken({ id: ssUser.user_id });

    const { req, res, next } = makeReqRes(token);
    await authenticateToken(req, res, next);

    expect(next).toHaveBeenCalled();
    expect(req.user.id).toBe(ssUser.user_id);
    expect(mockUserModel.findById).not.toHaveBeenCalled();
  });

  it('falls back to Mongo when secretStore misses the user', async () => {
    const mongoId = 'mongo-id-99';
    mockMongoUsers.carol = {
      _id: mongoId,
      username: 'carol',
      email: 'carol@example.com',
    };
    const token = makeToken({ id: mongoId, username: 'carol' });

    const { req, res, next } = makeReqRes(token);
    await authenticateToken(req, res, next);

    expect(next).toHaveBeenCalled();
    expect(req.user.username).toBe('carol');
    expect(req.user.id).toBe(mongoId);
    expect(mockUserModel.findById).toHaveBeenCalledTimes(1);
  });

  it('rejects with 401 when neither secretStore nor Mongo have the user (no raw JWT fallback)', async () => {
    const token = makeToken({ id: 'ghost-id', username: 'ghost' });
    const { req, res, next } = makeReqRes(token);
    await authenticateToken(req, res, next);

    // Security fix (Vuln 8): must NOT fall back to raw JWT claims.
    // User is not in any store — request must be rejected.
    expect(next).not.toHaveBeenCalled();
    expect(res.statusCode).toBe(401);
    expect(res.body && res.body.error).toMatch(/authentication service unavailable/i);
  });

  it('rejects missing token with 401', async () => {
    const { req, res, next } = makeReqRes(null);
    await authenticateToken(req, res, next);
    expect(res.statusCode).toBe(401);
    expect(next).not.toHaveBeenCalled();
  });

  it('rejects invalid token with 403', async () => {
    const { req, res, next } = makeReqRes('not.a.valid.jwt');
    await authenticateToken(req, res, next);
    expect(res.statusCode).toBe(403);
    expect(next).not.toHaveBeenCalled();
  });

  it('normalises id field from secretStore user_id', async () => {
    await secretStore.createUser('dana', { email: 'dana@example.com' });
    const ssUser = secretStore.getUser('dana');
    const token = makeToken({ id: ssUser.user_id, username: 'dana' });

    const { req, res, next } = makeReqRes(token);
    await authenticateToken(req, res, next);

    // id must be normalised — downstream code checks req.user.id
    expect(req.user.id).toBeTruthy();
    expect(typeof req.user.id).toBe('string');
  });
});
