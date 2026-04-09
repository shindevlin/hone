"use strict";
/**
 * Bot API — HTTP endpoints for Telegram bots.
 * Bots are thin clients; all DB access goes through here.
 * Protected by BOT_API_KEY header.
 */
const express = require('express');
const crypto = require('crypto');
const router = express.Router();

const User = require('../models/User');
const Wallet = require('../models/Wallet');
const Transaction = require('../models/Transaction');
const Node = require('../models/Node');
const ledger = require('../services/ledger');
const Epoch = require('../models/Epoch');
const WorkProof = require('../models/WorkProof');
const MiningProof = require('../models/MiningProof');
const GenesisDream = require('../models/GenesisDream');
const PeerRegistry = require('../models/PeerRegistry');
const Project = require('../models/Project');
const InferenceJob = require('../models/InferenceJob');
const { getBlockReward } = require('../services/emissionSchedule');
const { startLink, verifySignedChallenge } = require('../services/telegramVerify');

// ── Auth middleware ──
const BOT_API_KEY = process.env.BOT_API_KEY || 'btcpc_bot_secret_change_me';

function requireBotKey(req, res, next) {
  const key = req.headers['x-bot-key'];
  if (key !== BOT_API_KEY) return res.status(401).json({ error: 'invalid bot key' });
  next();
}

router.use(requireBotKey);

// ── Helper: resolve telegram ID to user ──
async function resolveUser(telegramId) {
  if (!telegramId) return null;
  return User.findOne({ telegramId: String(telegramId) });
}

// ════════════════════════════════════════════════════════════════════
// IDENTITY
// ════════════════════════════════════════════════════════════════════

// GET /api/bot/user?telegramId=xxx
router.get('/user', async (req, res) => {
  try {
    const user = await resolveUser(req.query.telegramId);
    if (!user) return res.json({ found: false });
    res.json({ found: true, username: user.username, telegramId: user.telegramId });
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// POST /api/bot/create { username, telegramId, telegramUsername }
// Creates a new BTCPC account from Telegram. Shows mnemonic ONCE. We don't save it.
router.post('/create', async (req, res) => {
  try {
    const { username, telegramId, telegramUsername } = req.body;
    if (!username || !telegramId) {
      return res.status(400).json({ error: 'username and telegramId required' });
    }

    // Check if username is taken
    const existing = await User.findOne({ username });
    if (existing) return res.status(400).json({ error: `Username "${username}" is already taken` });

    // Check if telegram is already linked
    const linked = await User.findOne({ telegramId: String(telegramId) });
    if (linked) return res.status(400).json({ error: `Telegram already linked to ${linked.username}` });

    // Validate username
    if (!/^[a-z0-9][a-z0-9._-]{2,19}$/.test(username)) {
      return res.status(400).json({ error: 'Username must be 3-20 chars, lowercase letters/numbers/.-_ only' });
    }

    // Create the account
    const { createAccount } = require('../wallet/accountManager');
    const keyManager = require('../wallet/keyManager');
    const account = await createAccount(username, null, username + '-telegram');

    // Derive full keys from mnemonic (createAccount doesn't return private keys)
    const keys = await keyManager.mnemonicToKeys(account.mnemonic);
    const chainWallets = await keyManager.deriveChainWallets(account.mnemonic);

    // Link telegram immediately
    const user = await User.findOne({ username });
    if (user) {
      user.telegramId = String(telegramId);
      user.telegramUsername = telegramUsername || null;
      await user.save();
    }

    // Record on permanent ledger
    const ledger = require('../services/ledger');
    await ledger.recordAccountCreate(username, account.publicKeys, account.chainWallets, 0);

    // Announce to P2P
    const p2p = require('../p2p/network');
    const { createMessage } = require('../p2p/protocol');
    try {
      const announceMsg = createMessage('ACCOUNT_ANNOUNCE', {
        username,
        public_keys: account.publicKeys,
        chain_addresses: account.chainWallets,
        epoch: 0
      }, p2p.NODE_ID);
      p2p.broadcast(announceMsg);
    } catch (_) {}

    // Auto-stake 0 for inference (just create the record so bot doesn't block)
    // User will need to receive tokens and stake manually for full access

    res.json({
      success: true,
      warning: 'SAVE YOUR MNEMONIC NOW. We do not store it. If you lose it, your account is gone forever.',
      security_note: 'Creating a wallet via Telegram is less secure than on a PC. Save your mnemonic to a password manager or write it on paper and store it safely.',
      username,
      mnemonic: account.mnemonic,
      keys: {
        owner: { private: keys.owner.privateKey, public: keys.owner.publicKey },
        active: { private: keys.active.privateKey, public: keys.active.publicKey },
        posting: { private: keys.posting.privateKey, public: keys.posting.publicKey },
        memo: { private: keys.memo.privateKey, public: keys.memo.publicKey }
      },
      addresses: {
        btcpc: account.address,
        evm: chainWallets.evm ? chainWallets.evm.address : null,
        solana: chainWallets.solana ? chainWallets.solana.address : null,
        bitcoin: chainWallets.bitcoin ? chainWallets.bitcoin.address : null,
        ton: chainWallets.ton ? chainWallets.ton.address : null,
        hive: username
      }
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// POST /api/bot/link { username, telegramId, telegramUsername }
router.post('/link', async (req, res) => {
  try {
    const { username, telegramId, telegramUsername } = req.body;
    const existing = await User.findOne({ telegramId: String(telegramId) });
    if (existing) return res.status(400).json({ error: `Already linked to ${existing.username}` });
    const result = await startLink(username, telegramId, telegramUsername);
    res.json(result);
  } catch (err) { res.status(400).json({ error: err.message }); }
});

// POST /api/bot/verify { telegramId, signature, recovery }
router.post('/verify', async (req, res) => {
  try {
    const { telegramId, signature, recovery } = req.body;
    const user = await User.findOne({ 'pendingTelegramLink.telegramId': String(telegramId) });
    if (!user) return res.status(404).json({ error: 'No pending link' });
    const result = await verifySignedChallenge(user.username, String(telegramId), signature, recovery);
    res.json(result);
  } catch (err) { res.status(400).json({ error: err.message }); }
});

// POST /api/bot/unlink { telegramId }
router.post('/unlink', async (req, res) => {
  try {
    const user = await resolveUser(req.body.telegramId);
    if (!user) return res.status(404).json({ error: 'No account linked' });
    user.telegramId = null;
    user.telegramUsername = null;
    await user.save();
    res.json({ success: true });
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// ════════════════════════════════════════════════════════════════════
// WALLET
// ════════════════════════════════════════════════════════════════════

// GET /api/bot/balance?telegramId=xxx
router.get('/balance', async (req, res) => {
  try {
    const user = await resolveUser(req.query.telegramId);
    if (!user) return res.status(404).json({ error: 'Not linked' });

    const wallet = await Wallet.findOne({ userId: user._id, chain: 'btcpc' });
    const balance = wallet?.balance?.get('BTCPC') || 0;
    const node = await Node.findOne({ account: user._id });
    const staked = node?.stake_amount || 0;
    const proofCount = await MiningProof.countDocuments({ miner: user.username });
    const dreamCount = await GenesisDream.countDocuments({ current_owner: user.username });

    res.json({
      username: user.username,
      balance, staked,
      address: wallet?.address || null,
      proofCount, dreamCount
    });
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// GET /api/bot/history?telegramId=xxx
router.get('/history', async (req, res) => {
  try {
    const user = await resolveUser(req.query.telegramId);
    if (!user) return res.status(404).json({ error: 'Not linked' });

    const wallet = await Wallet.findOne({ userId: user._id, chain: 'btcpc' });
    if (!wallet) return res.json({ transactions: [] });

    const txs = await Transaction.find({
      $or: [{ from: wallet.address }, { to: wallet.address }, { from: user.username }, { to: user.username }]
    }).sort({ timestamp: -1 }).limit(10).lean();

    res.json({
      transactions: txs.map(tx => ({
        type: tx.type, amount: tx.amount, from: tx.from, to: tx.to,
        direction: (tx.to === wallet.address || tx.to === user.username) ? 'in' : 'out',
        timestamp: tx.timestamp
      }))
    });
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// POST /api/bot/claim { telegramId }
router.post('/claim', async (req, res) => {
  try {
    const user = await resolveUser(req.body.telegramId);
    if (!user) return res.status(404).json({ error: 'Not linked' });

    let wallet = await Wallet.findOne({ userId: user._id, chain: 'btcpc' });
    if (!wallet) {
      wallet = new Wallet({
        userId: user._id, chain: 'btcpc',
        address: 'btcpc_' + crypto.randomBytes(20).toString('hex'),
        balance: new Map([['BTCPC', 0]])
      });
    }

    const balance = wallet.balance.get('BTCPC') || 0;
    if (balance > 0) return res.status(400).json({ error: `Still have ${balance} BTCPC. Use tokens before claiming.` });

    const faucetClaims = await Transaction.countDocuments({ to: wallet.address, type: 'faucet' });
    if (faucetClaims > 0) {
      const hasProject = await Project.findOne({ owner: user.username, verified: true });
      const hasSpent = await Transaction.findOne({ from: wallet.address });
      if (!hasProject && !hasSpent) {
        return res.status(400).json({ error: 'Faucet requires contribution. Register a project or spend tokens first.' });
      }
    }

    // Record on permanent ledger
    const epoch = await ledger.getCurrentEpoch();
    await ledger.recordFaucet(user.username, 1, epoch);

    // Update wallet cache
    wallet.balance.set('BTCPC', balance + 1);
    await wallet.save();

    // Record transaction (legacy index)
    await new Transaction({
      from: 'btcpc_faucet', to: wallet.address, amount: 1, type: 'faucet',
      memo: faucetClaims === 0 ? 'Welcome to BTCPC' : 'Faucet refill'
    }).save();

    res.json({ success: true, balance: balance + 1, firstClaim: faucetClaims === 0 });
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// ════════════════════════════════════════════════════════════════════
// MINING
// ════════════════════════════════════════════════════════════════════

// GET /api/bot/mining?telegramId=xxx
router.get('/mining', async (req, res) => {
  try {
    const user = await resolveUser(req.query.telegramId);
    if (!user) return res.status(404).json({ error: 'Not linked' });

    const totalEpochs = await Epoch.countDocuments({ status: 'finalized' });
    const totalWork = await WorkProof.countDocuments({ node_id: user.username });
    const totalTokens = await WorkProof.aggregate([
      { $match: { node_id: user.username } },
      { $group: { _id: null, total: { $sum: '$tokens_generated' } } },
    ]);
    const tokens = totalTokens[0]?.total || 0;

    const recentEpochs = await Epoch.find({ status: 'finalized' })
      .sort({ epoch_number: -1 }).limit(5).lean();

    res.json({
      username: user.username,
      totalEpochs, totalWork, tokensGenerated: tokens,
      recentEpochs: recentEpochs.map(e => ({
        epoch: e.epoch_number,
        reward: e.rewards_distributed?.[0]?.amount || e.block_reward || 0
      }))
    });
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// GET /api/bot/proofs?telegramId=xxx
router.get('/proofs', async (req, res) => {
  try {
    const user = await resolveUser(req.query.telegramId);
    if (!user) return res.status(404).json({ error: 'Not linked' });

    const proofs = await MiningProof.find({ miner: user.username })
      .sort({ block_number: -1 }).limit(10).lean();

    res.json({
      proofs: proofs.map(p => ({
        block: p.block_number, reward: p.reward_earned,
        tokens: p.tokens_computed, model: p.model
      }))
    });
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// GET /api/bot/dreams?telegramId=xxx
router.get('/dreams', async (req, res) => {
  try {
    const user = await resolveUser(req.query.telegramId);
    if (!user) return res.status(404).json({ error: 'Not linked' });

    const dreams = await GenesisDream.find({ current_owner: user.username })
      .sort({ block_number: -1 }).limit(10).lean();

    res.json({
      dreams: dreams.map(d => ({
        block: d.block_number,
        project: d.inscription?.project || null,
        miner: d.original_miner
      }))
    });
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// GET /api/bot/node?telegramId=xxx
router.get('/node', async (req, res) => {
  try {
    const user = await resolveUser(req.query.telegramId);
    if (!user) return res.status(404).json({ error: 'Not linked' });

    const node = await Node.findOne({ account: user._id }).lean();
    if (!node) return res.status(404).json({ error: 'No mining node registered' });

    res.json({
      status: node.status, models: node.models,
      endpoint: node.endpoint, stake: node.stake_amount,
      reputation: node.reputation, lastEpoch: node.last_epoch_commitment,
      hardware: node.hardware || {}
    });
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// ════════════════════════════════════════════════════════════════════
// NETWORK (no auth needed beyond bot key)
// ════════════════════════════════════════════════════════════════════

// GET /api/bot/network
router.get('/network', async (_req, res) => {
  try {
    const totalEpochs = await Epoch.countDocuments();
    const finalizedEpochs = await Epoch.countDocuments({ status: 'finalized' });
    const activeNodes = await Node.countDocuments({ status: 'active' });
    const totalUsers = await User.countDocuments();
    const totalProofs = await WorkProof.countDocuments();
    const totalMined = await Epoch.aggregate([
      { $match: { status: 'finalized' } },
      { $group: { _id: null, total: { $sum: '$block_reward' } } },
    ]);
    const mined = totalMined[0]?.total || 0;
    const currentReward = getBlockReward(totalEpochs);

    res.json({ totalEpochs, finalizedEpochs, activeNodes, totalUsers, totalProofs, mined, currentReward });
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// GET /api/bot/epoch
router.get('/epoch', async (_req, res) => {
  try {
    const current = await Epoch.findOne().sort({ epoch_number: -1 }).lean();
    if (!current) return res.json({ epoch: null });
    const reward = getBlockReward(current.epoch_number);
    res.json({
      epoch: current.epoch_number, status: current.status, reward,
      totalWork: current.total_work, difficulty: current.difficulty,
      startedAt: current.started_at, endedAt: current.ended_at
    });
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// GET /api/bot/reward
router.get('/reward', async (_req, res) => {
  try {
    const latest = await Epoch.findOne().sort({ epoch_number: -1 }).lean();
    const epoch = latest ? latest.epoch_number : 0;
    const reward = getBlockReward(epoch);
    const nextReward = getBlockReward(epoch + 1);
    res.json({ epoch, reward, nextReward });
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// GET /api/bot/pricing?model=xxx
router.get('/pricing', async (req, res) => {
  try {
    const { getCurrentPricing } = require('../services/pricing');
    const p = await getCurrentPricing(req.query.model || undefined);
    res.json(p);
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// GET /api/bot/models
router.get('/models', async (_req, res) => {
  try {
    const { getNetworkModels, getUnmetDemand } = require('../services/modelRegistry');
    const models = await getNetworkModels();
    const demand = getUnmetDemand();
    res.json({ models, demand });
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// ════════════════════════════════════════════════════════════════════
// PEERS
// ════════════════════════════════════════════════════════════════════

// GET /api/bot/peers
router.get('/peers', async (_req, res) => {
  try {
    const peers = await PeerRegistry.find().sort({ last_seen: -1 }).limit(50).lean();
    res.json({ peers: peers.map(p => ({ address: p.address, username: p.username, gpu: p.gpu, last_seen: p.last_seen })) });
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// POST /api/bot/peers/register { telegramId, address }
router.post('/peers/register', async (req, res) => {
  try {
    const { telegramId, address } = req.body;
    const user = await resolveUser(telegramId);
    if (!user) return res.status(404).json({ error: 'Not linked' });

    const node = await Node.findOne({ account: user._id }).lean();
    const gpu = node?.hardware?.gpu || null;

    await PeerRegistry.findOneAndUpdate(
      { username: user.username },
      { address, username: user.username, gpu, last_seen: new Date(), node_id: node?._id?.toString() || user._id.toString() },
      { upsert: true }
    );
    const count = await PeerRegistry.countDocuments();
    res.json({ ok: true, peers: count });
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// POST /api/bot/peers/heartbeat { telegramId }
router.post('/peers/heartbeat', async (req, res) => {
  try {
    const user = await resolveUser(req.body.telegramId);
    if (!user) return res.status(404).json({ error: 'Not linked' });
    const result = await PeerRegistry.findOneAndUpdate({ username: user.username }, { last_seen: new Date() });
    res.json({ ok: !!result });
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// ════════════════════════════════════════════════════════════════════
// PROJECTS
// ════════════════════════════════════════════════════════════════════

// GET /api/bot/projects?telegramId=xxx
router.get('/projects', async (req, res) => {
  try {
    const user = await resolveUser(req.query.telegramId);
    if (!user) return res.status(404).json({ error: 'Not linked' });

    const projects = await Project.find({ owner: user.username }).lean();
    res.json({
      projects: projects.map(p => ({
        name: p.name, verified: p.verified, balance: p.balance,
        totalRequests: p.totalRequests || 0
      }))
    });
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// ════════════════════════════════════════════════════════════════════
// INFERENCE
// ════════════════════════════════════════════════════════════════════

// POST /api/bot/inference { telegramId, prompt, model }
router.post('/inference', async (req, res) => {
  try {
    const { telegramId, prompt, model } = req.body;
    const user = await resolveUser(telegramId);
    if (!user) return res.status(404).json({ error: 'Not linked' });

    const wallet = await Wallet.findOne({ userId: user._id, chain: 'btcpc' });
    const balance = wallet?.balance?.get('BTCPC') || 0;

    const StakingPool = require('../models/StakingPool');
    const stake = await StakingPool.findOne({ account: user._id, status: 'active' });
    if (!stake || stake.staked_amount < 1) {
      return res.status(400).json({ error: 'Need at least 1 BTCPC staked for inference' });
    }
    if (balance < 0.01) {
      return res.status(400).json({ error: `Insufficient balance: ${balance}` });
    }

    // Submit via internal inference API
    const axios = require('axios');
    const API_URL = 'http://localhost:' + (process.env.PORT || 3100);
    const RELAY_KEY = process.env.BTCPC_RELAY_API_KEY;
    if (!RELAY_KEY) return res.status(500).json({ error: 'BTCPC_RELAY_API_KEY not configured' });

    const submitRes = await axios.post(`${API_URL}/v1/inference/submit`, {
      model: model || 'qwen3.5:27b',
      messages: [{ role: 'user', content: prompt }],
      max_tokens: 1024
    }, { timeout: 10000, headers: { 'Authorization': `Bearer ${RELAY_KEY}` } });

    res.json({ job_id: submitRes.data.job_id, balance });
  } catch (err) {
    res.status(500).json({ error: err.response?.data?.error || err.message });
  }
});

// GET /api/bot/inference/:jobId
router.get('/inference/:jobId', async (req, res) => {
  try {
    const job = await InferenceJob.findOne({ job_id: req.params.jobId }).lean();
    if (!job) return res.status(404).json({ error: 'Job not found' });

    const result = {
      status: job.status, model: job.model,
      result_text: job.result_text || null,
      tokens: job.tokens_generated || 0,
      elapsed_ms: job.elapsed_ms || 0,
      node: job.node_name || null,
      cost: job.cost || 0
    };

    // If completed, deduct from wallet
    if (job.status === 'completed' && !job._bot_deducted) {
      const { calculateCost } = require('../services/pricing');
      const { cost } = await calculateCost(result.tokens, job.model || 'qwen3.5:27b');
      result.cost = Math.max(0.001, cost);
    }

    res.json(result);
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// ════════════════════════════════════════════════════════════════════
// ALERTS
// ════════════════════════════════════════════════════════════════════

// GET /api/bot/linked-users — all users with telegramId set (for alerts)
router.get('/linked-users', async (_req, res) => {
  try {
    const users = await User.find({ telegramId: { $ne: null } }).select('telegramId username').lean();
    res.json({ users });
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// ════════════════════════════════════════════════════════════════════
// UPDATE MANAGEMENT
// ════════════════════════════════════════════════════════════════════

// GET /api/bot/update-status — check if there's a pending update
router.get('/update-status', async (_req, res) => {
  try {
    const { getPendingUpdate } = require('../services/autoUpdater');
    const update = getPendingUpdate();
    res.json({ pending: !!update, update });
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// POST /api/bot/approve-update — approve and restart with pending update
router.post('/approve-update', async (req, res) => {
  try {
    const { approveUpdate, getPendingUpdate } = require('../services/autoUpdater');
    const update = getPendingUpdate();
    if (!update) return res.status(404).json({ error: 'No pending update' });

    const approved = approveUpdate();
    if (approved) {
      res.json({ success: true, message: `Restarting to v${update.version}...` });
    } else {
      res.status(400).json({ error: 'Could not approve update' });
    }
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// ════════════════════════════════════════════════════════════════════
// CHAIN LINK — Link external wallets (ETH, etc.) to BTCPC account
// ════════════════════════════════════════════════════════════════════

// POST /api/bot/link-chain { telegramId, chain, address }
router.post('/link-chain', async (req, res) => {
  try {
    const user = await resolveUser(req.body.telegramId);
    if (!user) return res.status(404).json({ error: 'Not linked' });

    const { chain, address } = req.body;
    if (!chain || !address) return res.status(400).json({ error: 'chain and address required' });

    const chainLink = require('../services/chainLink');
    const challenge = chainLink.generateChallenge(user.username, chain, address);

    res.json({
      challengeId: challenge.challengeId,
      message: challenge.message,
      instructions: 'Sign this exact message with your ' + chain.toUpperCase() + ' wallet, then submit the signature via /verify-chain',
      expiresIn: challenge.expiresIn
    });
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// POST /api/bot/verify-chain { telegramId, challengeId, signature }
router.post('/verify-chain', async (req, res) => {
  try {
    const { challengeId, signature } = req.body;
    if (!challengeId || !signature) return res.status(400).json({ error: 'challengeId and signature required' });

    const chainLink = require('../services/chainLink');
    const result = await chainLink.verifyAndLink(challengeId, signature);

    if (!result.success) return res.status(400).json({ error: result.error });

    res.json({
      success: true,
      username: result.username,
      chain: result.chain,
      address: result.address,
      message: result.chain.toUpperCase() + ' wallet ' + result.address + ' linked to ' + result.username
    });
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// GET /api/bot/linked-addresses?telegramId=xxx
router.get('/linked-addresses', async (req, res) => {
  try {
    const user = await resolveUser(req.query.telegramId);
    if (!user) return res.status(404).json({ error: 'Not linked' });

    const chainLink = require('../services/chainLink');
    const linked = await chainLink.getLinkedAddresses(user.username);

    res.json({ username: user.username, linked });
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// ════════════════════════════════════════════════════════════════════
// HEARTBEAT
// ════════════════════════════════════════════════════════════════════

// POST /api/bot/heartbeat { telegramId }
router.post('/heartbeat', async (req, res) => {
  try {
    const user = await resolveUser(req.body.telegramId);
    if (!user) return res.status(404).json({ error: 'Not linked' });

    const ledger = require('../services/ledger');
    const epoch = await ledger.getCurrentEpoch();
    await ledger.recordHeartbeat(user.username, epoch);

    res.json({ success: true, username: user.username, message: 'Heartbeat recorded. Your dormancy clock is reset for 5 years.' });
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// ════════════════════════════════════════════════════════════════════
// RESOURCE MANAGEMENT
// ════════════════════════════════════════════════════════════════════

// POST /api/bot/miner-status { telegramId }
router.post('/miner-status', async (req, res) => {
  try {
    const resourceManager = require('../services/resourceManager');
    res.json(resourceManager.getStatus());
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// POST /api/bot/miner-pause { telegramId }
router.post('/miner-pause', async (req, res) => {
  try {
    const resourceManager = require('../services/resourceManager');
    resourceManager.pause();
    res.json({ success: true, mode: 'paused', message: 'Mining paused. Use /resume to restart.' });
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// POST /api/bot/miner-resume { telegramId }
router.post('/miner-resume', async (req, res) => {
  try {
    const resourceManager = require('../services/resourceManager');
    resourceManager.resume();
    res.json({ success: true, mode: 'auto', message: 'Mining resumed. Auto mode: full when idle, reduced when PC in use.' });
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// POST /api/bot/miner-mode { telegramId, mode }
router.post('/miner-mode', async (req, res) => {
  try {
    const { mode } = req.body;
    const resourceManager = require('../services/resourceManager');
    resourceManager.setManualMode(mode || null);
    res.json({ success: true, mode: resourceManager.getMode() });
  } catch (err) { res.status(500).json({ error: err.message }); }
});

module.exports = router;
