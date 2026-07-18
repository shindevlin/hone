"use strict";

const crypto = require('crypto');
const secp256k1 = require('secp256k1');
const { keccak256 } = require('js-sha3');
const axios = require('axios');

let pendingChallenges = new Map();
let pendingTransfers = new Map();
let policyCache = new Map();

// Chains where address derivation is meaningful for custody
const WALLET_CHAINS = new Set(['evm', 'solana', 'ton', 'bitcoin', 'hone']);

function resetState() {
  pendingChallenges.clear();
  pendingTransfers.clear();
  policyCache.clear();
}

function _evmMessageHash(message) {
  const prefix = '\x19Ethereum Signed Message:\n' + message.length + message;
  return Buffer.from(keccak256(Buffer.from(prefix)), 'hex');
}

function _recoverEvmAddress(message, signature) {
  const sigBuf = Buffer.from(signature.replace(/^0x/, ''), 'hex');
  const sigData = sigBuf.slice(0, 64);
  const recid = sigBuf[64] - 27;
  const msgHash = _evmMessageHash(message);
  const pubKey = secp256k1.ecdsaRecover(sigData, recid, msgHash, false);
  const pubKeyHash = keccak256(Buffer.from(pubKey.slice(1)));
  return '0x' + pubKeyHash.slice(-40);
}

function _sha256(data) {
  return crypto.createHash('sha256').update(data).digest('hex');
}

async function _loadUser(username) {
  const User = require('../models/User');
  const user = await User.findOne({ username });
  if (!user) throw new Error('User not found: ' + username);
  if (!user.privateAuth) {
    user.privateAuth = { enabled: false, threshold: 1, factors: [], updatedAt: new Date() };
  }
  if (!Array.isArray(user.privateAuth.factors)) {
    user.privateAuth.factors = [];
  }
  return user;
}

async function requestEnrollment(username, chain, label) {
  if (chain === 'lightning') {
    const providerUrl = process.env.HONE_LIGHTNING_PROVIDER_URL;
    if (!providerUrl) throw new Error('Lightning provider not configured');
    const resp = await axios.post(providerUrl + '/invoice', { amount_sats: 1, memo: 'hone-auth-enroll' });
    const challengeId = crypto.randomBytes(16).toString('hex');
    const factorId = crypto.randomBytes(16).toString('hex');
    pendingChallenges.set(challengeId, { username, chain, label, factorId, payment_hash: resp.data.payment_hash });
    return { challengeId, factorId, username, chain, label, invoice: resp.data.invoice, payment_hash: resp.data.payment_hash };
  }

  const challengeId = crypto.randomBytes(16).toString('hex');
  const factorId = crypto.randomBytes(16).toString('hex');
  const message = 'hone-auth-enroll:' + challengeId;
  pendingChallenges.set(challengeId, { username, chain, label, factorId, message });
  return { challengeId, factorId, username, chain, label, message };
}

async function verifyEnrollment(challengeId, signatureOrReceipt) {
  const challenge = pendingChallenges.get(challengeId);
  if (!challenge) throw new Error('Challenge not found');
  pendingChallenges.delete(challengeId);

  const user = await _loadUser(challenge.username);
  let commitment;

  if (challenge.chain === 'lightning') {
    const providerUrl = process.env.HONE_LIGHTNING_PROVIDER_URL;
    const receipt = (signatureOrReceipt && signatureOrReceipt.receipt) || signatureOrReceipt;
    const resp = await axios.get(providerUrl + '/invoice/' + receipt.payment_hash);
    if (!resp.data.settled) throw new Error('Lightning payment not settled');
    commitment = _sha256('lightning:' + receipt.payment_hash);
  } else if (challenge.chain === 'zkvm') {
    const verifierUrl = process.env.HONE_ZK_VERIFIER_URL;
    const proof = (signatureOrReceipt && signatureOrReceipt.proof) || signatureOrReceipt;
    const resp = await axios.post(verifierUrl + '/verify', { proof, challenge_id: challengeId });
    if (!resp.data.valid) throw new Error('zkvm proof invalid');
    commitment = _sha256('zkvm:' + (resp.data.receipt_id || challengeId));
  } else {
    const recoveredAddress = _recoverEvmAddress(challenge.message, signatureOrReceipt);
    commitment = _sha256('evm:' + recoveredAddress.toLowerCase());
  }

  user.privateAuth.factors.push({
    chain: challenge.chain,
    factorId: challenge.factorId,
    label: challenge.label,
    commitment,
    enrolledAt: new Date()
  });
  user.privateAuth.enabled = true;
  await user.save();

  return { success: true, username: challenge.username, chain: challenge.chain, factorId: challenge.factorId, hidden: true };
}

async function setPolicy(username, { threshold, enabled, controller }) {
  const user = await _loadUser(username);
  if (threshold !== undefined) user.privateAuth.threshold = threshold;
  if (enabled !== undefined) user.privateAuth.enabled = !!enabled;

  if (controller) {
    const normalized = Object.assign({}, controller);
    normalized.walletSource = 'hone_mnemonic';
    if (normalized.approvalChain && !WALLET_CHAINS.has(normalized.approvalChain)) {
      normalized.approvalChain = 'evm';
    }
    user.privateAuth.controller = normalized;
  }

  user.privateAuth.updatedAt = new Date();
  await user.save();

  const policy = {
    user: username,
    enabled: user.privateAuth.enabled,
    threshold: user.privateAuth.threshold,
    enrolled_devices: user.privateAuth.factors,
    controller: user.privateAuth.controller || null
  };
  policyCache.set(username, policy);
  return policy;
}

async function getPolicy(username) {
  if (policyCache.has(username)) return policyCache.get(username);
  try {
    const user = await _loadUser(username);
    const policy = {
      user: username,
      enabled: user.privateAuth.enabled || false,
      threshold: user.privateAuth.threshold || 0,
      enrolled_devices: user.privateAuth.factors,
      controller: user.privateAuth.controller || null
    };
    policyCache.set(username, policy);
    return policy;
  } catch (e) {
    return { user: username, enabled: false, threshold: 0, enrolled_devices: [], controller: null };
  }
}

async function requestTransferAuthorization(username, transferDetails) {
  const requestId = crypto.randomBytes(16).toString('hex');
  let invoice = null;
  let paymentHash = null;

  if (transferDetails.approval_chain === 'lightning') {
    const providerUrl = process.env.HONE_LIGHTNING_PROVIDER_URL;
    if (!providerUrl) throw new Error('Lightning provider not configured');
    const resp = await axios.post(providerUrl + '/invoice', {
      amount_sats: 1,
      memo: 'hone-auth-transfer-' + requestId
    });
    invoice = resp.data.invoice;
    paymentHash = resp.data.payment_hash;
  }

  const message = 'hone-transfer-auth:' + requestId + ':' + JSON.stringify({
    from: transferDetails.from,
    to: transferDetails.to,
    amount: transferDetails.amount,
    token: transferDetails.token || 'HONE',
    memo: transferDetails.memo || ''
  });

  pendingTransfers.set(requestId, { username, requestId, message, transferDetails, payment_hash: paymentHash });

  const result = Object.assign({ requestId, username, message }, transferDetails);
  if (invoice) {
    result.invoice = invoice;
    result.payment_hash = paymentHash;
  }
  return result;
}

async function verifyTransferAuthorization(username, transferData, privateAuth) {
  const user = await _loadUser(username);
  const threshold = user.privateAuth.threshold || 1;
  const factors = user.privateAuth.factors || [];

  const request = pendingTransfers.get(privateAuth.requestId);
  if (!request) throw new Error('Transfer request not found');

  const approvals = privateAuth.approvals || [];
  let approvalCount = 0;

  for (const approval of approvals) {
    const factor = factors.find(function(f) { return f.factorId === approval.factorId; });
    if (!factor) continue;

    if (approval.chain === 'evm') {
      const recoveredAddress = _recoverEvmAddress(request.message, approval.signature);
      const commitment = _sha256('evm:' + recoveredAddress.toLowerCase());
      if (commitment === factor.commitment) approvalCount++;
    } else if (approval.chain === 'lightning') {
      const providerUrl = process.env.HONE_LIGHTNING_PROVIDER_URL;
      const receipt = approval.receipt || {};
      const resp = await axios.get(providerUrl + '/invoice/' + receipt.payment_hash);
      if (resp.data.settled) approvalCount++;
    } else if (approval.chain === 'zkvm') {
      const verifierUrl = process.env.HONE_ZK_VERIFIER_URL;
      const resp = await axios.post(verifierUrl + '/verify', {
        proof: approval.proof,
        proof_backend: approval.proof_backend,
        request_id: privateAuth.requestId
      });
      if (resp.data.valid) approvalCount++;
    }
  }

  if (approvalCount < threshold) {
    throw new Error('Insufficient approvals: ' + approvalCount + ' of ' + threshold + ' required');
  }

  return {
    requestId: privateAuth.requestId,
    threshold,
    approvalCount,
    hidden: true,
    factors: approvals.map(function(a) { return a.factorId; })
  };
}

function getChainPreviewCopy(chain) {
  const copies = {
    bitcoin: {
      title: 'Bitcoin Multi-Factor Authorization',
      summary: 'Authorize transfers by submitting a signed Bitcoin challenge from your enrolled Bitcoin wallet.'
    },
    evm: {
      title: 'EVM Wallet Authorization',
      summary: 'Sign a challenge with your enrolled EVM-compatible wallet to authorize high-value transfers.'
    },
    lightning: {
      title: 'Lightning Network Authorization',
      summary: 'Pay a micro-invoice from your enrolled Lightning wallet to authorize transfers.'
    },
    zkvm: {
      title: 'ZK Proof Authorization',
      summary: 'Submit a zero-knowledge proof to authorize transfers without revealing private credentials.'
    }
  };
  return copies[chain] || { title: chain + ' Authorization', summary: 'Authorize via ' + chain + ' challenge.' };
}

function getPrivateAuthRouteSummary(basePath) {
  return [
    { path: basePath + '/preview', method: 'GET', description: 'Preview chain authorization options' },
    { path: basePath + '/enroll/request', method: 'POST', description: 'Request an enrollment challenge' },
    { path: basePath + '/enroll/verify', method: 'POST', description: 'Verify enrollment challenge response' },
    { path: basePath + '/policy', method: 'GET', description: 'Get authorization policy' },
    { path: basePath + '/policy', method: 'PUT', description: 'Update authorization policy' },
    { path: basePath + '/transfer/request', method: 'POST', description: 'Request transfer authorization challenge' },
    { path: basePath + '/transfer/verify', method: 'POST', description: 'Verify transfer authorization approvals' }
  ];
}

module.exports = {
  resetState,
  requestEnrollment,
  verifyEnrollment,
  setPolicy,
  getPolicy,
  requestTransferAuthorization,
  verifyTransferAuthorization,
  getChainPreviewCopy,
  getPrivateAuthRouteSummary
};
