const crypto = require('crypto');
const bcrypt = require('bcryptjs');

jest.mock('../src/models/User', () => ({
  findOne: jest.fn(),
  findById: jest.fn()
}));

jest.mock('../src/wallet/accountManager', () => ({
  createAccount: jest.fn()
}));

jest.mock('jsonwebtoken', () => ({
  sign: jest.fn(() => 'signed-jwt-token')
}));

const User = require('../src/models/User');
const { createAccount } = require('../src/wallet/accountManager');
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

describe('authController', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    process.env.JWT_SECRET = 'test-secret';
  });

  test('registerUser creates an account and returns the one-time mnemonic payload', async () => {
    createAccount.mockResolvedValue({
      username: 'alice',
      mnemonic: 'test mnemonic words',
      chainWallets: { btcpc: 'BTCPCabc' },
      publicKeys: { owner: 'owner-key' }
    });

    const req = { body: { username: 'alice', password: 'pw-123456' } };
    const res = createRes();

    await registerUser(req, res);

    expect(createAccount).toHaveBeenCalledWith('alice', null, 'pw-123456');
    expect(res.status).toHaveBeenCalledWith(201);
    expect(res.json).toHaveBeenCalledWith(expect.objectContaining({
      success: true,
      username: 'alice',
      mnemonic: 'test mnemonic words'
    }));
  });

  test('loginUser authenticates with a username against a bcrypt password', async () => {
    const hashedPassword = bcrypt.hashSync('pw-123456', 10);
    User.findOne.mockResolvedValue({
      _id: 'user-1',
      username: 'alice',
      email: 'alice@btcpc.local',
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
      email: 'legacy@btcpc.local',
      password: crypto.createHash('sha256').update('old-pass').digest('hex'),
      isActive: true,
      save: jest.fn().mockResolvedValue(undefined)
    };
    User.findOne.mockResolvedValue(legacyUser);

    const req = { body: { email: 'legacy@btcpc.local', password: 'old-pass' } };
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
