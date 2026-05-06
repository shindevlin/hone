"use strict";

const crypto = require("crypto");
const secp256k1 = require("secp256k1");
const stateStore = require("../chain/stateStore");
const { sanitizeAmount, sanitizeString } = require("../middlewares/validate");

const CHALLENGE_TTL_MS = 10 * 60 * 1000;
const transferChallenges = new Map();

function _challengeId() {
  return crypto.randomBytes(16).toString("hex");
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

function _challengeText(username, normalizedTransfer, challengeId) {
  return [
    "BTCPC ACTIVE TRANSFER",
    "challenge_id=" + challengeId,
    "account=" + username,
    "to=" + (normalizedTransfer.to || ""),
    "amount=" + String(normalizedTransfer.amount || ""),
    "token=" + normalizedTransfer.token,
    "memo=" + normalizedTransfer.memo,
  ].join("\n");
}

function _challengeRecord(username, transfer, challengeId) {
  return {
    challengeId,
    username,
    transfer: {
      to: transfer.to,
      amount: transfer.amount,
      token: transfer.token,
      memo: transfer.memo,
    },
    challenge: _challengeText(username, transfer, challengeId),
    createdAt: Date.now(),
    expiresAt: Date.now() + CHALLENGE_TTL_MS,
    used: false,
  };
}

function _cleanupExpired() {
  const now = Date.now();
  for (const [id, rec] of transferChallenges.entries()) {
    if (!rec || rec.expiresAt <= now || rec.used) {
      transferChallenges.delete(id);
    }
  }
}

function _getActivePublicKey(username) {
  const acct = stateStore.getAccount ? stateStore.getAccount(username) : null;
  const activePub = acct && acct.public_keys && acct.public_keys.active;
  if (activePub) return activePub;
  try {
    const User = require("../models/User");
    return User.findOne({ username })
      .then((user) => {
        if (!user || !user.activePublicKey) return null;
        return user.activePublicKey;
      })
      .catch(() => null);
  } catch (_) {
    return Promise.resolve(null);
  }
}

function buildTransferChallenge(username, transferInput) {
  _cleanupExpired();
  const to = _normalizeRecipient(transferInput);
  const amount = sanitizeAmount(transferInput && transferInput.amount);
  const token = _normalizeToken(transferInput);
  const memo = _normalizeMemo(transferInput);
  if (!to) throw new Error("recipient required");
  if (!amount) throw new Error("amount required");

  const challengeId = _challengeId();
  const normalizedTransfer = { to, amount, token, memo };
  const record = _challengeRecord(username, normalizedTransfer, challengeId);
  transferChallenges.set(challengeId, record);

  return {
    challengeId,
    challenge: record.challenge,
    expiresAt: new Date(record.expiresAt).toISOString(),
    transfer: record.transfer,
    signer: "active",
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

async function verifyActiveTransferSignature(
  username,
  transferInput,
  authInput,
) {
  const activePub = await _getActivePublicKey(username);
  if (!activePub) {
    return {
      ok: false,
      status: 422,
      error: "account has no active key registered on chain",
    };
  }

  const auth = authInput || {};
  const challengeId = sanitizeString(
    auth.active_challenge_id || auth.challengeId || auth.challenge_id || "",
    120,
  );
  const challenge = sanitizeString(
    auth.active_challenge || auth.challenge || "",
    1200,
  );
  const signature = sanitizeString(
    auth.active_signature || auth.signature || "",
    512,
  );
  if (!challengeId || !challenge || !signature) {
    return {
      ok: false,
      status: 403,
      error: "active key signature required",
    };
  }

  const rec = _getChallengeRecord(challengeId);
  if (!rec) {
    return {
      ok: false,
      status: 400,
      error: "active challenge not found or expired",
    };
  }
  if (rec.username !== username) {
    return {
      ok: false,
      status: 403,
      error: "active challenge does not belong to this account",
    };
  }

  const normalizedTransfer = {
    to: _normalizeRecipient(transferInput),
    amount: sanitizeAmount(transferInput && transferInput.amount),
    token: _normalizeToken(transferInput),
    memo: _normalizeMemo(transferInput),
  };
  if (
    rec.transfer.to !== normalizedTransfer.to ||
    rec.transfer.amount !== normalizedTransfer.amount ||
    rec.transfer.token !== normalizedTransfer.token ||
    rec.transfer.memo !== normalizedTransfer.memo
  ) {
    return {
      ok: false,
      status: 400,
      error: "active challenge transfer mismatch",
    };
  }
  if (challenge !== rec.challenge) {
    return {
      ok: false,
      status: 400,
      error: "active challenge mismatch",
    };
  }

  try {
    const msg = crypto.createHash("sha256").update(challenge).digest();
    const valid = secp256k1.ecdsaVerify(
      Buffer.from(signature, "hex"),
      msg,
      Buffer.from(activePub, "hex"),
    );
    if (!valid) {
      return { ok: false, status: 401, error: "active key signature invalid" };
    }
  } catch (_) {
    return {
      ok: false,
      status: 400,
      error: "invalid active key signature format",
    };
  }

  rec.used = true;
  transferChallenges.delete(challengeId);

  return {
    ok: true,
    signer: "active",
    challengeId,
    challenge,
    publicKey: activePub,
    transfer: normalizedTransfer,
    expiresAt: new Date(rec.expiresAt).toISOString(),
  };
}

module.exports = {
  buildTransferChallenge,
  verifyActiveTransferSignature,
};
