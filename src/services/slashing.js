"use strict";

/**
 * Slashing Service — enforces penalties for protocol violations.
 *
 * Three roles get slashed differently:
 *
 * Miners:
 *   EMPTY_GARBAGE_INFERENCE  — 10% → 25% → deregistered
 *   TIMING_FRAUD             — 5% → 15% → 25%
 *   REPEATED_ZERO_QUALITY    — warning → 10% → deregistered
 *
 * Verifiers:
 *   RUBBER_STAMPING          — 5% → 15% → deregistered
 *   GRIEFING                 — 5% → 10% → 15%
 *   COLLUSION                — 25% → deregistered
 *
 * Clocks:
 *   TIME_DRIFT               — warning → 1% → 5%
 *   CLOCK_OFFLINE            — 0 rewards → 1% → deregistered
 *
 * All slashed tokens go to hone_recycle (NEVER burned).
 * Appeals within 100 epochs, adjudicated by fresh random verifier panel.
 *
 * Phase E: SlashRecord Mongoose model removed. Records live in stateStore.
 */

const stateStore = require('../chain/stateStore');
const User = require('../models/User');
const ledger = require('./ledger');

const RECYCLE_ACCOUNT = 'hone_recycle';
const APPEAL_WINDOW_EPOCHS = 100;

/**
 * Slash schedule per offense type.
 * Each tier is an array entry: { percent, deregister }
 * percent = fraction of staked amount to slash (0 = warning / no slash)
 * deregister = true means the node is removed from active set
 */
const SLASH_SCHEDULE = {
  // Miner offenses
  EMPTY_GARBAGE_INFERENCE: [
    { percent: 0.10, deregister: false },
    { percent: 0.25, deregister: false },
    { percent: 0,    deregister: true }
  ],
  TIMING_FRAUD: [
    { percent: 0.05, deregister: false },
    { percent: 0.15, deregister: false },
    { percent: 0.25, deregister: false }
  ],
  REPEATED_ZERO_QUALITY: [
    { percent: 0,    deregister: false }, // warning
    { percent: 0.10, deregister: false },
    { percent: 0,    deregister: true }
  ],

  // Verifier offenses
  RUBBER_STAMPING: [
    { percent: 0.05, deregister: false },
    { percent: 0.15, deregister: false },
    { percent: 0,    deregister: true }
  ],
  GRIEFING: [
    { percent: 0.05, deregister: false },
    { percent: 0.10, deregister: false },
    { percent: 0.15, deregister: false }
  ],
  COLLUSION: [
    { percent: 0.25, deregister: false },
    { percent: 0,    deregister: true }
  ],
  // Verifier committed but never revealed (network dropout is forgivable tier-1)
  VERIFIER_NO_REVEAL: [
    { percent: 0,    deregister: false }, // first: warning
    { percent: 0.02, deregister: false }, // second: 2%
    { percent: 0.05, deregister: false }  // chronic: 5%
  ],
  // Verifier revealed a hash that doesn't match their commitment
  VERIFIER_EQUIVOCATION: [
    { percent: 0.10, deregister: false },
    { percent: 0,    deregister: true }
  ],

  // Clock offenses
  TIME_DRIFT: [
    { percent: 0,    deregister: false }, // warning
    { percent: 0.01, deregister: false },
    { percent: 0.05, deregister: false }
  ],
  CLOCK_OFFLINE: [
    { percent: 0,    deregister: false }, // 0 rewards (handled externally)
    { percent: 0.01, deregister: false },
    { percent: 0,    deregister: true }
  ]
};

/**
 * Map offense types to their role.
 */
const OFFENSE_ROLE = {
  EMPTY_GARBAGE_INFERENCE: 'miner',
  TIMING_FRAUD: 'miner',
  REPEATED_ZERO_QUALITY: 'miner',
  RUBBER_STAMPING: 'verifier',
  GRIEFING: 'verifier',
  COLLUSION: 'verifier',
  VERIFIER_NO_REVEAL: 'verifier',
  VERIFIER_EQUIVOCATION: 'verifier',
  TIME_DRIFT: 'clock',
  CLOCK_OFFLINE: 'clock'
};

/**
 * Get offense history for an account, optionally filtered by offense type.
 * Reads from stateStore slash records.
 */
async function getOffenseHistory(account, offenseType) {
  const records = stateStore.getSlashRecords ? stateStore.getSlashRecords(account) : [];
  if (offenseType) return records.filter(r => r.offenseType === offenseType);
  return records;
}

/**
 * Count prior offenses of the same type for tier escalation.
 */
async function getPriorOffenseCount(account, offenseType) {
  const records = stateStore.getSlashRecords ? stateStore.getSlashRecords(account) : [];
  return records.filter(r => r.offenseType === offenseType).length;
}

/**
 * Calculate slash amount based on current tier and staked amount.
 *
 * @param {string} account — username
 * @param {string} offenseType — one of SLASH_SCHEDULE keys
 * @returns {{ tier, percent, amount, deregister, warning }}
 */
async function calculateSlash(account, offenseType) {
  const schedule = SLASH_SCHEDULE[offenseType];
  if (!schedule) throw new Error('Unknown offense type: ' + offenseType);

  const priorCount = await getPriorOffenseCount(account, offenseType);
  // Clamp to last tier if past the schedule length
  const tier = Math.min(priorCount, schedule.length - 1);
  const tierRule = schedule[tier];

  // Look up staked amount from stateStore
  const stakePool = stateStore.getStakePool ? stateStore.getStakePool(account) : null;
  const stakedAmount = stakePool ? (stakePool.total_staked || 0) : 0;

  const slashAmount = parseFloat((stakedAmount * tierRule.percent).toFixed(10));
  const isWarning = tierRule.percent === 0 && !tierRule.deregister;

  return {
    tier,
    percent: tierRule.percent,
    amount: slashAmount,
    deregister: tierRule.deregister,
    warning: isWarning,
    stakedAmount
  };
}

/**
 * Execute a slash — transfer tokens from the account's stake to hone_recycle.
 * Updates stateStore slash tracking.
 *
 * @param {string} account — username
 * @param {number} amount — HONE to slash
 * @param {string} reason — human-readable memo
 * @param {number} epoch — current epoch
 * @returns {object} ledger entry (or null if amount is 0)
 */
async function executeSlash(account, amount, reason, epoch) {
  if (amount <= 0) return null;

  // Record the slash transfer on the permanent ledger
  const entry = await ledger.recordTransfer(
    account, RECYCLE_ACCOUNT, amount, 'HONE', null, epoch,
    'slash: ' + reason
  );

  // Deduct from stateStore stake pool
  if (stateStore.deductStake) {
    stateStore.deductStake(account, amount);
  }

  return entry;
}

/**
 * Record an offense, escalate tier, and execute the slash.
 *
 * This is the main entry point — call this when a violation is detected.
 *
 * @param {string} account — username of the offender
 * @param {string} offenseType — one of SLASH_SCHEDULE keys
 * @param {object} evidence — arbitrary evidence object
 * @returns {object} slash record
 */
async function recordOffense(account, offenseType, evidence) {
  if (!OFFENSE_ROLE[offenseType]) {
    throw new Error('Unknown offense type: ' + offenseType);
  }

  const epoch = await ledger.getCurrentEpoch();
  const calc = await calculateSlash(account, offenseType);

  // Execute the actual slash (transfers tokens to hone_recycle)
  let slashTxId = null;
  if (calc.amount > 0) {
    const entry = await executeSlash(account, calc.amount, offenseType, epoch);
    if (entry) slashTxId = entry._id ? entry._id.toString() : null;
  }

  // Create the offense record and store in stateStore
  const record = {
    account,
    role: OFFENSE_ROLE[offenseType],
    offenseType,
    tier: calc.tier,
    amount: calc.amount,
    evidence: evidence || null,
    slashTxId,
    deregistered: calc.deregister,
    epoch,
    timestamp: new Date().toISOString(),
    appeal: {
      deadline: epoch + APPEAL_WINDOW_EPOCHS
    }
  };

  // Store in stateStore slash records
  if (stateStore.addSlashRecord) {
    stateStore.addSlashRecord(account, record);
  }

  console.log(
    '[SLASH] %s | %s | tier=%d | amount=%s HONE | deregister=%s',
    account, offenseType, calc.tier,
    calc.amount > 0 ? calc.amount.toFixed(8) : 'warning',
    calc.deregister
  );

  return record;
}

/**
 * Submit an appeal for a slash.
 * Must be within APPEAL_WINDOW_EPOCHS of the original offense.
 *
 * @param {string} account — the slashed account
 * @param {string} slashRecordId — index or id of the record
 * @returns {object} updated record
 */
async function submitAppeal(account, slashRecordId) {
  const records = stateStore.getSlashRecords ? stateStore.getSlashRecords(account) : [];
  const record = records.find(r => r.slashTxId === slashRecordId || r.id === slashRecordId);
  if (!record) throw new Error('Slash record not found');
  if (record.account !== account) throw new Error('Not your slash record');
  if (record.appeal && record.appeal.submitted) throw new Error('Appeal already submitted');
  if (record.appeal && record.appeal.resolved) throw new Error('Slash already resolved');

  const currentEpoch = await ledger.getCurrentEpoch();
  if (currentEpoch > record.appeal.deadline) {
    throw new Error('Appeal window expired (deadline was epoch ' + record.appeal.deadline + ')');
  }

  record.appeal.submitted = true;
  record.appeal.submittedAt = new Date().toISOString();
  record.appeal.submittedAtEpoch = currentEpoch;
  record.appeal.panelSize = 5;

  console.log('[SLASH] Appeal submitted: %s for offense %s', account, record.offenseType);
  return record;
}

/**
 * Resolve an appeal with panel verdicts.
 * 66% supermajority required to overturn.
 *
 * @param {string} slashRecordId — id of the record
 * @param {Array<{verifier: string, vote: string}>} panelVerdicts — each vote is 'overturn' or 'uphold'
 * @returns {object} updated record
 */
async function resolveAppeal(slashRecordId, panelVerdicts) {
  // Find record across all accounts
  let record = null;
  let recordAccount = null;
  if (stateStore.getAllSlashRecords) {
    const allRecords = stateStore.getAllSlashRecords();
    for (const [acct, recs] of Object.entries(allRecords)) {
      const found = recs.find(r => r.slashTxId === slashRecordId || r.id === slashRecordId);
      if (found) { record = found; recordAccount = acct; break; }
    }
  }
  if (!record) throw new Error('Slash record not found');
  if (!record.appeal || !record.appeal.submitted) throw new Error('No appeal submitted');
  if (record.appeal.resolved) throw new Error('Appeal already resolved');

  record.appeal.verdicts = panelVerdicts;

  const overturnVotes = panelVerdicts.filter(v => v.vote === 'overturn').length;
  const totalVotes = panelVerdicts.length;
  const supermajority = totalVotes * 2 / 3;

  if (overturnVotes >= supermajority) {
    // Overturn — refund the slashed amount
    record.appeal.outcome = 'overturned';

    if (record.amount > 0) {
      const epoch = await ledger.getCurrentEpoch();
      // Refund from hone_recycle back to the account
      await ledger.recordTransfer(
        RECYCLE_ACCOUNT, record.account, record.amount, 'HONE', null, epoch,
        'slash-refund: appeal overturned for ' + record.offenseType
      );

      // Restore stake in stateStore
      if (stateStore.restoreStake) {
        stateStore.restoreStake(record.account, record.amount);
      }
    }

    record.deregistered = false;
    console.log('[SLASH] Appeal OVERTURNED: %s for %s (%d/%d votes)',
      record.account, record.offenseType, overturnVotes, totalVotes);
  } else {
    record.appeal.outcome = 'upheld';
    console.log('[SLASH] Appeal UPHELD: %s for %s (%d/%d overturn votes, needed %s)',
      record.account, record.offenseType, overturnVotes, totalVotes, Math.ceil(supermajority));
  }

  record.appeal.resolved = true;
  return record;
}

module.exports = {
  SLASH_SCHEDULE,
  OFFENSE_ROLE,
  RECYCLE_ACCOUNT,
  APPEAL_WINDOW_EPOCHS,
  recordOffense,
  calculateSlash,
  executeSlash,
  submitAppeal,
  resolveAppeal,
  getOffenseHistory
};
