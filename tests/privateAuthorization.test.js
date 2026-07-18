const secp256k1 = require('secp256k1');
const { keccak256 } = require('js-sha3');

jest.mock('../src/models/User', () => ({
  findOne: jest.fn()
}));
jest.mock('axios', () => ({
  post: jest.fn(),
  get: jest.fn()
}));

const User = require('../src/models/User');
const axios = require('axios');
const privateAuthorization = require('../src/services/privateAuthorization');

function evmMessageHash(message) {
  const prefix = '\x19Ethereum Signed Message:\n' + message.length + message;
  return Buffer.from(keccak256(Buffer.from(prefix)), 'hex');
}

function privateKeyToAddress(privateKey) {
  const pubKey = secp256k1.publicKeyCreate(privateKey, false);
  const pubKeyHash = keccak256(Buffer.from(pubKey.slice(1)));
  return '0x' + pubKeyHash.slice(-40);
}

function signEvm(message, privateKey) {
  const signed = secp256k1.ecdsaSign(evmMessageHash(message), privateKey);
  return '0x' + Buffer.concat([
    Buffer.from(signed.signature),
    Buffer.from([signed.recid + 27])
  ]).toString('hex');
}

describe('privateAuthorization', () => {
  let userDoc;

  beforeEach(() => {
    privateAuthorization.resetState();
    jest.clearAllMocks();
    process.env.HONE_PRIVATE_AUTH_ENABLED = 'true';
    delete process.env.HONE_LIGHTNING_PROVIDER_URL;
    delete process.env.HONE_LIGHTNING_PROVIDER_KEY;
    delete process.env.HONE_ZK_VERIFIER_URL;
    userDoc = {
      username: 'alice',
      privateAuth: { enabled: false, threshold: 1, factors: [], updatedAt: new Date() },
      save: jest.fn().mockResolvedValue(undefined)
    };
    User.findOne.mockImplementation(async (query) => {
      if (query && query.username === 'alice') return userDoc;
      return null;
    });
  });

  test('enrolls a hidden EVM factor without storing the address', async () => {
    const key = Buffer.from('1'.repeat(64), 'hex');
    const address = privateKeyToAddress(key);

    const challenge = await privateAuthorization.requestEnrollment('alice', 'evm', 'daily approval');
    const signature = signEvm(challenge.message, key);
    const result = await privateAuthorization.verifyEnrollment(challenge.challengeId, signature);

    expect(result).toEqual(expect.objectContaining({
      success: true,
      username: 'alice',
      chain: 'evm',
      hidden: true
    }));
    expect(userDoc.privateAuth.enabled).toBe(true);
    expect(userDoc.privateAuth.factors).toHaveLength(1);
    expect(userDoc.privateAuth.factors[0]).toEqual(expect.objectContaining({
      chain: 'evm',
      factorId: challenge.factorId
    }));
    expect(userDoc.privateAuth.factors[0].commitment).not.toContain(address.slice(2));
  });

  test('verifies a transfer only after threshold approvals from hidden factors', async () => {
    const keyOne = Buffer.from('2'.repeat(64), 'hex');
    const keyTwo = Buffer.from('3'.repeat(64), 'hex');

    const enrollmentOne = await privateAuthorization.requestEnrollment('alice', 'evm', 'primary hidden');
    await privateAuthorization.verifyEnrollment(enrollmentOne.challengeId, signEvm(enrollmentOne.message, keyOne));

    const enrollmentTwo = await privateAuthorization.requestEnrollment('alice', 'evm', 'backup hidden');
    await privateAuthorization.verifyEnrollment(enrollmentTwo.challengeId, signEvm(enrollmentTwo.message, keyTwo));

    await privateAuthorization.setPolicy('alice', { threshold: 2, enabled: true });

    const transfer = await privateAuthorization.requestTransferAuthorization('alice', {
      from: 'alice',
      to: 'bob',
      amount: 7,
      token: 'HONE',
      memo: 'payment',
      approval_chain: 'evm'
    });

    const result = await privateAuthorization.verifyTransferAuthorization('alice', {
      from: 'alice',
      to: 'bob',
      amount: 7,
      token: 'HONE',
      memo: 'payment'
    }, {
      requestId: transfer.requestId,
      approvals: [
        { factorId: enrollmentOne.factorId, chain: 'evm', signature: signEvm(transfer.message, keyOne) },
        { factorId: enrollmentTwo.factorId, chain: 'evm', signature: signEvm(transfer.message, keyTwo) }
      ]
    });

    expect(result).toEqual(expect.objectContaining({
      requestId: transfer.requestId,
      threshold: 2,
      approvalCount: 2,
      hidden: true
    }));
  });

  test('rejects transfer authorization when approvals are below threshold', async () => {
    const key = Buffer.from('4'.repeat(64), 'hex');
    const enrollment = await privateAuthorization.requestEnrollment('alice', 'evm', 'single factor');
    await privateAuthorization.verifyEnrollment(enrollment.challengeId, signEvm(enrollment.message, key));
    await privateAuthorization.setPolicy('alice', { threshold: 2, enabled: true });

    const transfer = await privateAuthorization.requestTransferAuthorization('alice', {
      from: 'alice',
      to: 'bob',
      amount: 1,
      token: 'HONE',
      memo: 'test',
      approval_chain: 'evm'
    });

    await expect(privateAuthorization.verifyTransferAuthorization('alice', {
      from: 'alice',
      to: 'bob',
      amount: 1,
      token: 'HONE',
      memo: 'test'
    }, {
      requestId: transfer.requestId,
      approvals: [
        { factorId: enrollment.factorId, chain: 'evm', signature: signEvm(transfer.message, key) }
      ]
    })).rejects.toThrow('Insufficient approvals');
  });

  test('supports lightning invoice enrollment and transfer authorization', async () => {
    process.env.HONE_LIGHTNING_PROVIDER_URL = 'https://ln.example';
    axios.post.mockResolvedValueOnce({ data: { invoice: 'ln-invoice-enroll', payment_hash: 'enroll-hash' } });
    axios.get.mockResolvedValueOnce({ data: { settled: true, amount_sats: 1 } });

    const enroll = await privateAuthorization.requestEnrollment('alice', 'lightning', 'mobile lightning');
    expect(enroll.invoice).toBe('ln-invoice-enroll');
    const enrolled = await privateAuthorization.verifyEnrollment(enroll.challengeId, {
      receipt: { payment_hash: 'enroll-hash', settled: true }
    });
    expect(enrolled.chain).toBe('lightning');

    await privateAuthorization.setPolicy('alice', { threshold: 1, enabled: true });

    axios.post.mockResolvedValueOnce({ data: { invoice: 'ln-invoice-transfer', payment_hash: 'transfer-hash' } });
    axios.get.mockResolvedValueOnce({ data: { settled: true, amount_sats: 1 } });

    const transfer = await privateAuthorization.requestTransferAuthorization('alice', {
      from: 'alice',
      to: 'bob',
      amount: 3,
      token: 'HONE',
      memo: 'ln pay',
      approval_chain: 'lightning'
    });

    const result = await privateAuthorization.verifyTransferAuthorization('alice', {
      from: 'alice',
      to: 'bob',
      amount: 3,
      token: 'HONE',
      memo: 'ln pay'
    }, {
      requestId: transfer.requestId,
      approvalChain: 'lightning',
      approvals: [
        {
          factorId: enroll.factorId,
          chain: 'lightning',
          receipt: { payment_hash: 'transfer-hash', settled: true }
        }
      ]
    });

    expect(result.hidden).toBe(true);
    expect(axios.post).toHaveBeenCalled();
    expect(axios.get).toHaveBeenCalled();
  });

  test('rejects lightning enrollment when no provider is configured', async () => {
    await expect(privateAuthorization.requestEnrollment('alice', 'lightning', 'mobile lightning')).rejects.toThrow(
      'Lightning provider not configured'
    );
  });

  test('supports zkvm verifier-backed authorization receipts', async () => {
    process.env.HONE_ZK_VERIFIER_URL = 'https://zk.example';
    axios.post.mockResolvedValue({ data: { valid: true, receipt_id: 'zk-ok' } });

    const enroll = await privateAuthorization.requestEnrollment('alice', 'zkvm', 'zk factor');
    const enrolled = await privateAuthorization.verifyEnrollment(enroll.challengeId, {
      proof: { public_inputs: { account: 'alice' }, proof_bytes: '0x1234' }
    });
    expect(enrolled.chain).toBe('zkvm');

    await privateAuthorization.setPolicy('alice', { threshold: 1, enabled: true });

    const transfer = await privateAuthorization.requestTransferAuthorization('alice', {
      from: 'alice',
      to: 'bob',
      amount: 4,
      token: 'HONE',
      memo: 'zk pay',
      approval_chain: 'zkvm',
      proof_backend: 'sp1'
    });

    const result = await privateAuthorization.verifyTransferAuthorization('alice', {
      from: 'alice',
      to: 'bob',
      amount: 4,
      token: 'HONE',
      memo: 'zk pay'
    }, {
      requestId: transfer.requestId,
      approvalChain: 'zkvm',
      approvals: [
        {
          factorId: enroll.factorId,
          chain: 'zkvm',
          proof_backend: 'sp1',
          proof: { public_inputs: { account: 'alice' }, proof_bytes: '0x1234' }
        }
      ]
    });

    expect(result.hidden).toBe(true);
    expect(axios.post).toHaveBeenCalled();
  });

  test('exposes staged preview copy and route summary helpers', async () => {
    const bitcoinCopy = privateAuthorization.getChainPreviewCopy('bitcoin');
    expect(bitcoinCopy.title).toContain('Bitcoin');
    expect(bitcoinCopy.summary).toContain('signed Bitcoin challenge');

    const summary = privateAuthorization.getPrivateAuthRouteSummary('/api/wallet/private-auth');
    expect(summary.some((route) => route.path.includes('/preview'))).toBe(true);
    expect(summary.some((route) => route.path.endsWith('/transfer/verify'))).toBe(true);
  });

  test('persists controller policy metadata for the HONE mnemonic-linked controller', async () => {
    await privateAuthorization.setPolicy('alice', {
      enabled: false,
      threshold: 1,
      controller: {
        provider: 'privy',
        mode: 'linked_chain',
        approvalChain: 'evm',
        walletSource: 'outside_wallet',
        allowActiveKey: false
      }
    });

    const policy = await privateAuthorization.getPolicy('alice');
    expect(policy.controller).toEqual(expect.objectContaining({
      provider: 'privy',
      mode: 'linked_chain',
      approvalChain: 'evm',
      walletSource: 'hone_mnemonic',
      allowActiveKey: false
    }));
  });

  test('forces controller wallet source to the HONE mnemonic-derived wallet', async () => {
    await privateAuthorization.setPolicy('alice', {
      enabled: false,
      threshold: 1,
      controller: {
        provider: 'privy',
        mode: 'linked_chain',
        approvalChain: 'bitcoin',
        walletSource: 'outside_wallet',
        allowActiveKey: true
      }
    });

    const policy = await privateAuthorization.getPolicy('alice');
    expect(policy.controller.walletSource).toBe('hone_mnemonic');
    expect(policy.controller.mode).toBe('linked_chain');
  });

  test('normalizes controller approval chains to wallet chains only', async () => {
    await privateAuthorization.setPolicy('alice', {
      enabled: false,
      threshold: 1,
      controller: {
        provider: 'privy',
        mode: 'linked_chain',
        approvalChain: 'lightning',
        allowActiveKey: false
      }
    });

    const policy = await privateAuthorization.getPolicy('alice');
    expect(policy.controller.approvalChain).toBe('evm');
  });
});
