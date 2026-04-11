"use strict";

const crypto = require('crypto');
const stateStore = require('../chain/stateStore');

/**
 * Compute a Merkle-like state hash for the current epoch.
 * Combines all account balances, active stakes, and work proofs
 * into a single deterministic hash representing the full network state.
 * Phase E: reads exclusively from stateStore (no Mongo).
 */
async function computeStateHash(epochNumber, previousStateHash) {
  const hasher = crypto.createHash('sha256');

  // Include previous state hash for chaining
  hasher.update(previousStateHash || '0'.repeat(64));
  hasher.update(`epoch:${epochNumber}`);

  // Hash all account balances from stateStore (sorted by username for determinism)
  const accounts = stateStore.getAllAccounts();
  const sortedAccounts = Object.keys(accounts).sort();
  for (const username of sortedAccounts) {
    const acct = accounts[username];
    if (!acct || !acct.balances) continue;
    const balanceEntries = Object.entries(acct.balances).map(([token, amount]) => `${token}:${amount}`).sort();
    hasher.update(`wallet:${username}|${balanceEntries.join(',')}`);
  }

  // Hash all active stakes from stateStore (sorted by username)
  const stakePools = stateStore.getAllStakePools();
  const sortedStakers = Object.keys(stakePools).sort();
  for (const username of sortedStakers) {
    const stake = stakePools[username];
    if (!stake || stake.status !== 'active') continue;
    hasher.update(`stake:${username}|${stake.staked_amount}`);
  }

  // Hash work proofs for this epoch from stateStore (sorted by prompt_hash)
  const proofs = stateStore.getComputeProofs(epochNumber).slice().sort((a, b) => {
    return (a.prompt_hash || '').localeCompare(b.prompt_hash || '');
  });
  for (const p of proofs) {
    hasher.update(`proof:${p.prompt_hash}|${p.result_hash}|${p.tokens_generated}|${p.model}`);
  }

  return hasher.digest('hex');
}

module.exports = { computeStateHash };
