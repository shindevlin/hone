"use strict";

/**
 * BTCPC Escrow Service
 * Shin Devlin
 *
 * Locks BTCPC during inference requests, releases on completion, refunds on expiry.
 *
 * Phase E: Escrow, Wallet, Transaction Mongoose models removed.
 * Escrow state lives in stateStore. Ledger entries are the canonical record.
 */

const stateStore = require('../chain/stateStore');
const User = require('../models/User');
const ledger = require('./ledger');

/**
 * Lock funds for an inference request.
 * Deducts maxFee from payer's stateStore balance, creates escrow entry.
 */
async function lockFunds(requestId, payerUsername, amount) {
  const balance = stateStore.getBalance(payerUsername, 'BTCPC');
  if (balance < amount) {
    throw new Error(`Insufficient balance: have ${balance}, need ${amount}`);
  }

  // Record on permanent ledger
  const epoch = await ledger.getCurrentEpoch();
  await ledger.recordEscrowLock(payerUsername, requestId, amount, epoch);

  return { request_id: requestId, payer: payerUsername, amount, status: 'locked' };
}

/**
 * Release escrowed funds to nodes after successful inference.
 * Distributes according to payout schedule.
 */
async function releaseFunds(requestId, payouts) {
  const escrow = stateStore.getEscrow ? stateStore.getEscrow(requestId) : null;
  if (!escrow) throw new Error('Escrow not found: ' + requestId);
  if (escrow.status !== 'locked') throw new Error('Escrow not locked: ' + escrow.status);

  const released = [];
  const epoch = await ledger.getCurrentEpoch();

  for (const payout of payouts) {
    const recipientUsername = payout.username || payout.node_id;
    if (!recipientUsername) continue;

    // Record on permanent ledger
    await ledger.recordEscrowRelease(recipientUsername, requestId, payout.amount, epoch, `Inference payout rank #${payout.rank}`);
    released.push({ username: recipientUsername, amount: payout.amount });
  }

  return { request_id: requestId, status: 'released', released_to: released };
}

/**
 * Refund escrowed funds to payer (timeout, no claims, etc).
 */
async function refundFunds(requestId) {
  const escrow = stateStore.getEscrow ? stateStore.getEscrow(requestId) : null;
  if (!escrow) throw new Error('Escrow not found: ' + requestId);
  if (escrow.status !== 'locked') throw new Error('Escrow not locked: ' + escrow.status);

  // Record on permanent ledger
  const epoch = await ledger.getCurrentEpoch();
  await ledger.recordEscrowRefund(escrow.payer, requestId, escrow.amount, epoch);

  return { request_id: requestId, status: 'refunded' };
}

/**
 * Sweep stale escrows — auto-refund any that are locked longer than maxAge.
 * Should run periodically (e.g. every epoch or every 5 minutes).
 */
async function sweepEscrows(maxAgeMs) {
  maxAgeMs = maxAgeMs || 600000; // 10 minutes default
  const cutoff = Date.now() - maxAgeMs;

  // Get all locked escrows from stateStore
  const allEscrows = stateStore.getAllEscrows ? stateStore.getAllEscrows() : {};
  const stale = Object.values(allEscrows).filter(e => {
    if (e.status !== 'locked') return false;
    const lockedAt = e.locked_at ? new Date(e.locked_at).getTime() : 0;
    return lockedAt > 0 && lockedAt < cutoff;
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
 * Pays the miner who completed the work.
 * If the project has a revenue split configured, distributes the miner
 * payout according to those splits instead of paying the miner directly.
 */
async function releaseForJob(requestId, minerUsername, amount, model, projectName) {
  const escrow = stateStore.getEscrow ? stateStore.getEscrow(requestId) : null;
  if (!escrow) return null; // no escrow for this job (pre-escrow era)
  if (escrow.status !== 'locked') return null; // already released/refunded

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

  // Check for project revenue split configuration
  const splitConfig = projectName && stateStore.getProjectRevenueSplit
    ? stateStore.getProjectRevenueSplit(projectName)
    : null;

  if (splitConfig && splitConfig.splits && splitConfig.splits.length > 0 && minerPayout > 0) {
    // Distribute the miner payout according to the project's revenue splits
    const splitPayouts = [];
    for (const split of splitConfig.splits) {
      const splitAmount = parseFloat((minerPayout * split.percent / 100).toFixed(10));
      if (splitAmount > 0.000001) {
        await ledger.recordMiningReward(split.account, splitAmount, epoch, null, null, 'inference_split');
        splitPayouts.push({ account: split.account, amount: splitAmount, percent: split.percent });
        console.log(`[BTCPC Escrow] Revenue split: ${split.account} earned ${splitAmount.toFixed(6)} BTCPC (${split.percent}% of ${projectName})`);
      }
    }
    // Release escrow with zero to miner (funds go to split accounts via MINING_REWARD)
    await ledger.recordEscrowRelease(minerUsername, requestId, 0, epoch, 'Inference settlement (revenue split applied)');
  } else {
    await ledger.recordEscrowRelease(minerUsername, requestId, minerPayout, epoch, 'Inference settlement');
    await ledger.updateWalletCache(minerUsername, 'BTCPC', minerPayout);
  }

  // Refund overpayment if escrow > actual cost
  const overpayment = escrow.amount - amount;
  if (overpayment > 0.000001) {
    await ledger.recordEscrowRefund(escrow.payer, requestId, overpayment, epoch);
    await ledger.updateWalletCache(escrow.payer, 'BTCPC', overpayment);
  }

  return { request_id: requestId, status: 'released', released_to: [{ username: minerUsername, amount }] };
}

module.exports = { lockFunds, releaseFunds, refundFunds, sweepEscrows, releaseForJob };
