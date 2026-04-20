"use strict";
const express = require('express');
const router = express.Router();
const { createWallet, getBalance, transfer, getTransactionHistory } = require('../controllers/walletController');
const { authenticateToken } = require('../middlewares/auth');
const { requireTOTP } = require('../services/totp');
const ledger = require('../services/ledger');
const stateStore = require('../chain/stateStore');

// All wallet routes require authentication
router.post('/create', authenticateToken, createWallet);
router.get('/balance', authenticateToken, getBalance);
router.post('/transfer', authenticateToken, requireTOTP, transfer);
router.get('/transactions', authenticateToken, getTransactionHistory);

// ── Nested wallet routes (v3.3) ─────────────────────────────────────────────

/**
 * POST /api/wallet/create-child
 * Body: { parent, child_name }
 * Auth: must be authenticated as the parent account (or parent's posting key holder).
 *
 * Creates account "parent/child_name" with parent as authority.
 */
router.post('/create-child', authenticateToken, async (req, res) => {
  try {
    const { parent, child_name } = req.body || {};

    if (!parent || !child_name) {
      return res.status(400).json({ error: 'parent and child_name are required' });
    }
    if (String(child_name).indexOf('/') !== -1) {
      return res.status(400).json({ error: 'child_name must not contain "/"' });
    }

    // Authorization: the authenticated user must be the parent account
    const callerUsername = req.user && req.user.username;
    if (!stateStore.isAuthorizedBy(parent, callerUsername)) {
      return res.status(403).json({ error: 'Not authorized to create children under ' + parent });
    }

    // Parent must exist
    if (!stateStore.hasAccount(parent)) {
      return res.status(404).json({ error: 'Parent account not found: ' + parent });
    }

    const epoch = stateStore.getChainHeight() || 0;
    const entry = await ledger.recordWalletCreateChild(parent, child_name, null, epoch);

    const childAccount = parent + '/' + child_name;
    return res.json({
      ok: true,
      child: childAccount,
      parent,
      epoch,
      entry_type: entry.type,
    });
  } catch (err) {
    return res.status(400).json({ error: err.message });
  }
});

/**
 * GET /api/wallet/:account/children
 * Public — returns list of child account names under a parent.
 */
router.get('/:account/children', (req, res) => {
  const account = req.params.account;
  if (!account) return res.status(400).json({ error: 'account required' });
  const children = stateStore.getChildren(account);
  return res.json({ account, children });
});

/**
 * GET /api/wallet/:account/parent
 * Public — returns parent account name, or null.
 */
router.get('/:account/parent', (req, res) => {
  const account = req.params.account;
  if (!account) return res.status(400).json({ error: 'account required' });
  const parent = stateStore.getParent(account);
  return res.json({ account, parent });
});

module.exports = router;
