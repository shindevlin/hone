"use strict";

const crypto = require('crypto');
const Wallet = require('../models/Wallet');
const StakingPool = require('../models/StakingPool');
const WorkProof = require('../models/WorkProof');

/**
 * Compute a Merkle-like state hash for the current epoch.
 * Combines all account balances, active stakes, and work proofs
 * into a single deterministic hash representing the full network state.
 */
async function computeStateHash(epochNumber, previousStateHash) {
  const hasher = crypto.createHash('sha256');

  // Include previous state hash for chaining
  hasher.update(previousStateHash || '0'.repeat(64));
  hasher.update(`epoch:${epochNumber}`);

  // Hash all wallet balances (sorted by address for determinism)
  const wallets = await Wallet.find({}).sort({ address: 1 }).lean();
  for (const w of wallets) {
    const balanceEntries = [];
    if (w.balance) {
      // balance is stored as a Map; lean() returns a plain object
      const balanceObj = w.balance instanceof Map ? Object.fromEntries(w.balance) : w.balance;
      for (const [token, amount] of Object.entries(balanceObj)) {
        balanceEntries.push(`${token}:${amount}`);
      }
    }
    hasher.update(`wallet:${w.address}|${balanceEntries.sort().join(',')}`);
  }

  // Hash all active stakes (sorted by wallet_address)
  const stakes = await StakingPool.find({ status: 'active' }).sort({ wallet_address: 1 }).lean();
  for (const s of stakes) {
    hasher.update(`stake:${s.wallet_address}|${s.staked_amount}`);
  }

  // Hash work proofs for this epoch (sorted by prompt_hash)
  const proofs = await WorkProof.find({ epoch_number: epochNumber }).sort({ prompt_hash: 1 }).lean();
  for (const p of proofs) {
    hasher.update(`proof:${p.prompt_hash}|${p.result_hash}|${p.tokens_generated}|${p.model}`);
  }

  return hasher.digest('hex');
}

module.exports = { computeStateHash };
