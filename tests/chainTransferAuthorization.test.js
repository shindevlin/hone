jest.mock('../src/chain/stateStore', () => ({
  getAccount: jest.fn()
}));

jest.mock('../src/services/chainLink', () => ({
  verifySignatureForChain: jest.fn(() => ({
    recoveredAddress: '0x' + 'a'.repeat(40),
    chain: 'evm'
  }))
}));

const chainTransferAuthorization = require('../src/services/chainTransferAuthorization');
const stateStore = require('../src/chain/stateStore');
const chainLink = require('../src/services/chainLink');

describe('chainTransferAuthorization', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    stateStore.getAccount.mockReturnValue({
      chain_addresses: {
        evm: '0x' + 'a'.repeat(40)
      }
    });
  });

  test('builds a challenge for the HONE mnemonic-linked wallet', () => {
    const challenge = chainTransferAuthorization.buildTransferChallenge('alice', {
      toAddress: 'bob',
      amount: 3,
      chain: 'evm'
    });

    expect(challenge.address).toBe('0x' + 'a'.repeat(40));
    expect(challenge.transfer.address).toBe('0x' + 'a'.repeat(40));
  });

  test('rejects a controller signature from a non-mnemonic wallet', async () => {
    stateStore.getAccount.mockReturnValue({
      chain_addresses: {
        evm: '0x' + 'a'.repeat(40)
      }
    });
    chainLink.verifySignatureForChain.mockImplementationOnce(() => {
      throw new Error('Address mismatch. Expected 0x' + 'a'.repeat(40) + ', recovered 0x' + 'b'.repeat(40));
    });

    const challenge = chainTransferAuthorization.buildTransferChallenge('alice', {
      toAddress: 'bob',
      amount: 3,
      chain: 'evm'
    });

    const result = await chainTransferAuthorization.verifyTransferAuthorization(
      'alice',
      {
        from: 'alice',
        to: 'bob',
        amount: 3,
        token: 'HONE',
        memo: ''
      },
      {
        chain: 'evm',
        challengeId: challenge.challengeId,
        challenge: challenge.challenge,
        signature: '0x01'
      }
    );

    expect(result.ok).toBe(false);
    expect(result.error).toMatch(/mnemonic-linked wallet/);
  });

  test('verifies a controller signature when the transfer payload omits the signer address', async () => {
    const challenge = chainTransferAuthorization.buildTransferChallenge('alice', {
      toAddress: 'bob',
      amount: 3,
      chain: 'evm'
    });

    const result = await chainTransferAuthorization.verifyTransferAuthorization(
      'alice',
      {
        from: 'alice',
        to: 'bob',
        amount: 3,
        token: 'HONE',
        memo: ''
      },
      {
        chain: 'evm',
        challengeId: challenge.challengeId,
        challenge: challenge.challenge,
        signature: '0x01'
      }
    );

    expect(result.ok).toBe(true);
    expect(result.recoveredAddress).toBe('0x' + 'a'.repeat(40));
  });
});
