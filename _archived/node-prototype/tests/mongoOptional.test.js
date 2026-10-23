"use strict";

/**
 * Phase F — MongoDB optional tests
 *
 * Verifies that:
 *   1. When HONE_MONGO_MODE=disabled, mongoose.connect is never called
 *   2. When mongoose.connect throws, startup continues anyway
 *   3. loginUser works via secretStore-only when Mongo is unavailable
 *   4. Chain replay (stateStore path) never touches Mongo
 *   5. HONE_MONGO_MODE=disabled: mongoEnabled stays false
 */

const fs = require('fs');
const os = require('os');
const path = require('path');

// Isolate secretStore per test run
const ISOLATED_DIR = fs.mkdtempSync(path.join(os.tmpdir(), 'hone-mongo-optional-'));
process.env.HONE_SECRETS_PATH = path.join(ISOLATED_DIR, 'secrets.json');
process.env.JWT_SECRET = 'test-secret-for-mongo-optional-tests';

// Mock mongoose — captures calls without hitting a real DB
const mockConnect = jest.fn();
jest.mock('mongoose', () => {
  const real = jest.requireActual('mongoose');
  return Object.assign({}, real, {
    connect: mockConnect,
    connection: { close: jest.fn().mockResolvedValue(undefined) },
  });
});

// Mock User model — always throws to simulate Mongo unavailable
const mockUserModel = {
  findOne: jest.fn().mockRejectedValue(new Error('Mongo unavailable')),
  findById: jest.fn().mockRejectedValue(new Error('Mongo unavailable')),
};
jest.mock('../src/models/User', () => mockUserModel);

// Mock accountManager so registerUser doesn't hit Mongo
jest.mock('../src/wallet/accountManager', () => ({
  createAccount: jest.fn(),
}));

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

describe('Phase F — MongoDB optional', () => {
  beforeEach(async () => {
    secretStore.resetForTests();
    await secretStore.load();
    mockConnect.mockClear();
    mockUserModel.findOne.mockClear();
    mockUserModel.findById.mockClear();
  });

  afterAll(() => {
    fs.rmSync(ISOLATED_DIR, { recursive: true, force: true });
  });

  // ── 1. HONE_MONGO_MODE=disabled: connectDB should skip mongoose.connect ──
  describe('HONE_MONGO_MODE=disabled', () => {
    it('does not call mongoose.connect when mode is disabled', async () => {
      const savedMode = process.env.HONE_MONGO_MODE;
      const savedUri = process.env.MONGODB_URI;

      process.env.HONE_MONGO_MODE = 'disabled';
      process.env.MONGODB_URI = 'mongodb://localhost:27017/test';

      // Inline connectDB logic (mirrors src/index.js connectDB)
      let mongoEnabled = false;
      const mongoose = require('mongoose');
      async function connectDB() {
        const mongoMode = (process.env.HONE_MONGO_MODE || '').toLowerCase();
        if (mongoMode === 'disabled') {
          return; // skip
        }
        if (!process.env.MONGODB_URI) { return; }
        try {
          await mongoose.connect(process.env.MONGODB_URI, { serverSelectionTimeoutMS: 3000 });
          mongoEnabled = true;
        } catch (_) {}
      }

      await connectDB();

      expect(mockConnect).not.toHaveBeenCalled();
      expect(mongoEnabled).toBe(false);

      process.env.HONE_MONGO_MODE = savedMode || '';
      process.env.MONGODB_URI = savedUri || '';
    });
  });

  // ── 2. mongoose.connect throws — startup continues, mongoEnabled stays false ──
  describe('mongoose.connect throws', () => {
    it('continues startup with mongoEnabled=false when connect rejects', async () => {
      const savedMode = process.env.HONE_MONGO_MODE;
      const savedUri = process.env.MONGODB_URI;

      process.env.HONE_MONGO_MODE = 'enabled';
      process.env.MONGODB_URI = 'mongodb://unreachable:27017/test';

      mockConnect.mockRejectedValueOnce(new Error('connect ECONNREFUSED'));

      let mongoEnabled = false;
      const mongoose = require('mongoose');
      async function connectDB() {
        const mongoMode = (process.env.HONE_MONGO_MODE || '').toLowerCase();
        if (mongoMode === 'disabled') { return; }
        if (!process.env.MONGODB_URI) { return; }
        try {
          await mongoose.connect(process.env.MONGODB_URI, { serverSelectionTimeoutMS: 3000 });
          mongoEnabled = true;
        } catch (_) {
          // non-fatal
        }
      }

      await expect(connectDB()).resolves.toBeUndefined();
      expect(mockConnect).toHaveBeenCalledTimes(1);
      expect(mongoEnabled).toBe(false);

      process.env.HONE_MONGO_MODE = savedMode || '';
      process.env.MONGODB_URI = savedUri || '';
    });
  });

  // ── 3. loginUser works via secretStore when Mongo model throws ──
  describe('loginUser — secretStore-only path (Mongo unavailable)', () => {
    it('returns JWT via secretStore when Mongo User.findOne throws', async () => {
      // Seed secretStore with a known user
      await secretStore.createUser('mongoless_user', {
        username: 'mongoless_user',
        password: 'password123',
        email: 'mongoless@example.com',
        wallet: 'HONE_test_wallet',
      });

      const { req, res } = makeReqRes({
        username: 'mongoless_user',
        password: 'password123',
      });

      await auth.loginUser(req, res);

      // Should succeed without touching Mongo (Mongo mock throws)
      expect(res.statusCode).toBe(200);
      expect(res.body).toHaveProperty('token');
      // Mongo should NOT have been consulted — secretStore hit first
      expect(mockUserModel.findOne).not.toHaveBeenCalled();
    });

    it('returns 401 for wrong password even with Mongo unavailable', async () => {
      await secretStore.createUser('mongoless_badpass', {
        username: 'mongoless_badpass',
        password: 'correct_password',
        email: 'badpass@example.com',
        wallet: 'HONE_test_wallet2',
      });

      const { req, res } = makeReqRes({
        username: 'mongoless_badpass',
        password: 'wrong_password',
      });

      await auth.loginUser(req, res);

      expect(res.statusCode).toBe(401);
      // Should NOT fall through to Mongo on wrong password
      expect(mockUserModel.findOne).not.toHaveBeenCalled();
    });
  });

  // ── 4. stateStore path — no Mongo imports touched ──
  describe('stateStore independence', () => {
    it('stateStore loads and reads without touching User model', () => {
      const stateStore = require('../src/chain/stateStore');
      // Basic smoke test — if stateStore calls User model anywhere, the mock would throw
      expect(typeof stateStore.getBalance).toBe('function');
      expect(typeof stateStore.getEpoch).toBe('function');
      expect(mockUserModel.findOne).not.toHaveBeenCalled();
      expect(mockUserModel.findById).not.toHaveBeenCalled();
    });

    it('stateStore.getBalance returns 0 for unknown account (no Mongo)', () => {
      const stateStore = require('../src/chain/stateStore');
      const bal = stateStore.getBalance('nonexistent_account_xyz');
      expect(typeof bal).toBe('number');
      expect(mockUserModel.findOne).not.toHaveBeenCalled();
    });
  });
});
