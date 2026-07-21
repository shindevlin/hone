const crypto = require('crypto');
const bcrypt = require('bcryptjs');
const fs = require('fs');
const os = require('os');
const path = require('path');

// Isolate secretStore per worker BEFORE requiring authController.
// Without this, the test reads the real ~/.hone/secrets.json and any
// users from previous runs make D.5-gamma's secretStore-first lookup
// short-circuit before reaching the Mongo mock.
const ISOLATED_DIR = fs.mkdtempSync(path.join(os.tmpdir(), 'hone-authctrl-test-'));
process.env.HONE_SECRETS_PATH = path.join(ISOLATED_DIR, 'secrets.json');

jest.mock('../src/models/User', () => ({
  findOne: jest.fn(),
  findById: jest.fn()
}));

jest.mock('../src/wallet/accountManager', () => ({
  createAccount: jest.fn()
}));

jest.mock('../src/services/accountCreation', () => ({
  createAccountForUser: jest.fn(),
}));

jest.mock('jsonwebtoken', () => ({
  sign: jest.fn(() => 'signed-jwt-token')
}));

const User = require('../src/models/User');
const { createAccount } = require('../src/wallet/accountManager');
const { createAccountForUser } = require('../src/services/accountCreation');
const jwt = require('jsonwebtoken');
const {
  registerUser,
  loginUser,
  matchesLegacySha256
} = require('../src/controllers/authController');

function createRes() {
  return {
    status: jest.fn().mockReturnThis(),
    json: jest.fn().mockReturnThis()
  };
}

const secretStore = require('../src/services/secretStore');

describe('authController', () => {
  beforeEach(async () => {
    jest.clearAllMocks();
    process.env.JWT_SECRET = 'test-secret';
    // Wipe secretStore between cases so the secretStore-first path
    // doesn't accidentally short-circuit a Mongo-fallback assertion.
    secretStore.resetForTests();
    await secretStore.load();
  });

  afterAll(() => {
    try { fs.rmSync(ISOLATED_DIR, { recursive: true, force: true }); } catch (_) {}
  });

  test('registerUser creates an account and returns the one-time mnemonic payload', async () => {
    createAccountForUser.mockResolvedValue({
      success: true,
      username: 'alice',
      mnemonic: 'test mnemonic words',
      wallet_export: 'wallet export',
      wallet_rows: [],
      public_keys: { owner: 'owner-key' },
      role_private_keys: { owner: 'owner-private' },
      chain_addresses: { hone: 'HONEabc' },
      chain_wallets: {},
      wallet_balance: 0,
      delegated_balance: 1,
      faucet_claimed: true,
      faucet_note: 'delegated',
    });

    const req = { body: { username: 'alice', password: 'pw-123456' } };
    const res = createRes();

    await registerUser(req, res);

    expect(createAccountForUser).toHaveBeenCalledWith(expect.objectContaining({
      username: 'alice',
      password: 'pw-123456',
      requirePassword: true,
      claimFaucet: true,
    }));
    expect(res.status).toHaveBeenCalledWith(201);
    expect(res.json).toHaveBeenCalledWith(expect.objectContaining({
      success: true,
      username: 'alice',
      mnemonic: 'test mnemonic words',
      wallet_export: 'wallet export',
    }));
  });

  test('registerUser rejects reserved system usernames', async () => {
    const req = { body: { username: 'wallet', password: 'pw-123456' } };
    const res = createRes();

    await registerUser(req, res);

    expect(createAccount).not.toHaveBeenCalled();
    expect(res.status).toHaveBeenCalledWith(400);
    expect(res.json).toHaveBeenCalledWith({ error: 'That username is reserved for system use' });
  });

  test('loginUser authenticates with a username against a bcrypt password', async () => {
    const hashedPassword = bcrypt.hashSync('pw-123456', 10);
    User.findOne.mockResolvedValue({
      _id: 'user-1',
      username: 'alice',
      email: 'alice@hone.local',
      password: hashedPassword,
      isActive: true
    });

    const req = { body: { username: 'alice', password: 'pw-123456' } };
    const res = createRes();

    await loginUser(req, res);

    expect(User.findOne).toHaveBeenCalledWith({
      $or: [{ email: 'alice' }, { username: 'alice' }]
    });
    expect(jwt.sign).toHaveBeenCalled();
    expect(res.json).toHaveBeenCalledWith(expect.objectContaining({
      success: true,
      token: 'signed-jwt-token'
    }));
  });

  test('loginUser accepts legacy sha256 passwords and upgrades them to bcrypt', async () => {
    const legacyUser = {
      _id: 'user-2',
      username: 'legacy',
      email: 'legacy@hone.local',
      password: crypto.createHash('sha256').update('old-pass').digest('hex'),
      isActive: true,
      save: jest.fn().mockResolvedValue(undefined)
    };
    User.findOne.mockResolvedValue(legacyUser);

    const req = { body: { email: 'legacy@hone.local', password: 'old-pass' } };
    const res = createRes();

    await loginUser(req, res);

    expect(legacyUser.save).toHaveBeenCalled();
    expect(legacyUser.password.startsWith('$2')).toBe(true);
    expect(res.json).toHaveBeenCalledWith(expect.objectContaining({
      success: true,
      token: 'signed-jwt-token'
    }));
  });

  test('loginUser rejects invalid credentials', async () => {
    User.findOne.mockResolvedValue(null);

    const req = { body: { username: 'missing', password: 'pw-123' } };
    const res = createRes();

    await loginUser(req, res);

    expect(res.status).toHaveBeenCalledWith(401);
    expect(res.json).toHaveBeenCalledWith({ error: 'Invalid credentials' });
  });

  test('matchesLegacySha256 recognizes legacy hashes only', () => {
    const legacyHash = crypto.createHash('sha256').update('pw-123').digest('hex');
    const bcryptHash = bcrypt.hashSync('pw-123', 10);

    expect(matchesLegacySha256('pw-123', legacyHash)).toBe(true);
    expect(matchesLegacySha256('pw-123', bcryptHash)).toBe(false);
    expect(matchesLegacySha256('wrong', legacyHash)).toBe(false);
  });
});
