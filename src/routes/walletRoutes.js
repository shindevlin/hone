"use strict";
const express = require('express');
const router = express.Router();
const { createWallet, getBalance, transfer, getTransactionHistory } = require('../controllers/walletController');
const { authenticateToken } = require('../middlewares/auth');
const { requireTOTP } = require('../services/totp');
const ledger = require('../services/ledger');
const stateStore = require('../chain/stateStore');
const privateAuthorization = require('../services/privateAuthorization');

// All wallet routes require authentication
router.post('/create', authenticateToken, createWallet);
router.get('/balance', authenticateToken, getBalance);
router.post('/transfer', authenticateToken, requireTOTP, transfer);
router.get('/transactions', authenticateToken, getTransactionHistory);

router.get('/private-auth', authenticateToken, async (req, res) => {
  try {
    const user = req.user && req.user.username ? req.user.username : null;
    if (!user) return res.status(401).json({ error: 'unauthenticated' });
    const policy = await privateAuthorization.getPolicy(user);
    res.json({ success: true, policy });
  } catch (err) {
    res.status(400).json({ error: err.message });
  }
});

router.post('/private-auth/policy', authenticateToken, async (req, res) => {
  try {
    const user = req.user && req.user.username ? req.user.username : null;
    if (!user) return res.status(401).json({ error: 'unauthenticated' });
    const threshold = req.body && req.body.threshold;
    const enabled = req.body && req.body.enabled;
    const policy = await privateAuthorization.setPolicy(user, { threshold, enabled });
    res.json({ success: true, policy });
  } catch (err) {
    res.status(400).json({ error: err.message });
  }
});

router.post('/private-auth/enroll/request', authenticateToken, async (req, res) => {
  try {
    const user = req.user && req.user.username ? req.user.username : null;
    if (!user) return res.status(401).json({ error: 'unauthenticated' });
    const chain = req.body && req.body.chain;
    const label = req.body && req.body.label;
    const address = req.body && req.body.address;
    const challenge = await privateAuthorization.requestEnrollment(user, chain, label, address);
    res.json({ success: true, challenge });
  } catch (err) {
    res.status(400).json({ error: err.message });
  }
});

router.post('/private-auth/enroll/verify', authenticateToken, async (req, res) => {
  try {
    const challengeId = req.body && req.body.challengeId;
    const signature = req.body && req.body.signature;
    if (!challengeId || !signature) {
      return res.status(400).json({ error: 'challengeId and signature required' });
    }
    const result = await privateAuthorization.verifyEnrollment(challengeId, signature);
    if (!result.success) return res.status(400).json({ error: result.error });
    res.json({ success: true, result });
  } catch (err) {
    res.status(400).json({ error: err.message });
  }
});

router.post('/private-auth/transfer/request', authenticateToken, async (req, res) => {
  try {
    const user = req.user && req.user.username ? req.user.username : null;
    if (!user) return res.status(401).json({ error: 'unauthenticated' });
    const toAddress = req.body && req.body.toAddress;
    const amount = req.body && req.body.amount;
    const token = req.body && req.body.token ? req.body.token : 'HONE';
    const memo = req.body && req.body.memo ? req.body.memo : null;
    const approvalChain = req.body && req.body.approval_chain ? req.body.approval_chain : req.body.chain;
    const proofBackend = req.body && req.body.proof_backend ? req.body.proof_backend : null;
    const challenge = await privateAuthorization.requestTransferAuthorization(user, {
      from: user,
      to: toAddress,
      amount,
      token,
      memo,
      approval_chain: approvalChain,
      proof_backend: proofBackend
    });
    res.json({ success: true, challenge });
  } catch (err) {
    res.status(400).json({ error: err.message });
  }
});

// ── Nested wallet routes (v3.3) ─────────────────────────────────────────────

/**
 * POST /api/wallet/create-nested
 * Body: { parent, nested_name } or legacy { parent, child_name }
 * Auth: must be authenticated as the parent account (or parent's posting key holder).
 *
 * Creates nested wallet "parent/nested_name" with parent as namespace authority.
 */
async function createNestedWallet(req, res) {
  try {
    const body = req.body || {};
    const parent = body.parent;
    const nestedName = body.nested_name || body.child_name;

    if (!parent || !nestedName) {
      return res.status(400).json({ error: 'parent and nested_name are required' });
    }
    if (String(nestedName).indexOf('/') !== -1) {
      return res.status(400).json({ error: 'nested_name must not contain "/"' });
    }

    // Authorization: the authenticated user must be the parent account
    const callerUsername = req.user && req.user.username;
    if (!stateStore.isAuthorizedBy(parent, callerUsername)) {
      return res.status(403).json({ error: 'Not authorized to create nested wallets under ' + parent });
    }

    // Parent must exist
    if (!stateStore.hasAccount(parent)) {
      return res.status(404).json({ error: 'Parent account not found: ' + parent });
    }

    const epoch = stateStore.getChainHeight() || 0;
    const entry = await ledger.recordWalletCreateChild(parent, nestedName, null, epoch);

    const nestedWallet = parent + '/' + nestedName;
    return res.json({
      ok: true,
      nested_wallet: nestedWallet,
      child: nestedWallet,
      parent,
      epoch,
      entry_type: entry.type,
    });
  } catch (err) {
    return res.status(400).json({ error: err.message });
  }
}

router.post('/create-nested', authenticateToken, createNestedWallet);
router.post('/create-child', authenticateToken, createNestedWallet);

/**
 * POST /api/wallet/unnest
 * Body: { parent, nested_name, native_name?, public_keys, chain_addresses? }
 *
 * Promotes "parent/nested_name" into native "native_name" using the
 * recipient's public keys. Private keys and mnemonic never come to the server.
 */
router.post('/unnest', authenticateToken, async (req, res) => {
  try {
    const body = req.body || {};
    const parent = body.parent;
    const nestedName = body.nested_name || body.child_name;
    const nativeName = body.native_name || nestedName;
    const publicKeys = body.public_keys || {};
    const chainAddresses = body.chain_addresses || {};

    if (!parent || !nestedName) {
      return res.status(400).json({ error: 'parent and nested_name are required' });
    }
    if (!publicKeys.owner || !publicKeys.active || !publicKeys.posting) {
      return res.status(400).json({ error: 'recipient owner, active, and posting public keys are required' });
    }

    const callerUsername = req.user && req.user.username;
    if (!stateStore.isAuthorizedBy(parent, callerUsername)) {
      return res.status(403).json({ error: 'Not authorized to un-nest wallets under ' + parent });
    }

    const epoch = stateStore.getChainHeight() || 0;
    const entry = await ledger.recordWalletUnnest(
      parent,
      nestedName,
      nativeName,
      publicKeys,
      chainAddresses,
      null,
      epoch
    );

    return res.json({
      ok: true,
      nested_wallet: parent + '/' + nestedName,
      native_account: nativeName,
      parent,
      epoch,
      entry_type: entry.type,
    });
  } catch (err) {
    return res.status(400).json({ error: err.message });
  }
});

/**
 * POST /api/wallet/assign-alias
 * Body: { parent, nested_name, target_account }
 *
 * Assigns a reserved nested wallet name as a public alias of an existing
 * canonical account.
 */
router.post('/assign-alias', authenticateToken, async (req, res) => {
  try {
    const body = req.body || {};
    const parent = body.parent;
    const nestedName = body.nested_name || body.child_name;
    const targetAccount = body.target_account;
    if (!parent || !nestedName || !targetAccount) {
      return res.status(400).json({ error: 'parent, nested_name, and target_account are required' });
    }

    const callerUsername = req.user && req.user.username;
    if (!stateStore.isAuthorizedBy(parent, callerUsername)) {
      return res.status(403).json({ error: 'Not authorized to assign aliases under ' + parent });
    }

    const epoch = stateStore.getChainHeight() || 0;
    const entry = await ledger.recordAccountAliasAssign(parent, nestedName, targetAccount, null, epoch);
    return res.json({
      ok: true,
      alias: nestedName,
      target_account: targetAccount,
      nested_wallet: parent + '/' + nestedName,
      transferable: true,
      entry_type: entry.type,
    });
  } catch (err) {
    return res.status(400).json({ error: err.message });
  }
});

/**
 * POST /api/wallet/transfer-alias
 * Body: { alias, to_account }
 *
 * Transfers an assigned alias from the authenticated canonical owner to another
 * existing account. Production clients should protect this with an owner-key
 * signature before exposing it broadly.
 */
router.post('/transfer-alias', authenticateToken, async (req, res) => {
  try {
    const body = req.body || {};
    const alias = body.alias;
    const toAccount = body.to_account;
    const fromAccount = req.user && req.user.username;
    if (!alias || !toAccount) {
      return res.status(400).json({ error: 'alias and to_account are required' });
    }
    if (stateStore.getAliasTarget(alias) !== fromAccount) {
      return res.status(403).json({ error: 'Alias is not assigned to authenticated account' });
    }

    const epoch = stateStore.getChainHeight() || 0;
    const entry = await ledger.recordAccountAliasTransfer(alias, fromAccount, toAccount, null, epoch);
    return res.json({
      ok: true,
      alias,
      from_account: fromAccount,
      to_account: toAccount,
      entry_type: entry.type,
    });
  } catch (err) {
    return res.status(400).json({ error: err.message });
  }
});

/**
 * GET /api/wallet/:account/nested
 * Public — returns nested wallet account names under a parent.
 */
function listNestedWallets(req, res) {
  const account = req.params.account;
  if (!account) return res.status(400).json({ error: 'account required' });
  const nestedWallets = stateStore.getChildren(account);
  return res.json({ account, nested_wallets: nestedWallets, children: nestedWallets });
}

router.get('/:account/nested', listNestedWallets);
router.get('/:account/children', listNestedWallets);

router.get('/:account/aliases', (req, res) => {
  const account = req.params.account;
  const canonical = stateStore.resolveAccountName(account) || account;
  return res.json({ account, canonical_account: canonical, aliases: stateStore.getAliases(canonical) });
});

router.get('/alias/:alias', (req, res) => {
  const alias = req.params.alias;
  return res.json({ alias, target_account: stateStore.getAliasTarget(alias) });
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

// ── Cross-chain identity link (v3.1.114) ──────────────────────────────────────

/**
 * POST /api/wallet/identity-link
 * Body: { chain_addresses: { eth, solana, ton, bitcoin }, signature }
 *
 * Creates a permanent on-chain attestation that this HONE account owns the
 * listed addresses on external chains. Visible on honescan under the account.
 * Requires owner key signature over canonical JSON of { account, chain_addresses }.
 */
router.post('/identity-link', authenticateToken, async (req, res) => {
  try {
    const account = req.user.username;
    const { chain_addresses, signature } = req.body || {};

    if (!chain_addresses || typeof chain_addresses !== 'object') {
      return res.status(400).json({ error: 'chain_addresses required (e.g. { eth: "0x...", solana: "5..." })' });
    }
    if (!signature) {
      return res.status(403).json({ error: 'owner key signature required' });
    }

    const epoch = stateStore.getChainHeight ? stateStore.getChainHeight() : 0;
    const entry = await ledger.recordIdentityLink(account, chain_addresses, signature, epoch);

    return res.json({
      success: true,
      account,
      chain_addresses,
      epoch,
      entry_type: entry.type,
    });
  } catch (err) {
    return res.status(400).json({ error: err.message });
  }
});

/**
 * GET /api/wallet/:account/identity
 * Public — returns the linked chain addresses for an account.
 */
router.get('/:account/identity', (req, res) => {
  const account = (req.params.account || '').toLowerCase().trim();
  if (!account) return res.status(400).json({ error: 'account required' });
  const acct = stateStore.getAccount ? stateStore.getAccount(account) : null;
  if (!acct) return res.status(404).json({ error: 'account not found' });
  return res.json({
    account,
    chain_addresses: acct.chain_addresses || {},
    identity_linked_epoch: acct.identity_linked_epoch || null,
  });
});

// ── Cross-chain credits (v3.1.114) ────────────────────────────────────────────

/**
 * GET /api/wallet/:account/cross-chain-credits
 * Public — returns unclaimed wHONE credits per chain for an account.
 */
router.get('/:account/cross-chain-credits', (req, res) => {
  const account = (req.params.account || '').toLowerCase().trim();
  if (!account) return res.status(400).json({ error: 'account required' });
  const acct = stateStore.getAccount ? stateStore.getAccount(account) : null;
  if (!acct) return res.status(404).json({ error: 'account not found' });
  const credits = stateStore.getCrossChainCredits(account);
  const chain_addresses = acct.chain_addresses || {};
  const chains = Object.entries(credits).map(([chain, amount]) => ({
    chain,
    unclaimed_hone: Math.round(amount * 1e8) / 1e8,
    claim_address: chain_addresses[chain] || null,
    can_claim: !!chain_addresses[chain] && amount > 0,
  }));
  return res.json({ account, chains });
});

/**
 * POST /api/wallet/claim-cross-chain
 * Body: { chain, amount?, signature }
 * Auth: required — generates a signed claim message for the wHONE contract on the target chain.
 */
router.post('/claim-cross-chain', authenticateToken, async (req, res) => {
  try {
    const account = req.user.username;
    const { chain, amount, signature } = req.body || {};
    const supported = stateStore.CROSS_CHAIN_SUPPORTED || [];
    if (!chain) return res.status(400).json({ error: 'chain required', supported_chains: supported });
    if (!supported.includes(chain)) return res.status(400).json({ error: 'unsupported chain: ' + chain, supported_chains: supported });
    if (!signature) return res.status(403).json({ error: 'posting key signature required' });

    const acct = stateStore.getAccount ? stateStore.getAccount(account) : null;
    if (!acct) return res.status(404).json({ error: 'account not found' });

    const claimAddress = (acct.chain_addresses || {})[chain];
    if (!claimAddress) {
      return res.status(400).json({
        error: `No ${chain} address linked. POST /api/wallet/identity-link first.`,
      });
    }

    const credits = stateStore.getCrossChainCredits(account);
    const available = credits[chain] || 0;
    if (available <= 0) return res.status(400).json({ error: 'no claimable credits on ' + chain });

    const claimAmount = amount ? Math.min(parseFloat(amount), available) : available;
    if (claimAmount <= 0) return res.status(400).json({ error: 'amount must be > 0' });

    const epoch = stateStore.getChainHeight ? stateStore.getChainHeight() : 0;
    const crossChainFinality = require('../services/crossChainFinalityConsumer');
    const finalityCutoff = crossChainFinality.getFinalityCutoff(chain);
    if (finalityCutoff === null || epoch > finalityCutoff) {
      return res.status(409).json({
        error: 'Current epoch ' + epoch + ' has not been finalized for ' + chain,
        current_epoch: epoch,
        finality_cutoff: finalityCutoff
      });
    }
    await ledger.recordCrossChainClaim(account, chain, claimAmount, claimAddress, signature, epoch);

    // The signed claim payload — the wHONE oracle verifies this and mints on the target chain
    const crypto = require('crypto');
    const nonce = crypto.randomBytes(32).toString('hex');

    // Generate EVM-compatible proof if oracle key is configured and chain is EVM
    let claimProof = null;
    const oracleKey = process.env.HONE_ORACLE_PRIVKEY;
    const evmChains = ['ethereum', 'base', 'arbitrum', 'optimism', 'bsc', 'polygon'];
    if (oracleKey && evmChains.includes(chain)) {
      try {
        const { generateClaimProof } = require('../claims/claimProofGenerator');
        claimProof = generateClaimProof({
          honeAccount: account,
          chain,
          targetWallet: claimAddress,
          amount: claimAmount,
          nonce,
          epoch,
          oraclePrivKey: oracleKey,
        });
      } catch (e) {
        console.error('[HONE] claim proof generation failed:', e.message);
      }
    }

    return res.json({
      success: true,
      account,
      chain,
      claim_amount: claimAmount,
      claim_address: claimAddress,
      mint_fee_hone: 0.001,
      epoch,
      nonce,
      evm_proof: claimProof,
      message: claimProof
        ? 'Submit evm_proof to the wHONE contract on ' + chain + '. Gas paid separately.'
        : 'Claim recorded. EVM proof requires HONE_ORACLE_PRIVKEY to be configured on this node.',
    });
  } catch (err) {
    return res.status(400).json({ error: err.message });
  }
});

module.exports = router;
