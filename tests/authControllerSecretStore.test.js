"use strict";

/**
 * authController secretStore-first login — D.5-gamma tests
 *
 * Verifies the three login paths:
 *   1. secretStore hit  — zero Mongo, returns JWT directly
 *   2. secretStore miss, Mongo hit — falls back, migrates into
 *      secretStore, returns JWT
 *   3. secretStore hit with wrong password — does NOT fall through
 *      to Mongo (would be a credential oracle), returns 401
 */

const fs = require('fs');
const os = require('os');
const path = require('path');

// Isolate secretStore per worker before the module loads
const ISOLATED_DIR = fs.mkdtempSync(path.join(os.tmpdir(), 'hone-auth-secretstore-'));
process.env.HONE_SECRETS_PATH = path.join(ISOLATED_DIR, 'secrets.json');
process.env.JWT_SECRET = 'test-secret-for-auth-controller-tests';

// Mock User model — drop-in for mongoose User
const mockMongoUsers = {};
const mockUserModel = {
  findOne: jest.fn(async (query) => {
    const or = query && query.$or;
    if (!or) return null;
    for (const clause of or) {
      if (clause.username && mockMongoUsers[clause.username]) {
        return mockMongoUsers[clause.username];
      }
      if (clause.email) {
        for (const u of Object.values(mockMongoUsers)) {
          if (u.email === clause.email) return u;
        }
      }
    }
    return null;
  }),
  findById: jest.fn(async (id) => {
    for (const u of Object.values(mockMongoUsers)) {
      if (String(u._id) === String(id)) return u;
    }
    return null;
  }),
};
jest.mock('../src/models/User', () => mockUserModel);

// Mock accountManager so registerUser doesn't try to hit mongo
jest.mock('../src/wallet/accountManager', () => ({
  createAccount: jest.fn(),
}));

const bcrypt = require('bcryptjs');
const secretStore = require('../src/services/secretStore');
const auth = require('../src/controllers/authController');

function makeReqRes(body) {
  const req = { body: body || {}, user: null };
  const res = {
    statusCode: 200,
    body: null,
    status(code) { this.statusCode = code; return this; },
    json(obj) { this.body = obj; return this; },
  };
  return { req, res };
}

describe('authController — secretStore-first login (D.5-gamma)', () => {
  beforeEach(async () => {
    // Fresh secretStore per test
    secretStore.resetForTests();
    await secretStore.load();

    // Clear mongo mock
    for (const k of Object.keys(mockMongoUsers)) delete mockMongoUsers[k];
    mockUserModel.findOne.mockClear();
    mockUserModel.findById.mockClear();
  });

  afterAll(() => {
    try { fs.rmSync(ISOLATED_DIR, { recursive: true, force: true }); } catch (_) {}
  });

  it('logs in directly via secretStore when the user lives there', async () => {
    await secretStore.createUser('alice', { password: 'hunter2', email: 'alice@example.com' });
    const { req, res } = makeReqRes({ username: 'alice', password: 'hunter2' });

    await auth.loginUser(req, res);

    expect(res.statusCode).toBe(200);
    expect(res.body.success).toBe(true);
    expect(res.body.token).toBeTruthy();
    expect(res.body.user.username).toBe('alice');
    // Mongo was not consulted
    expect(mockUserModel.findOne).not.toHaveBeenCalled();
  });

  it('rejects wrong password without falling through to Mongo', async () => {
    await secretStore.createUser('alice', { password: 'hunter2' });
    // Also stash a legacy mongo user with the CORRECT password — if the
    // fallback fired we would get a successful login, which would be a
    // credential-oracle bug.
    mockMongoUsers.alice = {
      _id: 'mongo-id-1',
      username: 'alice',
      email: 'alice@mongo.local',
      password: bcrypt.hashSync('hunter2', 10),
      isActive: true,
    };

    const { req, res } = makeReqRes({ username: 'alice', password: 'wrong' });
    await auth.loginUser(req, res);

    expect(res.statusCode).toBe(401);
    expect(res.body.error).toMatch(/invalid credentials/i);
    // Mongo was NOT consulted — secretStore hit took precedence
    expect(mockUserModel.findOne).not.toHaveBeenCalled();
  });

  it('falls back to Mongo when secretStore does not have the user, then migrates', async () => {
    mockMongoUsers.bob = {
      _id: 'mongo-id-2',
      username: 'bob',
      email: 'bob@example.com',
      password: bcrypt.hashSync('correct-password', 10),
      isActive: true,
    };

    // secretStore does NOT know bob yet
    expect(secretStore.hasUser('bob')).toBe(false);

    const { req, res } = makeReqRes({ username: 'bob', password: 'correct-password' });
    await auth.loginUser(req, res);

    expect(res.statusCode).toBe(200);
    expect(res.body.success).toBe(true);
    expect(res.body.token).toBeTruthy();

    // Mongo was consulted exactly once for the lookup
    expect(mockUserModel.findOne).toHaveBeenCalledTimes(1);

    // Lazy migration: bob now exists in secretStore
    expect(secretStore.hasUser('bob')).toBe(true);
    const migrated = secretStore.getUser('bob');
    expect(migrated.password_hash).toBe(mockMongoUsers.bob.password);
  });

  it('second login after Mongo fallback hits secretStore only', async () => {
    mockMongoUsers.carol = {
      _id: 'mongo-id-3',
      username: 'carol',
      email: 'carol@example.com',
      password: bcrypt.hashSync('pw123', 10),
      isActive: true,
    };

    // First login: fallback to Mongo + migrate
    const r1 = makeReqRes({ username: 'carol', password: 'pw123' });
    await auth.loginUser(r1.req, r1.res);
    expect(r1.res.statusCode).toBe(200);
    expect(mockUserModel.findOne).toHaveBeenCalledTimes(1);

    mockUserModel.findOne.mockClear();

    // Second login: should be zero-Mongo
    const r2 = makeReqRes({ username: 'carol', password: 'pw123' });
    await auth.loginUser(r2.req, r2.res);
    expect(r2.res.statusCode).toBe(200);
    expect(mockUserModel.findOne).not.toHaveBeenCalled();
  });

  it('returns 401 when neither secretStore nor Mongo know the user', async () => {
    const { req, res } = makeReqRes({ username: 'ghost', password: 'anything' });
    await auth.loginUser(req, res);
    expect(res.statusCode).toBe(401);
  });

  it('returns 400 on missing password', async () => {
    const { req, res } = makeReqRes({ username: 'alice' });
    await auth.loginUser(req, res);
    expect(res.statusCode).toBe(400);
    expect(res.body.error).toMatch(/required/);
  });

  it('returns 400 on missing identifier', async () => {
    const { req, res } = makeReqRes({ password: 'hunter2' });
    await auth.loginUser(req, res);
    expect(res.statusCode).toBe(400);
  });

  it('treats case and whitespace consistently on the identifier', async () => {
    await secretStore.createUser('dave', { password: 'pw' });

    // Uppercase username — the controller lowercases + trims
    const { req, res } = makeReqRes({ username: '  DAVE  ', password: 'pw' });
    await auth.loginUser(req, res);
    expect(res.statusCode).toBe(200);
  });
});
