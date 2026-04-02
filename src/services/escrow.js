"use strict";

/**
 * BTCPC Escrow Service
 * Shin Devlin
 *
 * Locks BTCPC during inference requests, releases on completion, refunds on expiry.
 */

const Escrow = require("../models/Escrow");
const Wallet = require("../models/Wallet");
const User = require("../models/User");
const Transaction = require("../models/Transaction");
const ledger = require("./ledger");

/**
 * Lock funds for an inference request.
 * Deducts maxFee from payer's wallet, creates Escrow record.
 */
async function lockFunds(requestId, payerUsername, amount) {
  const user = await User.findOne({ username: payerUsername });
  if (!user) throw new Error("Payer not found: " + payerUsername);

  const wallet = await Wallet.findOne({ userId: user._id, chain: "btcpc" });
  if (!wallet) throw new Error("No BTCPC wallet for " + payerUsername);

  const balance = wallet.balance.get("BTCPC") || 0;
  if (balance < amount) {
    throw new Error(`Insufficient balance: have ${balance}, need ${amount}`);
  }

  // Record on permanent ledger
  const epoch = await ledger.getCurrentEpoch();
  await ledger.recordEscrowLock(payerUsername, requestId, amount, epoch);

  // Update wallet cache
  wallet.balance.set("BTCPC", balance - amount);
  await wallet.save();

  // Create escrow
  const escrow = new Escrow({
    request_id: requestId,
    payer: payerUsername,
    amount,
    status: "locked",
  });
  await escrow.save();

  // Record transaction (legacy index)
  const tx = new Transaction({
    from: payerUsername,
    to: "escrow:" + requestId,
    amount,
    type: "escrow_lock",
    memo: "Inference request escrow",
  });
  await tx.save();

  return escrow;
}

/**
 * Release escrowed funds to nodes after successful inference.
 * Distributes according to payout schedule.
 */
async function releaseFunds(requestId, payouts) {
  const escrow = await Escrow.findOne({ request_id: requestId });
  if (!escrow) throw new Error("Escrow not found: " + requestId);
  if (escrow.status !== "locked") throw new Error("Escrow not locked: " + escrow.status);

  const released = [];

  const epoch = await ledger.getCurrentEpoch();

  for (const payout of payouts) {
    const node = await require("../models/Node").findById(payout.node_id);
    if (!node) continue;
    const user = await User.findById(node.account);
    if (!user) continue;
    const wallet = await Wallet.findOne({ userId: user._id, chain: "btcpc" });
    if (!wallet) continue;

    // Record on permanent ledger
    await ledger.recordEscrowRelease(user.username, requestId, payout.amount, epoch, `Inference payout rank #${payout.rank}`);

    // Update wallet cache
    wallet.balance.set("BTCPC", (wallet.balance.get("BTCPC") || 0) + payout.amount);
    await wallet.save();

    released.push({ node_id: payout.node_id, amount: payout.amount });

    // Record transaction (legacy index)
    const tx = new Transaction({
      from: "escrow:" + requestId,
      to: user.username,
      amount: payout.amount,
      type: "escrow_release",
      memo: `Inference payout rank #${payout.rank}`,
    });
    await tx.save();
  }

  escrow.status = "released";
  escrow.released_at = new Date();
  escrow.released_to = released;
  await escrow.save();

  return escrow;
}

/**
 * Refund escrowed funds to payer (timeout, no claims, etc).
 */
async function refundFunds(requestId) {
  const escrow = await Escrow.findOne({ request_id: requestId });
  if (!escrow) throw new Error("Escrow not found: " + requestId);
  if (escrow.status !== "locked") throw new Error("Escrow not locked: " + escrow.status);

  const user = await User.findOne({ username: escrow.payer });
  if (!user) throw new Error("Payer not found");
  const wallet = await Wallet.findOne({ userId: user._id, chain: "btcpc" });
  if (!wallet) throw new Error("Wallet not found");

  // Record on permanent ledger
  const epoch = await ledger.getCurrentEpoch();
  await ledger.recordEscrowRefund(escrow.payer, requestId, escrow.amount, epoch);

  // Update wallet cache
  wallet.balance.set("BTCPC", (wallet.balance.get("BTCPC") || 0) + escrow.amount);
  await wallet.save();

  escrow.status = "refunded";
  escrow.released_at = new Date();
  await escrow.save();

  // Record transaction (legacy index)
  const tx = new Transaction({
    from: "escrow:" + requestId,
    to: escrow.payer,
    amount: escrow.amount,
    type: "escrow_refund",
    memo: "Inference request expired/cancelled",
  });
  await tx.save();

  return escrow;
}

module.exports = { lockFunds, releaseFunds, refundFunds };
