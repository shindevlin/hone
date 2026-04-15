"use strict";

jest.mock('../src/models/User', () => ({
  findOne: jest.fn(),
  create: jest.fn(),
}));

jest.mock('../src/wallet/keyManager', () => ({
  generateMnemonic: jest.fn(),
  validateMnemonic: jest.fn(),
  mnemonicToKeys: jest.fn(),
  deriveSecondFactor: jest.fn(),
  deriveChainWallets: jest.fn(),
}));

const { validateUsername, createAccount } = require('../src/wallet/accountManager');
const User = require('../src/models/User');

describe('accountManager username validation', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('accepts protocol-compliant usernames', () => {
    expect(validateUsername('abc')).toEqual({ valid: true });
    expect(validateUsername('alpha-1')).toEqual({ valid: true });
    expect(validateUsername('toolong-but-still-20')).toEqual({ valid: true });
  });

  it('rejects usernames with leading or trailing hyphens', () => {
    expect(validateUsername('-alpha')).toEqual({
      valid: false,
      reason: 'Username must be 3-20 chars, lowercase letters/numbers/hyphens only, with no leading or trailing hyphen'
    });
    expect(validateUsername('alpha-')).toEqual({
      valid: false,
      reason: 'Username must be 3-20 chars, lowercase letters/numbers/hyphens only, with no leading or trailing hyphen'
    });
  });

  it('rejects reserved system usernames', () => {
    expect(validateUsername('wallet')).toEqual({
      valid: false,
      reason: 'That username is reserved for system use'
    });
  });

  it('rejects invalid characters and uppercase input', () => {
    expect(validateUsername('alpha_beta')).toEqual({
      valid: false,
      reason: 'Username must be 3-20 chars, lowercase letters/numbers/hyphens only, with no leading or trailing hyphen'
    });
    expect(validateUsername('Alpha')).toEqual({
      valid: false,
      reason: 'Username must be 3-20 chars, lowercase letters/numbers/hyphens only, with no leading or trailing hyphen'
    });
  });

  it('createAccount rejects reserved names before hitting persistence', async () => {
    await expect(createAccount('wallet', null, 'password123')).rejects.toThrow('That username is reserved for system use');
    expect(User.findOne).not.toHaveBeenCalled();
  });
});
