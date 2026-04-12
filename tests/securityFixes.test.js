"use strict";

/**
 * Security fix regression tests
 *
 * Covers:
 *   Vuln 1  — Command injection via Service Host Runner (command allowlist + arg metachar check)
 *   Vuln 8  — Auth middleware raw JWT fallback removal
 *
 * Vuln 7 (bridge balance check) tests live in bridgeRoutes.test.js (balance-check suite).
 * Vuln 9 (explorer bot API key) tests live in explorerBotAuth.test.js.
 */

// ─────────────────────────────────────────────────────────────────────────────
// Vuln 1: Command Injection — Service Host Runner allowlist
// ─────────────────────────────────────────────────────────────────────────────

const serviceHostRunner = require('../src/services/serviceHostRunner');

function makeRunner(spawnerCalls) {
  const mockSpawner = function (cmd, args, opts) {
    if (spawnerCalls) spawnerCalls.push({ cmd, args, opts });
    var listeners = {};
    return {
      killed: false,
      pid: 9999,
      on: function (evt, cb) { listeners[evt] = cb; },
      kill: function () { this.killed = true; },
    };
  };
  const mockBlobFetcher = function () { return Buffer.from('binary'); };
  return serviceHostRunner.createRunner({
    spawner: mockSpawner,
    blobFetcher: mockBlobFetcher,
  });
}

describe('Vuln 1 — serviceHostRunner command allowlist', () => {
  it('ALLOWED_COMMANDS is exported and contains expected entries', () => {
    const allowed = serviceHostRunner.ALLOWED_COMMANDS;
    expect(Array.isArray(allowed)).toBe(true);
    expect(allowed).toContain('node');
    expect(allowed).toContain('python3');
    expect(allowed).toContain('python');
    expect(allowed).toContain('wasmer');
    expect(allowed).toContain('wasmtime');
    expect(allowed).toContain('deno');
    expect(allowed).toContain('bun');
    expect(allowed).toContain('java');
    expect(allowed).toContain('dotnet');
  });

  it('spawning with command "node" succeeds', () => {
    const calls = [];
    const runner = makeRunner(calls);
    runner.register({
      slug: 'ok-node',
      deployer: 'alice',
      runtime: { binary_cid: 'cid1', command: 'node', args: ['index.js'], type: 'http', port: 3000 },
    });
    runner.prepareBinary('ok-node');
    const record = runner.start('ok-node');
    expect(record.state).toBe('running');
    expect(calls.length).toBe(1);
    expect(calls[0].cmd).toBe('node');
  });

  it('spawning with command "/bin/sh" throws "not in allowlist"', () => {
    const runner = makeRunner();
    runner.register({
      slug: 'evil-sh',
      deployer: 'attacker',
      runtime: { binary_cid: 'cid2', command: '/bin/sh', args: ['-c', 'id'], type: 'http', port: 3001 },
    });
    runner.prepareBinary('evil-sh');
    expect(() => runner.start('evil-sh')).toThrow(/not in allowlist/i);
    expect(runner.getStatus('evil-sh').state).toBe('failed');
    expect(runner.getStatus('evil-sh').failure_reason).toMatch(/not in allowlist/i);
  });

  it('spawning with command "bash" throws "not in allowlist"', () => {
    const runner = makeRunner();
    runner.register({
      slug: 'evil-bash',
      deployer: 'attacker',
      runtime: { binary_cid: 'cid3', command: 'bash', args: [], type: 'http', port: 3002 },
    });
    runner.prepareBinary('evil-bash');
    expect(() => runner.start('evil-bash')).toThrow(/not in allowlist/i);
  });

  it('args with pipe | are rejected (shell metachar)', () => {
    const runner = makeRunner();
    runner.register({
      slug: 'evil-pipe',
      deployer: 'attacker',
      runtime: { binary_cid: 'cid4', command: 'node', args: ['app.js', '| cat /etc/passwd'], type: 'http', port: 3003 },
    });
    runner.prepareBinary('evil-pipe');
    expect(() => runner.start('evil-pipe')).toThrow(/shell metacharacters/i);
    expect(runner.getStatus('evil-pipe').state).toBe('failed');
  });

  it('args with semicolon ; are rejected (shell metachar)', () => {
    const runner = makeRunner();
    runner.register({
      slug: 'evil-semi',
      deployer: 'attacker',
      runtime: { binary_cid: 'cid5', command: 'node', args: ['app.js', '; rm -rf /'], type: 'http', port: 3004 },
    });
    runner.prepareBinary('evil-semi');
    expect(() => runner.start('evil-semi')).toThrow(/shell metacharacters/i);
  });

  it('normal args with no metacharacters are accepted', () => {
    const calls = [];
    const runner = makeRunner(calls);
    runner.register({
      slug: 'safe-args',
      deployer: 'alice',
      runtime: {
        binary_cid: 'cid6',
        command: 'node',
        args: ['server.js', '--port', '8080', '--env', 'production'],
        type: 'http',
        port: 3005,
      },
    });
    runner.prepareBinary('safe-args');
    const record = runner.start('safe-args');
    expect(record.state).toBe('running');
    expect(calls[0].args).toEqual(['server.js', '--port', '8080', '--env', 'production']);
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// Vuln 8: Auth Middleware Raw JWT Fallback Removal
// ─────────────────────────────────────────────────────────────────────────────

const fs = require('fs');
const os = require('os');
const path = require('path');
const jwt = require('jsonwebtoken');

const ISOLATED_DIR = fs.mkdtempSync(path.join(os.tmpdir(), 'btcpc-sec-auth-'));
process.env.BTCPC_SECRETS_PATH = path.join(ISOLATED_DIR, 'secrets.json');
process.env.JWT_SECRET = 'security-fix-test-secret';

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
const { authenticateToken } = require('../src/middlewares/auth');

function makeToken(payload, secret) {
  return jwt.sign(payload, secret || process.env.JWT_SECRET, { expiresIn: '1h' });
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
  const next = jest.fn();
  return { req, res, next };
}

describe('Vuln 8 — auth middleware: no raw JWT fallback', () => {
  beforeEach(async () => {
    secretStore.resetForTests();
    await secretStore.load();
    for (const k of Object.keys(mockMongoUsers)) delete mockMongoUsers[k];
    mockUserModel.findById.mockClear();
  });

  afterAll(() => {
    try { fs.rmSync(ISOLATED_DIR, { recursive: true, force: true }); } catch (_) {}
  });

  it('valid JWT + user in secretStore → next() called (200 path)', async () => {
    await secretStore.createUser('sectest-alice', { email: 'alice@sec.test' });
    const ssUser = secretStore.getUser('sectest-alice');
    const token = makeToken({ id: ssUser.user_id, username: 'sectest-alice' });

    const { req, res, next } = makeReqRes(token);
    await authenticateToken(req, res, next);

    expect(next).toHaveBeenCalled();
    expect(req.user.username).toBe('sectest-alice');
    expect(res.statusCode).toBe(200); // no error response set
  });

  it('valid JWT + user in Mongo but not secretStore → next() called', async () => {
    const mongoId = 'sec-mongo-id-42';
    mockMongoUsers.bob = { _id: mongoId, username: 'sectest-bob', email: 'bob@sec.test' };
    const token = makeToken({ id: mongoId, username: 'sectest-bob' });

    const { req, res, next } = makeReqRes(token);
    await authenticateToken(req, res, next);

    expect(next).toHaveBeenCalled();
    expect(req.user.username).toBe('sectest-bob');
  });

  it('valid JWT + user in neither store → 401 (no raw JWT fallback)', async () => {
    // No user in secretStore, no user in Mongo
    const token = makeToken({ id: 'ghost-id-999', username: 'ghost-user' });

    const { req, res, next } = makeReqRes(token);
    await authenticateToken(req, res, next);

    // Must be rejected — this is the core of Vuln 8
    expect(next).not.toHaveBeenCalled();
    expect(res.statusCode).toBe(401);
    expect(res.body.error).toMatch(/authentication service unavailable/i);
  });

  it('expired JWT → 403', async () => {
    const expiredToken = jwt.sign(
      { id: 'user-x', username: 'userx' },
      process.env.JWT_SECRET,
      { expiresIn: '-1s' }
    );

    const { req, res, next } = makeReqRes(expiredToken);
    await authenticateToken(req, res, next);

    expect(next).not.toHaveBeenCalled();
    expect(res.statusCode).toBe(403);
  });

  it('no token → 401', async () => {
    const { req, res, next } = makeReqRes(null);
    await authenticateToken(req, res, next);
    expect(next).not.toHaveBeenCalled();
    expect(res.statusCode).toBe(401);
  });
});
