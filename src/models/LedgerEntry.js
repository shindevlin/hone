"use strict";
var mongoose = require("mongoose");
var Schema = mongoose.Schema;

/**
 * LedgerEntry — permanent on-chain record.
 *
 * The ledger NEVER prunes. Every entry from genesis to present is stored.
 * Balance = sum(incoming) - sum(outgoing) for any account at any time.
 *
 * Types: ACCOUNT_CREATE, TRANSFER, TOKEN_CREATE, STAKE, DELEGATE,
 *        UNDELEGATE, MINING_REWARD, FAUCET
 */
var ledgerEntrySchema = new Schema({
  type: {
    type: String,
    enum: [
      'ACCOUNT_CREATE', 'TRANSFER', 'TOKEN_CREATE',
      'STAKE', 'DELEGATE', 'UNDELEGATE',
      'MINING_REWARD', 'FAUCET'
    ],
    required: true,
    index: true
  },

  // Who initiated this entry
  from: { type: String, default: null },
  to: { type: String, default: null },

  // Token and amount
  token: { type: String, default: 'BTCPC' },
  amount: { type: Number, default: 0 },

  // Epoch this was included in
  epoch: { type: Number, required: true, index: true },

  // Signature (active key for transfers, posting key for lightweight ops)
  signature: { type: String, default: null },
  signed_by: { type: String, enum: ['owner', 'active', 'posting', null], default: null },

  // Account creation data (type: ACCOUNT_CREATE)
  account_data: {
    username: { type: String, default: null },
    public_keys: {
      owner: { type: String, default: null },
      active: { type: String, default: null },
      posting: { type: String, default: null },
      memo: { type: String, default: null }
    },
    chain_addresses: {
      btcpc: { type: String, default: null },
      evm: { type: String, default: null },
      solana: { type: String, default: null },
      bitcoin: { type: String, default: null },
      ton: { type: String, default: null },
      hive: { type: String, default: null }
    }
  },

  // Token creation data (type: TOKEN_CREATE)
  token_data: {
    name: { type: String, default: null },
    symbol: { type: String, default: null },
    supply: { type: Number, default: null },
    decimals: { type: Number, default: null }
  },

  // Delegation data (type: DELEGATE/UNDELEGATE)
  delegation_data: {
    purpose: { type: String, default: null } // mining, clock_node, etc.
  },

  // Metadata
  memo: { type: String, default: null },
  timestamp: { type: Date, default: Date.now }
});

// Indexes for fast balance computation
ledgerEntrySchema.index({ to: 1, token: 1, epoch: 1 });
ledgerEntrySchema.index({ from: 1, token: 1, epoch: 1 });
ledgerEntrySchema.index({ 'account_data.username': 1 });

module.exports = mongoose.model("LedgerEntry", ledgerEntrySchema);
