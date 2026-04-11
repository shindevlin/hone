const secp256k1 = require('secp256k1');
const { keccak256 } = require('js-sha3');

// Phase E: LedgerEntry model removed — chainLink uses ledger service directly
const mockRecordAccountCreate = jest.fn().mockResolvedValue(undefined);
jest.mock('../src/services/ledger', () => ({
  getCurrentEpoch: jest.fn().mockResolvedValue(42),
  recordAccountCreate: mockRecordAccountCreate
}));
jest.mock('../src/models/User', () => ({
  findOne: jest.fn().mockResolvedValue(null)
}));

function evmMessageHash(message) {
  const prefix = '\x19Ethereum Signed Message:\n' + message.length + message;
  return Buffer.from(keccak256(Buffer.from(prefix)), 'hex');
}

function privateKeyToAddress(privateKey) {
  const pubKey = secp256k1.publicKeyCreate(privateKey, false);
  const pubKeyHash = keccak256(Buffer.from(pubKey.slice(1)));
  return '0x' + pubKeyHash.slice(-40);
}

function signEVM(message, privateKey) {
  const signed = secp256k1.ecdsaSign(evmMessageHash(message), privateKey);
  return '0x' + Buffer.concat([
    Buffer.from(signed.signature),
    Buffer.from([signed.recid + 27])
  ]).toString('hex');
}

describe('chainLink', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockRecordAccountCreate.mockClear();
  });

  test('generateChallenge creates an EVM challenge with a stable signed message', () => {
    const chainLink = require('../src/services/chainLink');
    const challenge = chainLink.generateChallenge('alice', 'evm', '0x1111111111111111111111111111111111111111');

    expect(challenge.challengeId).toMatch(/^[0-9a-f]{32}$/);
    expect(challenge.message).toContain('BTCPC-LINK:alice:evm:0x1111111111111111111111111111111111111111:');
    expect(challenge.expiresIn).toBe(600);
  });

  test('recoverEVMAddress recovers the signer address from a challenge signature', () => {
    const chainLink = require('../src/services/chainLink');
    const privateKey = Buffer.from('1'.repeat(64), 'hex');
    const address = privateKeyToAddress(privateKey);
    const message = 'BTCPC-LINK:alice:evm:' + address + ':test';
    const signature = signEVM(message, privateKey);

    expect(chainLink.recoverEVMAddress(message, signature)).toBe(address);
  });

  test('verifyAndLink records a ledger link when recovered EVM address matches', async () => {
    const chainLink = require('../src/services/chainLink');
    const privateKey = Buffer.from('2'.repeat(64), 'hex');
    const address = privateKeyToAddress(privateKey);
    const challenge = chainLink.generateChallenge('alice', 'evm', address);
    const signature = signEVM(challenge.message, privateKey);

    const result = await chainLink.verifyAndLink(challenge.challengeId, signature);

    expect(result).toEqual(expect.objectContaining({
      success: true,
      username: 'alice',
      chain: 'evm',
      address
    }));
    expect(mockRecordAccountCreate).toHaveBeenCalledWith(
      'alice',
      {},
      { evm: address },
      42,
      'chain-link:evm:' + address
    );
  });

  test('verifyAndLink rejects signatures for a different EVM address', async () => {
    const chainLink = require('../src/services/chainLink');
    const privateKey = Buffer.from('3'.repeat(64), 'hex');
    const wrongAddress = '0x1111111111111111111111111111111111111111';
    const challenge = chainLink.generateChallenge('alice', 'evm', wrongAddress);
    const signature = signEVM(challenge.message, privateKey);

    const result = await chainLink.verifyAndLink(challenge.challengeId, signature);

    expect(result.success).toBe(false);
    expect(result.error).toContain('Address mismatch');
  });
});
