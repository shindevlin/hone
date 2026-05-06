"use strict";

const crypto = require("crypto");
const chainLink = require("./chainLink");
const stateStore = require("../chain/stateStore");
const { sanitizeAmount, sanitizeString } = require("../middlewares/validate");

const CHALLENGE_TTL_MS = 10 * 60 * 1000;
const transferChallenges = new Map();

function _challengeId() {
  return crypto.randomBytes(16).toString("hex");
}

function _normalizeChain(chain) {
  return String(chain || "").trim().toLowerCase();
}

function _normalizeRecipient(transfer) {
  return sanitizeString(
    transfer && (transfer.toAddress || transfer.to || transfer.recipient || ""),
    200,
  );
}

function _normalizeToken(transfer) {
  return sanitizeString((transfer && transfer.token) || "BTCPC", 20) || "BTCPC";
}

function _normalizeMemo(transfer) {
  return sanitizeString((transfer && transfer.memo) || "", 500) || "";
}

function _normalizeAddress(address) {
  return sanitizeString(address || "", 200);
}

function _challengeText(username, normalizedTransfer, chain, challengeId, address) {
  return [
    "BTCPC CHAIN TRANSFER",
    "challenge_id=" + challengeId,
    "account=" + username,
    "chain=" + chain,
    "address=" + (address || ""),
    "to=" + (normalizedTransfer.to || ""),
    "amount=" + String(normalizedTransfer.amount || ""),
    "token=" + normalizedTransfer.token,
    "memo=" + normalizedTransfer.memo,
  ].join("\n");
}

function _cleanupExpired() {
  const now = Date.now();
  for (const [id, rec] of transferChallenges.entries()) {
    if (!rec || rec.expiresAt <= now || rec.used) {
      transferChallenges.delete(id);
    }
  }
}

function _getLinkedAddresses(username) {
  const linked = chainLink.getLinkedAddresses ? chainLink.getLinkedAddresses(username) : {};
  return linked || {};
}

function _getMnemonicWalletAddress(username, chain) {
  const acct = stateStore.getAccount ? stateStore.getAccount(username) : null;
  const addresses = acct && acct.chain_addresses ? acct.chain_addresses : null;
  if (!addresses) return null;
  const family = _normalizeChain(chain);
  const address = addresses[family];
  return address ? _normalizeAddress(address) : null;
}

function buildTransferChallenge(username, transferInput) {
  _cleanupExpired();
  const chain = _normalizeChain(transferInput && transferInput.chain);
  const to = _normalizeRecipient(transferInput);
  const amount = sanitizeAmount(transferInput && transferInput.amount);
  const token = _normalizeToken(transferInput);
  const memo = _normalizeMemo(transferInput);
  const mnemonicAddress = _getMnemonicWalletAddress(username, chain);
  const address = _normalizeAddress(transferInput && transferInput.address) || mnemonicAddress;

  if (!chain) throw new Error("approval chain required");
  if (!to) throw new Error("recipient required");
  if (!amount) throw new Error("amount required");
  if (!mnemonicAddress) {
    throw new Error("no BTCPC mnemonic-linked wallet available for " + chain + " controller mode");
  }
  if (address && _normalizeAddress(address) !== mnemonicAddress) {
    throw new Error("controller approval must use the BTCPC mnemonic-linked " + chain + " wallet");
  }

  const challengeId = _challengeId();
  const normalizedTransfer = { to, amount, token, memo, chain, address };
  const challenge = _challengeText(username, normalizedTransfer, chain, challengeId, address);
  const record = {
    challengeId,
    username,
    chain,
    address,
    transfer: {
      to,
      amount,
      token,
      memo,
      chain,
      address,
    },
    challenge,
    createdAt: Date.now(),
    expiresAt: Date.now() + CHALLENGE_TTL_MS,
    used: false,
  };
  transferChallenges.set(challengeId, record);

  return {
    challengeId,
    challenge,
    expiresAt: new Date(record.expiresAt).toISOString(),
    transfer: record.transfer,
    signer: chain,
    address,
  };
}

function _getChallengeRecord(challengeId) {
  _cleanupExpired();
  const rec = transferChallenges.get(challengeId);
  if (!rec) return null;
  if (rec.used || rec.expiresAt <= Date.now()) {
    transferChallenges.delete(challengeId);
    return null;
  }
  return rec;
}

async function verifyTransferAuthorization(username, transferInput, authInput) {
  const auth = authInput || {};
  const chain = _normalizeChain(auth.chain || auth.approval_chain || auth.signer_chain || auth.approvalChain);
  const challengeId = sanitizeString(
    auth.challengeId || auth.challenge_id || auth.chain_challenge_id || auth.active_challenge_id || "",
    120,
  );
  const challenge = sanitizeString(auth.challenge || auth.chain_challenge || auth.active_challenge || "", 1200);
  const signature = sanitizeString(auth.signature || auth.chain_signature || auth.active_signature || "", 512);
  const address = _normalizeAddress(auth.address || auth.signer_address || auth.public_address || "");

  if (!challengeId || !challenge || !signature || !chain) {
    return {
      ok: false,
      status: 403,
      error: "chain authorization requires chain, challenge, signature, and challengeId",
    };
  }

  const rec = _getChallengeRecord(challengeId);
  if (!rec) {
    return {
      ok: false,
      status: 400,
      error: "chain challenge not found or expired",
    };
  }
  if (rec.username !== username) {
    return {
      ok: false,
      status: 403,
      error: "chain challenge does not belong to this account",
    };
  }

  const normalizedTransfer = {
    to: _normalizeRecipient(transferInput),
    amount: sanitizeAmount(transferInput && transferInput.amount),
    token: _normalizeToken(transferInput),
    memo: _normalizeMemo(transferInput),
    chain: _normalizeChain(transferInput && (transferInput.approval_chain || transferInput.chain) || chain),
    address: _normalizeAddress(transferInput && (transferInput.address || transferInput.signer_address || "")) || (rec && rec.transfer ? rec.transfer.address : ""),
  };
  const mnemonicAddress = _getMnemonicWalletAddress(username, chain);
  if (
    rec.transfer.to !== normalizedTransfer.to ||
    rec.transfer.amount !== normalizedTransfer.amount ||
    rec.transfer.token !== normalizedTransfer.token ||
    rec.transfer.memo !== normalizedTransfer.memo ||
    rec.transfer.chain !== normalizedTransfer.chain ||
    ((normalizedTransfer.address || "") && (rec.transfer.address || "") !== (normalizedTransfer.address || ""))
  ) {
    return {
      ok: false,
      status: 400,
      error: "chain challenge transfer mismatch",
    };
  }
  if (challenge !== rec.challenge) {
    return {
      ok: false,
      status: 400,
      error: "chain challenge mismatch",
    };
  }
  if (!mnemonicAddress) {
    return {
      ok: false,
      status: 422,
      error: "no BTCPC mnemonic-linked wallet available for this approval chain",
    };
  }
  if ((rec.address || "") !== mnemonicAddress) {
    return {
      ok: false,
      status: 403,
      error: "controller approval must use the BTCPC mnemonic-linked wallet",
    };
  }

  let recovered;
  try {
    recovered = chainLink.verifySignatureForChain(chain, challenge, signature, mnemonicAddress);
  } catch (err) {
    return { ok: false, status: 401, error: "signature does not match the BTCPC mnemonic-linked wallet: " + err.message };
  }

  const normalizedRecovered = String(recovered.recoveredAddress || "").trim().toLowerCase();
  if (normalizedRecovered !== mnemonicAddress) {
    return {
      ok: false,
      status: 403,
      error: "signer is not the BTCPC mnemonic-linked wallet",
    };
  }

  rec.used = true;
  transferChallenges.delete(challengeId);

  return {
    ok: true,
    signer: chain,
    challengeId,
    challenge,
    recoveredAddress: recovered.recoveredAddress,
    transfer: normalizedTransfer,
    expiresAt: new Date(rec.expiresAt).toISOString(),
  };
}

module.exports = {
  buildTransferChallenge,
  verifyTransferAuthorization,
};
