"use strict";

/**
 * BTCPC State Manager
 * Shin Devlin
 *
 * Manages the Sparse Merkle Tree for account state commitments.
 * Applies ledger entries to update account states in the SMT.
 * Generates and loads finality snapshots.
 *
 * The SMT tracks: { balance, staked, delegated, nonce } per account.
 * Balance changes come from ledger entries (transfers, rewards, staking, etc.).
 */

var SparseMerkleTree = require("./sparseMerkleTree");

// Live SMT instance — singleton
var smt = new SparseMerkleTree();

// In-memory account state cache: username → { balance, staked, delegated, nonce }
var accountStates = new Map();

/**
 * Get or create account state for a username.
 */
function _getState(username) {
  if (!username) return null;
  var state = accountStates.get(username);
  if (!state) {
    state = { balance: 0, staked: 0, delegated: 0, nonce: 0 };
    accountStates.set(username, state);
  }
  return state;
}

/**
 * Update account in the SMT after state change.
 */
function _updateSMT(username) {
  if (!username) return;
  // Skip system/virtual accounts
  if (username === 'btcpc_staking_pool' || username === 'btcpc_escrow' ||
      username === 'btcpc_genesis' || username === 'btcpc_mint' ||
      username.startsWith('project:') || username.startsWith('escrow:')) {
    return;
  }
  var state = _getState(username);
  smt.update(username, state);
}

/**
 * Apply a single ledger entry to the state.
 * Updates affected accounts' state and rehashes their SMT paths.
 */
function _applyEntry(entry) {
  var from = entry.from;
  var to = entry.to;
  var amount = entry.amount || 0;

  switch (entry.type) {
    case 'ACCOUNT_CREATE':
      // Just ensure the account exists in the SMT
      if (to) _updateSMT(to);
      break;

    case 'TRANSFER':
    case 'FAUCET':
    case 'MINING_REWARD':
      // Credit receiver
      if (to) {
        var toState = _getState(to);
        toState.balance = parseFloat((toState.balance + amount).toFixed(10));
        toState.nonce++;
        _updateSMT(to);
      }
      // Debit sender
      if (from) {
        var fromState = _getState(from);
        fromState.balance = parseFloat((fromState.balance - amount).toFixed(10));
        fromState.nonce++;
        _updateSMT(from);
      }
      break;

    case 'STAKE':
      // Move from balance to staked
      if (from) {
        var stakeFrom = _getState(from);
        stakeFrom.balance = parseFloat((stakeFrom.balance - amount).toFixed(10));
        stakeFrom.staked = parseFloat((stakeFrom.staked + amount).toFixed(10));
        stakeFrom.nonce++;
        _updateSMT(from);
      }
      break;

    case 'UNSTAKE':
      // Move from staked back to balance
      if (to) {
        var unstakeTo = _getState(to);
        unstakeTo.staked = parseFloat((unstakeTo.staked - amount).toFixed(10));
        unstakeTo.balance = parseFloat((unstakeTo.balance + amount).toFixed(10));
        unstakeTo.nonce++;
        _updateSMT(to);
      }
      break;

    case 'DELEGATE':
      // Move from balance to delegated
      if (from) {
        var delFrom = _getState(from);
        delFrom.balance = parseFloat((delFrom.balance - amount).toFixed(10));
        delFrom.delegated = parseFloat((delFrom.delegated + amount).toFixed(10));
        delFrom.nonce++;
        _updateSMT(from);
      }
      break;

    case 'UNDELEGATE':
      // Move from delegated back to balance
      if (to) {
        var undelTo = _getState(to);
        undelTo.delegated = parseFloat((undelTo.delegated - amount).toFixed(10));
        undelTo.balance = parseFloat((undelTo.balance + amount).toFixed(10));
        undelTo.nonce++;
        _updateSMT(to);
      }
      break;

    case 'ESCROW_LOCK':
      // Debit from payer
      if (from) {
        var escFrom = _getState(from);
        escFrom.balance = parseFloat((escFrom.balance - amount).toFixed(10));
        escFrom.nonce++;
        _updateSMT(from);
      }
      break;

    case 'ESCROW_RELEASE':
      // Credit to recipient
      if (to) {
        var escTo = _getState(to);
        escTo.balance = parseFloat((escTo.balance + amount).toFixed(10));
        escTo.nonce++;
        _updateSMT(to);
      }
      break;

    case 'ESCROW_REFUND':
      // Credit back to payer
      if (to) {
        var refTo = _getState(to);
        refTo.balance = parseFloat((refTo.balance + amount).toFixed(10));
        refTo.nonce++;
        _updateSMT(to);
      }
      break;

    case 'TOKEN_CREATE':
      // Token creation — nonce bump for creator
      if (from) {
        var tcFrom = _getState(from);
        tcFrom.nonce++;
        _updateSMT(from);
      }
      break;
  }
}

/**
 * Apply an array of ledger entries to the SMT.
 * Call this during block finalization with the epoch's entries.
 */
function applyLedgerEntries(entries) {
  for (var i = 0; i < entries.length; i++) {
    _applyEntry(entries[i]);
  }
}

/**
 * Get the current state root (SMT root hash).
 * Returns a 64-character hex string.
 */
function getStateRoot() {
  return smt.getRoot();
}

/**
 * Get an account's current state and SMT proof.
 * Returns { state, proof, root } or null if account not found.
 */
function getAccountState(username) {
  var state = accountStates.get(username);
  if (!state) return null;

  return {
    state: state,
    proof: smt.getProof(username),
    root: smt.getRoot()
  };
}

/**
 * Verify an account state against a root.
 */
function verifyAccountState(root, username, state, proof) {
  return SparseMerkleTree.verifyProof(root, username, state, proof);
}

/**
 * Generate a finality snapshot — full state for a finality block.
 * Returns a serializable object.
 */
function generateFinalitySnapshot() {
  var accounts = {};
  accountStates.forEach(function (state, username) {
    accounts[username] = {
      balance: state.balance,
      staked: state.staked,
      delegated: state.delegated,
      nonce: state.nonce
    };
  });

  return {
    accounts: accounts,
    smt: smt.serialize(),
    account_count: accountStates.size,
    state_root: smt.getRoot()
  };
}

/**
 * Load state from a finality block snapshot.
 * Replaces the current SMT and account states.
 */
function loadFromFinality(snapshot) {
  // Clear current state
  accountStates.clear();

  // Restore account states
  if (snapshot.accounts) {
    var usernames = Object.keys(snapshot.accounts);
    for (var i = 0; i < usernames.length; i++) {
      var username = usernames[i];
      accountStates.set(username, snapshot.accounts[username]);
    }
  }

  // Restore SMT
  if (snapshot.smt) {
    smt = SparseMerkleTree.deserialize(snapshot.smt);
  } else {
    // Rebuild SMT from account states
    smt = new SparseMerkleTree();
    accountStates.forEach(function (state, username) {
      smt.update(username, state);
    });
  }
}

/**
 * Reset all state (for testing or fresh start).
 */
function reset() {
  smt = new SparseMerkleTree();
  accountStates.clear();
}

/**
 * Get the number of tracked accounts.
 */
function getAccountCount() {
  return accountStates.size;
}

/**
 * Get all tracked usernames.
 */
function getAllUsernames() {
  return Array.from(accountStates.keys());
}

module.exports = {
  applyLedgerEntries: applyLedgerEntries,
  getStateRoot: getStateRoot,
  getAccountState: getAccountState,
  verifyAccountState: verifyAccountState,
  generateFinalitySnapshot: generateFinalitySnapshot,
  loadFromFinality: loadFromFinality,
  reset: reset,
  getAccountCount: getAccountCount,
  getAllUsernames: getAllUsernames
};
