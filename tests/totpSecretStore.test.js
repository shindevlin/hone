"use strict";

/**
 * totp.js secretStore-first — D.5-delta tests
 *
 * Verifies:
 *   1. generateSecret / enableTOTP / disableTOTP hit secretStore when the user lives there
 *   2. Mongo fallback when secretStore doesn't have the user
 *   3. TOTP secret is never included in error paths
 */

const fs = require('fs');
const os = require('os');
const path = require('path');

// Isolate secretStore per test run
const ISOLATED_DIR = fs.mkdtempSync(path.join(os.tmpdir(), 'btcpc-totp-delta-'));
process.env.BTCPC_SECRETS_PATH = path.join(ISOLATED_DIR, 'secrets.json');

// Mock User model
const mockMongoUsers = {};
const mockUserModel = {
  findOne: jest.fn(async (query) => {
    if (query && query.username) return mockMongoUsers[query.username] || null;
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

// Stub speakeasy so we don't need a live authenticator
jest.mock('speakeasy', () => ({
  generateSecret: jest.fn(() => ({
    base32: 'TESTSECRETBASE32',
    otpauth_url: 'otpauth://totp/BTCPC:test?secret=TESTSECRETBASE32',
  })),
  totp: {
    verify: jest.fn(() => true), // always valid in tests
  },
}));

// Stub QRCode
jest.mock('qrcode', () => ({
  toDataURL: jest.fn(async () => 'data:image/png;base64,fake'),
}));

const secretStore = require('../src/services/secretStore');
// Require totp after mocks are in place
const totp = require('../src/services/totp');

afterAll(() => {
  try { fs.rmSync(ISOLATED_DIR, { recursive: true, force: true }); } catch (_) {}
});

describe('totp — secretStore-first (D.5-delta)', () => {
  beforeEach(async () => {
    secretStore.resetForTests();
    await secretStore.load();
    for (const k of Object.keys(mockMongoUsers)) delete mockMongoUsers[k];
    mockUserModel.findOne.mockClear();
    mockUserModel.findById.mockClear();
    // Reset the lazy-load flag inside totp module
    const speakeasy = require('speakeasy');
    speakeasy.totp.verify.mockReturnValue(true);
  });

  it('generateSecret hits secretStore when user exists there', async () => {
    await secretStore.createUser('alice', { email: 'alice@example.com' });
    const result = await totp.generateSecret('alice');
    expect(result.secret).toBe('TESTSECRETBASE32');
    expect(result.qrDataUrl).toMatch(/^data:image/);
    // Mongo was not consulted
    expect(mockUserModel.findOne).not.toHaveBeenCalled();
    // Secret was written to secretStore
    const rec = secretStore.getUser('alice');
    expect(rec.totp_secret).toBe('TESTSECRETBASE32');
  });

  it('generateSecret falls back to Mongo when user not in secretStore', async () => {
    const mongoUser = {
      _id: 'mongo-1',
      username: 'bob',
      totpSecret: null,
      totpEnabled: false,
      totpBackupCodes: [],
      save: jest.fn(async function () { return this; }),
    };
    mockMongoUsers.bob = mongoUser;

    const result = await totp.generateSecret('bob');
    expect(result.secret).toBe('TESTSECRETBASE32');
    expect(mockUserModel.findOne).toHaveBeenCalled();
    // Should have written to Mongo
    expect(mongoUser.save).toHaveBeenCalled();
    expect(mongoUser.totpSecret).toBe('TESTSECRETBASE32');
  });

  it('generateSecret throws when user not found anywhere', async () => {
    await expect(totp.generateSecret('ghost')).rejects.toThrow('Account not found');
  });

  it('generateSecret throws when TOTP already enabled in secretStore', async () => {
    await secretStore.createUser('carol', {
      totp_secret: 'EXISTINGSECRET',
      totp_enabled: true,
    });
    await expect(totp.generateSecret('carol')).rejects.toThrow('TOTP already enabled');
  });

  it('enableTOTP enables TOTP in secretStore and sets backup codes', async () => {
    await secretStore.createUser('dave', { totp_secret: 'TESTSECRETBASE32', totp_enabled: false });
    const result = await totp.enableTOTP('dave', '123456');
    expect(result.enabled).toBe(true);
    expect(Array.isArray(result.backupCodes)).toBe(true);
    expect(result.backupCodes.length).toBe(8);
    // Persisted in secretStore
    const rec = secretStore.getUser('dave');
    expect(rec.totp_enabled).toBe(true);
    expect(rec.totp_backup_codes).toEqual(result.backupCodes);
    // Mongo was not consulted
    expect(mockUserModel.findOne).not.toHaveBeenCalled();
  });

  it('enableTOTP falls back to Mongo when user not in secretStore', async () => {
    const mongoUser = {
      _id: 'mongo-2',
      username: 'eve',
      totpSecret: 'TESTSECRETBASE32',
      totpEnabled: false,
      totpBackupCodes: [],
      save: jest.fn(async function () { return this; }),
    };
    mockMongoUsers.eve = mongoUser;

    const result = await totp.enableTOTP('eve', '123456');
    expect(result.enabled).toBe(true);
    expect(mockUserModel.findOne).toHaveBeenCalled();
    expect(mongoUser.totpEnabled).toBe(true);
    expect(mongoUser.save).toHaveBeenCalled();
  });

  it('verifyToken uses secretStore when user is there', async () => {
    await secretStore.createUser('frank', { totp_secret: 'TESTSECRETBASE32', totp_enabled: true });
    const valid = await totp.verifyToken('frank', '123456');
    expect(valid).toBe(true);
    expect(mockUserModel.findOne).not.toHaveBeenCalled();
  });

  it('disableTOTP disables TOTP in secretStore', async () => {
    await secretStore.createUser('grace', {
      totp_secret: 'TESTSECRETBASE32',
      totp_enabled: true,
      totp_backup_codes: ['abc', 'def'],
    });
    const result = await totp.disableTOTP('grace', '123456');
    expect(result.disabled).toBe(true);
    const rec = secretStore.getUser('grace');
    expect(rec.totp_enabled).toBe(false);
    expect(rec.totp_secret).toBeNull();
    expect(rec.totp_backup_codes).toEqual([]);
    expect(mockUserModel.findOne).not.toHaveBeenCalled();
  });

  it('disableTOTP throws when TOTP not enabled', async () => {
    await secretStore.createUser('hank', { totp_enabled: false });
    await expect(totp.disableTOTP('hank', '123456')).rejects.toThrow('TOTP not enabled');
  });

  it('generateBackupCodes replaces backup codes in secretStore', async () => {
    await secretStore.createUser('iris', {
      totp_secret: 'TESTSECRETBASE32',
      totp_enabled: true,
      totp_backup_codes: ['old1', 'old2'],
    });
    const result = await totp.generateBackupCodes('iris', '123456');
    expect(result.backupCodes.length).toBe(8);
    const rec = secretStore.getUser('iris');
    expect(rec.totp_backup_codes).toEqual(result.backupCodes);
    expect(rec.totp_backup_codes).not.toContain('old1');
    expect(mockUserModel.findOne).not.toHaveBeenCalled();
  });

  it('verifyBackupCode consumes the code and returns true', async () => {
    await secretStore.createUser('jake', {
      totp_secret: 'TESTSECRETBASE32',
      totp_enabled: true,
      totp_backup_codes: ['aabbccdd', 'eeff0011'],
    });
    const used = await totp.verifyBackupCode('jake', 'aabbccdd');
    expect(used).toBe(true);
    const rec = secretStore.getUser('jake');
    expect(rec.totp_backup_codes).not.toContain('aabbccdd');
    expect(rec.totp_backup_codes).toContain('eeff0011');
  });

  it('verifyBackupCode returns false for unknown code', async () => {
    await secretStore.createUser('kira', {
      totp_secret: 'TESTSECRETBASE32',
      totp_enabled: true,
      totp_backup_codes: ['aabbccdd'],
    });
    const used = await totp.verifyBackupCode('kira', 'notacode');
    expect(used).toBe(false);
  });
});
