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

/**
 * Sweep stale escrows — auto-refund any that are locked longer than maxAge.
 * Should run periodically (e.g. every epoch or every 5 minutes).
 * This prevents BTCPC from getting permanently stuck in escrow.
 */
async function sweepEscrows(maxAgeMs) {
  maxAgeMs = maxAgeMs || 600000; // 10 minutes default
  const cutoff = new Date(Date.now() - maxAgeMs);

  // Use _id timestamp (ObjectId embeds creation time) as fallback
  const mongoose = require("mongoose");
  const cutoffId = mongoose.Types.ObjectId.createFromTime(Math.floor(cutoff.getTime() / 1000));
  const stale = await Escrow.find({
    status: "locked",
    _id: { $lt: cutoffId }
  });

  let refunded = 0;
  let totalRefunded = 0;

  for (const escrow of stale) {
    try {
      await refundFunds(escrow.request_id);
      refunded++;
      totalRefunded += escrow.amount;
    } catch (err) {
      // Skip individual failures — don't crash the sweep
    }
  }

  if (refunded > 0) {
    console.log(`[BTCPC Escrow] Swept ${refunded} stale escrows, refunded ${totalRefunded.toFixed(4)} BTCPC`);
  }

  return { refunded, totalRefunded };
}

/**
 * Release escrow for a settled inference job by job_id.
 * Pays the miner who completed the work. Simpler than releaseFunds
 * which expects node_id payouts — this works with username directly.
 */
async function releaseForJob(requestId, minerUsername, amount, model) {
  const escrow = await Escrow.findOne({ request_id: requestId });
  if (!escrow) return null; // no escrow for this job (pre-escrow era)
  if (escrow.status !== "locked") return null; // already released/refunded

  const epoch = await ledger.getCurrentEpoch();

  // Revenue share — model creators earn a cut of inference fees
  let revSharePaid = 0;
  if (model && amount > 0) {
    try {
      const payouts = await ledger.distributeRevenueShare(model, amount, epoch);
      for (const p of payouts) {
        revSharePaid += p.amount;
        console.log(`[BTCPC Escrow] Rev share: ${p.to} earned ${p.amount.toFixed(6)} BTCPC (${p.percent}% of ${model})`);
      }
    } catch (_) {}
  }

  // Pay the miner (minus revenue share)
  const minerPayout = parseFloat((amount - revSharePaid).toFixed(10));
  await ledger.recordEscrowRelease(minerUsername, requestId, minerPayout, epoch, "Inference settlement");
  await ledger.updateWalletCache(minerUsername, "BTCPC", minerPayout);

  // Refund overpayment if escrow > actual cost
  const overpayment = escrow.amount - amount;
  if (overpayment > 0.000001) {
    await ledger.recordEscrowRefund(escrow.payer, requestId, overpayment, epoch);
    await ledger.updateWalletCache(escrow.payer, "BTCPC", overpayment);
  }

  escrow.status = "released";
  escrow.released_at = new Date();
  escrow.released_to = [{ username: minerUsername, amount }];
  await escrow.save();

  return escrow;
}

module.exports = { lockFunds, releaseFunds, refundFunds, sweepEscrows, releaseForJob };
