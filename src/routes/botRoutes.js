"use strict";
/**
 * Bot API — HTTP endpoints for Telegram bots.
 * Bots are thin clients; all DB access goes through here.
 * Protected by BOT_API_KEY header.
 */
const express = require('express');
const crypto = require('crypto');
const jwt = require('jsonwebtoken');
const router = express.Router();

const User = require('../models/User');
const ledger = require('../services/ledger');
const stateStore = require('../chain/stateStore');
const nodeRegistry = require('../chain/nodeRegistry');
const Project = require('../models/Project');
const { getDreams: getDreamsForAccount } = require('../controllers/dreamController');
const { getJob: getInferenceJob } = require('../inference/p2pRouter');
const { getBlockReward } = require('../services/emissionSchedule');
const secretStore = require('../services/secretStore');
const { authenticateToken } = require('../middlewares/auth');

// In-memory peer registry (Phase E bridge — PeerRegistry model removed)
// username → { address, gpu, last_seen, node_id }
const peerStore = new Map();
const { startLink, verifySignedChallenge } = require('../services/telegramVerify');
const {
  rejectObjectInputs, validAccountName, sanitizeAmount, sanitizeString,
  sanitizeTelegramId, validModel, validChain, validAddress, validEndpoint,
  blockedAccountNameReason,
  validMinerMode
} = require('../middlewares/validate');

// ── Auth middleware ──
const BOT_API_KEY = process.env.BOT_API_KEY;
if (!BOT_API_KEY) {
  console.error('[BTCPC] FATAL: BOT_API_KEY not set in .env — bot routes are disabled');
}

function requireBotKey(req, res, next) {
  if (!BOT_API_KEY) return res.status(503).json({ error: 'Bot API not configured' });
  const key = req.headers['x-bot-key'];
  if (key !== BOT_API_KEY) return res.status(401).json({ error: 'invalid bot key' });
  next();
}

router.use(requireBotKey);

// ── Helper: ensure secretStore is loaded ──
let _secretStoreLoaded = false;
async function ensureSecretStore() {
  if (!_secretStoreLoaded) {
    await secretStore.load();
    _secretStoreLoaded = true;
  }
}

// ── Helper: compute quiz_words from mnemonic array ──
function buildQuizWords(wordArray) {
  const pos1 = Math.floor(Math.random() * 12) + 1;
  let pos2 = Math.floor(Math.random() * 12) + 1;
  while (pos2 === pos1) pos2 = Math.floor(Math.random() * 12) + 1;
  const w1 = wordArray[pos1 - 1];
  const w2 = wordArray[pos2 - 1];
  const answersHash = crypto.createHash('sha256').update(w1 + '|' + w2).digest('hex');
  return { positions: [pos1, pos2], answers_hash: answersHash };
}

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
    const tid = sanitizeTelegramId(req.query.telegramId);
    if (!tid) return res.status(400).json({ error: 'valid telegramId required' });
    const user = await resolveUser(tid);
    if (!user) return res.json({ found: false });
    res.json({ found: true, username: user.username, telegramId: user.telegramId });
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// POST /api/bot/create { username, telegramId?, telegramUsername?, password? }
// Creates a new BTCPC account from Telegram. Shows mnemonic ONCE. We don't save it.
// telegramId and password are both optional — backward compatible with old bot code.
router.post('/create', async (req, res) => {
  try {
    const objErr = rejectObjectInputs(req.body, ['username', 'telegramId', 'telegramUsername', 'password']);
    if (objErr) return res.status(400).json({ error: objErr });
    const username = sanitizeString(req.body.username, 20);
    const telegramId = req.body.telegramId ? sanitizeTelegramId(req.body.telegramId) : null;
    const telegramUsername = sanitizeString(req.body.telegramUsername, 40) || null;
    const password = typeof req.body.password === 'string' ? req.body.password.slice(0, 200) : null;

    if (!username) {
      return res.status(400).json({ error: 'username required' });
    }
    if (!validAccountName(username)) {
      return res.status(400).json({ error: 'Username must be 3-20 chars, lowercase letters/numbers/.-_ only' });
    }
    const blockedReason = blockedAccountNameReason(username);
    if (blockedReason) {
      return res.status(400).json({ error: blockedReason });
    }

    const existing = await User.findOne({ username });
    if (existing) return res.status(400).json({ error: `Username "${username}" is already taken` });

    if (telegramId) {
      const linked = await User.findOne({ telegramId: String(telegramId) });
      if (linked) return res.status(400).json({ error: `Telegram already linked to ${linked.username}` });
    }

    // Create the account
    const { createAccount } = require('../wallet/accountManager');
    const keyManager = require('../wallet/keyManager');
    const accountPassword = password || crypto.randomBytes(32).toString('hex');
    const account = await createAccount(username, null, accountPassword);

    // Derive full keys from mnemonic
    const keys = await keyManager.mnemonicToKeys(account.mnemonic);
    const chainWallets = await keyManager.deriveChainWallets(account.mnemonic);

    // Link telegram if provided
    if (telegramId) {
      const user = await User.findOne({ username });
      if (user) {
        user.telegramId = String(telegramId);
        user.telegramUsername = telegramUsername || null;
        await user.save();
      }
    }

    // Store password in secretStore if provided
    let passwordSet = false;
    if (password) {
      try {
        await ensureSecretStore();
        if (!secretStore.hasUser(username)) {
          await secretStore.createUser(username, {
            password,
            telegram_id: telegramId ? String(telegramId) : null,
            owner_public_key: account.publicKeys.owner,
            active_public_key: account.publicKeys.active,
            posting_public_key: account.publicKeys.posting,
            memo_public_key: account.publicKeys.memo,
          });
        } else {
          await secretStore.updateUser(username, { password });
        }
        passwordSet = true;
      } catch (_) {}
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

    // Build quiz words for mnemonic verification
    const mnemonicWords = account.mnemonic.split(' ');
    const quizWords = buildQuizWords(mnemonicWords);

    // Resolve chain wallet addresses
    const evmAddr = chainWallets.evm ? (chainWallets.evm.address || chainWallets.evm) : null;
    const solanaAddr = chainWallets.solana ? (chainWallets.solana.address || chainWallets.solana) : null;
    const bitcoinAddr = chainWallets.bitcoin ? (chainWallets.bitcoin.address || chainWallets.bitcoin) : null;
    const tonAddr = chainWallets.ton ? (chainWallets.ton.address || chainWallets.ton) : null;

    res.json({
      success: true,
      username,
      mnemonic: account.mnemonic,
      password_set: passwordSet,
      addresses: {
        btcpc: account.address,
        evm: evmAddr,
        solana: solanaAddr,
        bitcoin: bitcoinAddr,
        ton: tonAddr,
        hive: username,
      },
      public_keys: {
        owner: account.publicKeys.owner,
        active: account.publicKeys.active,
        posting: account.publicKeys.posting,
        memo: account.publicKeys.memo,
      },
      quiz_words: quizWords,
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// ── JWT helpers ──
function signBotJwt(username) {
  const secret = process.env.JWT_SECRET || 'btcpc-bot-dev-secret';
  return jwt.sign({ username, src: 'bot' }, secret, { expiresIn: '30d' });
}

// POST /api/bot/login { username, password, telegramId? }
// Verify password via secretStore; optionally link Telegram; return JWT.
router.post('/login', async (req, res) => {
  try {
    const objErr = rejectObjectInputs(req.body, ['username', 'password', 'telegramId']);
    if (objErr) return res.status(400).json({ error: objErr });
    const username = sanitizeString(req.body.username, 20);
    const password = typeof req.body.password === 'string' ? req.body.password.slice(0, 200) : null;
    const telegramId = req.body.telegramId ? sanitizeTelegramId(req.body.telegramId) : null;

    if (!username || !password) {
      return res.status(400).json({ error: 'username and password required' });
    }

    await ensureSecretStore();
    const ok = await secretStore.verifyPassword(username, password);
    if (!ok) {
      return res.status(401).json({ success: false, error: 'Invalid credentials' });
    }

    // Link Telegram if provided
    let linked = false;
    if (telegramId) {
      try {
        await secretStore.updateUser(username, { telegram_id: String(telegramId) });
        linked = true;
      } catch (_) {}
      // Also link in Mongo User
      try {
        const mongoUser = await User.findOne({ username });
        if (mongoUser && !mongoUser.telegramId) {
          mongoUser.telegramId = String(telegramId);
          await mongoUser.save();
        }
      } catch (_) {}
    }

    const token = signBotJwt(username);
    res.json({ success: true, username, linked, token });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// POST /api/bot/change-password  (requires JWT via Authorization: Bearer <token>)
// { old_password, new_password }
router.post('/change-password', authenticateToken, async (req, res) => {
  try {
    const objErr = rejectObjectInputs(req.body, ['old_password', 'new_password']);
    if (objErr) return res.status(400).json({ error: objErr });
    const oldPassword = typeof req.body.old_password === 'string' ? req.body.old_password.slice(0, 200) : null;
    const newPassword = typeof req.body.new_password === 'string' ? req.body.new_password.slice(0, 200) : null;

    if (!oldPassword || !newPassword) {
      return res.status(400).json({ error: 'old_password and new_password required' });
    }

    const username = req.user.username;
    await ensureSecretStore();
    const ok = await secretStore.verifyPassword(username, oldPassword);
    if (!ok) {
      return res.status(401).json({ success: false, error: 'Invalid credentials' });
    }

    await secretStore.updateUser(username, { password: newPassword });
    res.json({ success: true });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// GET /api/bot/addresses?telegramId=xxx
// Returns all 6 chain addresses + owner public key for a linked account.
router.get('/addresses', async (req, res) => {
  try {
    const tid = sanitizeTelegramId(req.query.telegramId);
    if (!tid) return res.status(400).json({ error: 'valid telegramId required' });

    // Resolve via secretStore first, then Mongo
    await ensureSecretStore();
    let username = null;
    const ssUser = secretStore.getUserByTelegramId(String(tid));
    if (ssUser) {
      username = Object.keys(secretStore.stats ? {} : {}).find(() => false) || null;
      // Walk the secretStore state to find the username
      try {
        // getUserByTelegramId returns the user record; we need to find username by reverse lookup
        const allUsers = secretStore.getAllUsers ? secretStore.getAllUsers() : [];
        for (const u of allUsers) {
          const rec = secretStore.getUser(u.username);
          if (rec && rec.telegram_id === String(tid)) { username = u.username; break; }
        }
      } catch (_) {}
    }

    // Fallback to Mongo
    if (!username) {
      const mongoUser = await User.findOne({ telegramId: String(tid) });
      if (mongoUser) username = mongoUser.username;
    }

    if (!username) return res.status(404).json({ error: 'No account linked to this Telegram ID' });

    // Resolve addresses from stateStore / ledger
    const accountState = stateStore.getAccount ? stateStore.getAccount(username) : null;
    const chainAddresses = accountState && accountState.chain_addresses ? accountState.chain_addresses : {};
    const publicKeys = accountState && accountState.public_keys ? accountState.public_keys : {};

    // Fallback: owner key from secretStore
    let ownerKey = publicKeys.owner || null;
    if (!ownerKey) {
      const ssRec = secretStore.getUser(username);
      if (ssRec) ownerKey = ssRec.owner_public_key || null;
    }

    // Fallback: addresses from Mongo User
    if (!chainAddresses.evm) {
      try {
        const mongoUser = await User.findOne({ username });
        if (mongoUser) {
          if (!ownerKey) ownerKey = mongoUser.ownerPublicKey || null;
        }
      } catch (_) {}
    }

    res.json({
      username,
      addresses: {
        btcpc: chainAddresses.btcpc || username,
        evm: chainAddresses.evm || null,
        solana: chainAddresses.solana || null,
        bitcoin: chainAddresses.bitcoin || null,
        ton: chainAddresses.ton || null,
        hive: chainAddresses.hive || username,
      },
      public_keys: {
        owner: ownerKey,
      },
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// POST /api/onboard or /api/bot/onboard { username, source: "openclaw" }
// Single-step onboarding for OpenClaw integrations.
router.post('/onboard', async (req, res) => {
  try {
    const objErr = rejectObjectInputs(req.body, ['username', 'source']);
    if (objErr) return res.status(400).json({ error: objErr });

    const username = sanitizeString(req.body.username, 20);
    const source = sanitizeString(req.body.source, 20);

    if (!username || source !== 'openclaw') {
      return res.status(400).json({ error: 'username and source=openclaw required' });
    }
    if (!/^[a-z0-9][a-z0-9-]{2,19}$/.test(username)) {
      return res.status(400).json({ error: 'Username must be 3-20 chars, lowercase letters/numbers/hyphens only' });
    }
    const blockedReason = blockedAccountNameReason(username);
    if (blockedReason) {
      return res.status(400).json({ error: blockedReason });
    }

    const existingUser = await User.findOne({ username });
    if (existingUser) return res.status(400).json({ error: `Username "${username}" is already taken` });

    const existingProject = await Project.findOne({ owner: username, repo: source });
    if (existingProject) return res.status(400).json({ error: 'Onboarding project already exists for this username' });

    const { createAccount } = require('../wallet/accountManager');
    const randomPassword = crypto.randomBytes(32).toString('hex');
    const account = await createAccount(username, null, randomPassword);

    await ledger.recordAccountCreate(username, account.publicKeys, account.chainWallets, 0);

    const apiKey = 'btcpc_' + crypto.randomBytes(32).toString('hex');
    const project = new Project({
      name: `openclaw/${username}`,
      repoUrl: `openclaw://${username}`,
      owner: username,
      repo: source,
      apiKey,
      walletAddress: account.address,
      balance: 1,
      verified: true,
      verifiedAt: new Date()
    });
    await project.save();

    const epoch = await ledger.getCurrentEpoch();
    await ledger.recordTransfer('btcpc_treasury', username, 1, 'BTCPC', null, epoch, 'OpenClaw onboarding bonus');

    res.status(201).json({
      username,
      api_key: apiKey,
      balance: 1,
      models_url: '/v1/models',
      chat_url: '/v1/chat/completions'
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// POST /api/bot/link { username, telegramId, telegramUsername }
router.post('/link', async (req, res) => {
  try {
    const objErr = rejectObjectInputs(req.body, ['username', 'telegramId', 'telegramUsername']);
    if (objErr) return res.status(400).json({ error: objErr });
    const username = sanitizeString(req.body.username, 20);
    const telegramId = sanitizeTelegramId(req.body.telegramId);
    const telegramUsername = sanitizeString(req.body.telegramUsername, 40) || null;
    if (!username || !telegramId) return res.status(400).json({ error: 'username and telegramId required' });
    if (!validAccountName(username)) return res.status(400).json({ error: 'invalid username format' });
    const existing = await User.findOne({ telegramId: String(telegramId) });
    if (existing) return res.status(400).json({ error: `Already linked to ${existing.username}` });
    const result = await startLink(username, telegramId, telegramUsername);
    res.json(result);
  } catch (err) { res.status(400).json({ error: err.message }); }
});

// POST /api/bot/verify { telegramId, signature, recovery }
router.post('/verify', async (req, res) => {
  try {
    const objErr = rejectObjectInputs(req.body, ['telegramId', 'signature', 'recovery']);
    if (objErr) return res.status(400).json({ error: objErr });
    const telegramId = sanitizeTelegramId(req.body.telegramId);
    const signature = sanitizeString(req.body.signature, 500);
    const recovery = sanitizeString(req.body.recovery, 500);
    if (!telegramId) return res.status(400).json({ error: 'valid telegramId required' });
    const user = await User.findOne({ 'pendingTelegramLink.telegramId': String(telegramId) });
    if (!user) return res.status(404).json({ error: 'No pending link' });
    const result = await verifySignedChallenge(user.username, String(telegramId), signature, recovery);
    res.json(result);
  } catch (err) { res.status(400).json({ error: err.message }); }
});

// POST /api/bot/unlink { telegramId }
router.post('/unlink', async (req, res) => {
  try {
    const tid = sanitizeTelegramId(req.body.telegramId);
    if (!tid) return res.status(400).json({ error: 'valid telegramId required' });
    const user = await resolveUser(tid);
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
    const tid = sanitizeTelegramId(req.query.telegramId);
    if (!tid) return res.status(400).json({ error: 'valid telegramId required' });
    const user = await resolveUser(tid);
    if (!user) return res.status(404).json({ error: 'Not linked' });

    // Balance from stateStore (O(1), no Mongo round-trip)
    const balance = stateStore.getBalance(user.username, 'BTCPC');
    const nodeInfo = nodeRegistry.getNode(user.username);
    const staked = nodeInfo?.stake || 0;
    const stakePool = stateStore.getStakePool ? stateStore.getStakePool(user.username) : null;
    const stakedAmount = stakePool?.staked_amount || staked;
    const userProofs = stateStore.getComputeProofs ? Object.values(stateStore.getAllComputeProofs ? stateStore.getAllComputeProofs() : {}).flat().filter(p => p && p.node_id === user.username) : [];
    const proofCount = userProofs.length;
    const dreams = await getDreamsForAccount(user.username);
    const dreamCount = dreams.length;
    // Derive BTCPC address from stateStore account
    const accountState = stateStore.getAccount ? stateStore.getAccount(user.username) : null;
    const address = accountState?.chain_addresses?.btcpc || null;

    res.json({
      username: user.username,
      balance, staked: stakedAmount,
      address,
      proofCount, dreamCount
    });
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// GET /api/bot/history?telegramId=xxx
router.get('/history', async (req, res) => {
  try {
    const tid = sanitizeTelegramId(req.query.telegramId);
    if (!tid) return res.status(400).json({ error: 'valid telegramId required' });
    const user = await resolveUser(tid);
    if (!user) return res.status(404).json({ error: 'Not linked' });

    // Transaction history from block replay — return empty for now (Phase E bridge)
    res.json({ transactions: [], note: 'Full history available via block explorer' });
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// POST /api/bot/claim { telegramId }
router.post('/claim', async (req, res) => {
  try {
    const tid = sanitizeTelegramId(req.body.telegramId);
    if (!tid) return res.status(400).json({ error: 'valid telegramId required' });
    const user = await resolveUser(tid);
    if (!user) return res.status(404).json({ error: 'Not linked' });

    const balance = stateStore.getBalance(user.username, 'BTCPC');
    if (balance > 0) return res.status(400).json({ error: `Still have ${balance} BTCPC. Use tokens before claiming.` });

    // Check if this is a first-time claim (no account state yet)
    const accountState = stateStore.getAccount ? stateStore.getAccount(user.username) : null;
    const isFirstClaim = !accountState;

    if (!isFirstClaim) {
      const hasProject = await Project.findOne({ owner: user.username, verified: true });
      if (!hasProject) {
        return res.status(400).json({ error: 'Faucet requires contribution. Register a project or spend tokens first.' });
      }
    }

    // Record on permanent ledger
    const epoch = await ledger.getCurrentEpoch();
    await ledger.recordFaucet(user.username, 1, epoch);

    res.json({ success: true, balance: balance + 1, firstClaim: isFirstClaim });
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// ════════════════════════════════════════════════════════════════════
// MINING
// ════════════════════════════════════════════════════════════════════

// GET /api/bot/mining?telegramId=xxx
router.get('/mining', async (req, res) => {
  try {
    const tid = sanitizeTelegramId(req.query.telegramId);
    if (!tid) return res.status(400).json({ error: 'valid telegramId required' });
    const user = await resolveUser(tid);
    if (!user) return res.status(404).json({ error: 'Not linked' });

    const totalEpochs = Math.max(0, stateStore.getChainHeight() + 1);
    // Aggregate compute proofs from stateStore for this user
    let totalWork = 0;
    let totalTokens = 0;
    if (stateStore.getAllComputeProofs) {
      const allProofs = stateStore.getAllComputeProofs();
      for (const proofs of Object.values(allProofs)) {
        for (const p of (proofs || [])) {
          if (p && p.node_id === user.username) {
            totalWork++;
            totalTokens += p.tokens_generated || 0;
          }
        }
      }
    }
    const tokens = totalTokens;

    const recentEpochs = stateStore.getRecentEpochs ? stateStore.getRecentEpochs(5) : [];

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
    const tid = sanitizeTelegramId(req.query.telegramId);
    if (!tid) return res.status(400).json({ error: 'valid telegramId required' });
    const user = await resolveUser(tid);
    if (!user) return res.status(404).json({ error: 'Not linked' });

    // Gather mining proofs from stateStore compute proofs
    const allProofsMap = stateStore.getAllComputeProofs ? stateStore.getAllComputeProofs() : {};
    const userProofsList = [];
    for (const [epochNum, proofs] of Object.entries(allProofsMap)) {
      for (const p of (proofs || [])) {
        if (p && p.node_id === user.username) {
          userProofsList.push({ ...p, epoch: Number(epochNum) });
        }
      }
    }
    userProofsList.sort((a, b) => b.epoch - a.epoch);
    const recentProofs = userProofsList.slice(0, 10);

    res.json({
      proofs: recentProofs.map(p => ({
        block: p.epoch, reward: p.reward || 0,
        tokens: p.tokens_generated || 0, model: p.model || null
      }))
    });
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// GET /api/bot/dreams?telegramId=xxx
router.get('/dreams', async (req, res) => {
  try {
    const tid = sanitizeTelegramId(req.query.telegramId);
    if (!tid) return res.status(400).json({ error: 'valid telegramId required' });
    const user = await resolveUser(tid);
    if (!user) return res.status(404).json({ error: 'Not linked' });

    const allDreams = await getDreamsForAccount(user.username);
    const dreams = allDreams.slice(-10).reverse();

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
    const tid = sanitizeTelegramId(req.query.telegramId);
    if (!tid) return res.status(400).json({ error: 'valid telegramId required' });
    const user = await resolveUser(tid);
    if (!user) return res.status(404).json({ error: 'Not linked' });

    const node = nodeRegistry.getNode(user.username);
    if (!node) return res.status(404).json({ error: 'No mining node registered' });

    res.json({
      status: node.status || 'active', models: node.models || [],
      endpoint: node.p2p_address || node.endpoint || null, stake: node.stake || 0,
      reputation: node.reputation || 0, lastEpoch: node.last_epoch || null,
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
    // Chain height + epoch counts come from stateStore
    const totalEpochs = Math.max(0, stateStore.getChainHeight() + 1);
    const finalizedEpochs = totalEpochs; // every replayed epoch is finalized
    const activeNodes = nodeRegistry.getRegisteredNodes().length;
    const totalUsers = await User.countDocuments();
    // Count all compute proofs across all epochs
    let totalProofs = 0;
    if (stateStore.getAllComputeProofs) {
      for (const proofs of Object.values(stateStore.getAllComputeProofs())) {
        totalProofs += (proofs || []).length;
      }
    }
    // Sum mined from stateStore epochs
    let mined = 0;
    for (let i = 0; i < totalEpochs; i++) {
      const ep = stateStore.getEpoch ? stateStore.getEpoch(i) : null;
      if (ep) mined += ep.block_reward || 0;
    }
    const currentReward = getBlockReward(totalEpochs);

    res.json({ totalEpochs, finalizedEpochs, activeNodes, totalUsers, totalProofs, mined, currentReward });
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// GET /api/bot/epoch
router.get('/epoch', async (_req, res) => {
  try {
    // Chain state from stateStore
    const chainHeight = stateStore.getChainHeight();
    const current = stateStore.getLatestEpoch();
    if (!current) return res.json({ epoch: null });
    const epochNumber = chainHeight;
    const reward = getBlockReward(epochNumber);
    res.json({
      epoch: epochNumber, status: current.status, reward,
      totalWork: current.total_work, difficulty: current.difficulty,
      startedAt: current.started_at, endedAt: current.ended_at
    });
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// GET /api/bot/reward
router.get('/reward', async (_req, res) => {
  try {
    const epoch = Math.max(0, stateStore.getChainHeight());
    const reward = getBlockReward(epoch);
    const nextReward = getBlockReward(epoch + 1);
    res.json({ epoch, reward, nextReward });
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// GET /api/bot/pricing?model=xxx
router.get('/pricing', async (req, res) => {
  try {
    const modelParam = req.query.model ? sanitizeString(req.query.model, 100) : undefined;
    if (modelParam && !validModel(modelParam)) return res.status(400).json({ error: 'invalid model name' });
    const { getCurrentPricing } = require('../services/pricing');
    const p = await getCurrentPricing(modelParam);
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
    const peers = Array.from(peerStore.values())
      .sort((a, b) => new Date(b.last_seen) - new Date(a.last_seen))
      .slice(0, 50);
    res.json({ peers: peers.map(p => ({ address: p.address, username: p.username, gpu: p.gpu, last_seen: p.last_seen })) });
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// POST /api/bot/peers/register { telegramId, address }
router.post('/peers/register', async (req, res) => {
  try {
    const objErr = rejectObjectInputs(req.body, ['telegramId', 'address']);
    if (objErr) return res.status(400).json({ error: objErr });
    const telegramId = sanitizeTelegramId(req.body.telegramId);
    const address = sanitizeString(req.body.address, 500);
    if (!telegramId) return res.status(400).json({ error: 'valid telegramId required' });
    if (!address) return res.status(400).json({ error: 'address required' });
    const user = await resolveUser(telegramId);
    if (!user) return res.status(404).json({ error: 'Not linked' });

    const nodeInfo = nodeRegistry.getNode(user.username);
    const gpu = nodeInfo?.hardware?.gpu || null;

    peerStore.set(user.username, {
      address, username: user.username, gpu, last_seen: new Date(),
      node_id: user._id.toString()
    });
    const count = peerStore.size;
    res.json({ ok: true, peers: count });
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// POST /api/bot/peers/heartbeat { telegramId }
router.post('/peers/heartbeat', async (req, res) => {
  try {
    const tid = sanitizeTelegramId(req.body.telegramId);
    if (!tid) return res.status(400).json({ error: 'valid telegramId required' });
    const user = await resolveUser(tid);
    if (!user) return res.status(404).json({ error: 'Not linked' });
    const existing = peerStore.get(user.username);
    if (existing) {
      existing.last_seen = new Date();
      peerStore.set(user.username, existing);
    }
    res.json({ ok: !!existing });
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// ════════════════════════════════════════════════════════════════════
// PROJECTS
// ════════════════════════════════════════════════════════════════════

// GET /api/bot/projects?telegramId=xxx
router.get('/projects', async (req, res) => {
  try {
    const tid = sanitizeTelegramId(req.query.telegramId);
    if (!tid) return res.status(400).json({ error: 'valid telegramId required' });
    const user = await resolveUser(tid);
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
    const objErr = rejectObjectInputs(req.body, ['telegramId', 'prompt', 'model']);
    if (objErr) return res.status(400).json({ error: objErr });
    const telegramId = sanitizeTelegramId(req.body.telegramId);
    const prompt = sanitizeString(req.body.prompt, 10000);
    const model = req.body.model ? sanitizeString(req.body.model, 100) : null;
    if (!telegramId) return res.status(400).json({ error: 'valid telegramId required' });
    if (!prompt || prompt.length === 0) return res.status(400).json({ error: 'prompt is required' });
    if (model && !validModel(model)) return res.status(400).json({ error: 'invalid model name' });
    const user = await resolveUser(telegramId);
    if (!user) return res.status(404).json({ error: 'Not linked' });

    // Balance from stateStore (no Mongo round-trip for the balance check)
    const balance = stateStore.getBalance(user.username, 'BTCPC');

    const stakePool = stateStore.getStakePool ? stateStore.getStakePool(user.username) : null;
    if (!stakePool || (stakePool.staked_amount || 0) < 1) {
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
    const jobId = sanitizeString(req.params.jobId, 100);
    if (!jobId || !/^[a-zA-Z0-9_-]+$/.test(jobId)) return res.status(400).json({ error: 'invalid job ID' });
    const job = getInferenceJob(jobId);
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
// TOKEN CREATION
// ════════════════════════════════════════════════════════════════════

// POST /api/bot/create-token { telegramId, name, symbol }
router.post('/create-token', async (req, res) => {
  try {
    const objErr = rejectObjectInputs(req.body, ['telegramId', 'name', 'symbol']);
    if (objErr) return res.status(400).json({ error: objErr });
    const tid = sanitizeTelegramId(req.body.telegramId);
    if (!tid) return res.status(400).json({ error: 'valid telegramId required' });
    const user = await resolveUser(tid);
    if (!user) return res.status(404).json({ error: 'Not linked' });

    const name = sanitizeString(req.body.name, 50);
    const symbol = sanitizeString(req.body.symbol, 10);
    if (!name || !symbol) return res.status(400).json({ error: 'name and symbol required' });
    if (!/^[A-Za-z0-9 _-]+$/.test(name)) return res.status(400).json({ error: 'invalid token name' });
    if (!/^[A-Za-z0-9]+$/.test(symbol)) return res.status(400).json({ error: 'invalid token symbol' });

    const ledger = require('../services/ledger');
    const balance = await ledger.getBalance(user.username, 'BTCPC');
    const fee = 42; // 42 BTCPC flat fee, 42M supply standard

    if (balance < fee) {
      return res.status(400).json({ error: `Insufficient balance. Need ${fee} BTCPC, have ${balance.toFixed(2)}` });
    }

    const epoch = await ledger.getCurrentEpoch();
    await ledger.recordTokenCreate(user.username, {
      name,
      symbol: symbol.toUpperCase(),
      supply: 42000000,
      decimals: 10,
      type: 'fungible'
    }, fee, epoch);

    res.json({
      success: true,
      token: {
        name,
        symbol: symbol.toUpperCase(),
        supply: 42000000,
        decimals: 10,
        fee,
        creator: user.username
      },
      message: `Token ${symbol.toUpperCase()} created. 42M supply minted to ${user.username}. Fee: ${fee} BTCPC.`
    });
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// ════════════════════════════════════════════════════════════════════
// CHAIN LINK — Link external wallets (ETH, etc.) to BTCPC account
// ════════════════════════════════════════════════════════════════════

// POST /api/bot/link-chain { telegramId, chain, address }
router.post('/link-chain', async (req, res) => {
  try {
    const objErr = rejectObjectInputs(req.body, ['telegramId', 'chain', 'address']);
    if (objErr) return res.status(400).json({ error: objErr });
    const tid = sanitizeTelegramId(req.body.telegramId);
    if (!tid) return res.status(400).json({ error: 'valid telegramId required' });
    const user = await resolveUser(tid);
    if (!user) return res.status(404).json({ error: 'Not linked' });

    const chain = sanitizeString(req.body.chain, 20);
    const address = sanitizeString(req.body.address, 200);
    if (!chain || !address) return res.status(400).json({ error: 'chain and address required' });
    if (!validChain(chain)) return res.status(400).json({ error: 'unsupported chain' });

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
    const objErr = rejectObjectInputs(req.body, ['challengeId', 'signature']);
    if (objErr) return res.status(400).json({ error: objErr });
    const challengeId = sanitizeString(req.body.challengeId, 200);
    const signature = sanitizeString(req.body.signature, 1000);
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
    const tid = sanitizeTelegramId(req.query.telegramId);
    if (!tid) return res.status(400).json({ error: 'valid telegramId required' });
    const user = await resolveUser(tid);
    if (!user) return res.status(404).json({ error: 'Not linked' });

    const chainLink = require('../services/chainLink');
    const linked = await chainLink.getLinkedAddresses(user.username);

    res.json({ username: user.username, linked });
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// ════════════════════════════════════════════════════════════════════
// REDDIT LINKING
// ════════════════════════════════════════════════════════════════════

// POST /api/bot/reddit-link { redditUsername, btcpcUsername }
// Step 1: Reddit user requests to link their BTCPC account
router.post('/reddit-link', async (req, res) => {
  try {
    const objErr = rejectObjectInputs(req.body, ['redditUsername', 'btcpcUsername']);
    if (objErr) return res.status(400).json({ error: objErr });
    const redditUser = sanitizeString(req.body.redditUsername, 50);
    const btcpcUser = sanitizeString(req.body.btcpcUsername, 20);
    if (!redditUser || !btcpcUser) return res.status(400).json({ error: 'redditUsername and btcpcUsername required' });

    // Verify BTCPC account exists
    const User = require('../models/User');
    const user = await User.findOne({ username: btcpcUser });
    if (!user) return res.status(404).json({ error: 'BTCPC account not found' });

    const chainLink = require('../services/chainLink');
    const challenge = chainLink.generateChallenge(btcpcUser, 'reddit', redditUser);

    res.json({
      challengeId: challenge.challengeId,
      code: challenge.challengeId, // For Reddit, the challenge ID IS the confirmation code
      instructions: 'Confirm this link in the BTCPC Devvit app to prove you control this Reddit account',
      expiresIn: challenge.expiresIn
    });
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// POST /api/bot/reddit-verify { challengeId, redditUsername, signature }
// Step 2: Devvit app confirms the link with HMAC proof
// The Devvit app computes HMAC(challengeId, BTCPC_DEVVIT_SECRET) as the signature
router.post('/reddit-verify', async (req, res) => {
  try {
    const objErr = rejectObjectInputs(req.body, ['challengeId', 'redditUsername', 'signature']);
    if (objErr) return res.status(400).json({ error: objErr });
    const challengeId = sanitizeString(req.body.challengeId, 200);
    const redditUser = sanitizeString(req.body.redditUsername, 50);
    const signature = sanitizeString(req.body.signature, 200);
    if (!challengeId || !redditUser || !signature) return res.status(400).json({ error: 'challengeId, redditUsername, and signature required' });

    const chainLink = require('../services/chainLink');
    const result = await chainLink.verifyAndLink(challengeId, signature);

    if (!result.success) return res.status(400).json({ error: result.error });

    res.json({
      success: true,
      btcpcUsername: result.username,
      redditUsername: result.address,
      message: 'Reddit u/' + result.address + ' linked to BTCPC ' + result.username
    });
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// GET /api/bot/reddit-account?redditUsername=xxx
// Look up BTCPC account by Reddit username
router.get('/reddit-account', async (req, res) => {
  try {
    const redditUser = sanitizeString(req.query.redditUsername, 50);
    if (!redditUser) return res.status(400).json({ error: 'redditUsername required' });

    const chainLink = require('../services/chainLink');

    // Search stateStore accounts for a reddit link entry
    let username = null;
    if (stateStore.getAllAccounts) {
      const allAccounts = stateStore.getAllAccounts();
      for (const [uname, account] of Object.entries(allAccounts)) {
        const chainAddresses = account.chain_addresses || {};
        if (chainAddresses.reddit && chainAddresses.reddit.toLowerCase() === redditUser.toLowerCase()) {
          username = uname;
          break;
        }
      }
    }

    if (!username) return res.status(404).json({ error: 'No BTCPC account linked to u/' + redditUser });

    const linked = await chainLink.getLinkedAddresses(username);

    // Get balance
    const ledger = require('../services/ledger');
    const balance = await ledger.getBalance(username);

    res.json({
      btcpcUsername: username,
      redditUsername: redditUser,
      balance,
      linked
    });
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// ════════════════════════════════════════════════════════════════════
// HEARTBEAT
// ════════════════════════════════════════════════════════════════════

// POST /api/bot/heartbeat { telegramId }
router.post('/heartbeat', async (req, res) => {
  try {
    const tid = sanitizeTelegramId(req.body.telegramId);
    if (!tid) return res.status(400).json({ error: 'valid telegramId required' });
    const user = await resolveUser(tid);
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
    const mode = req.body.mode ? sanitizeString(req.body.mode, 20) : null;
    if (mode && !validMinerMode(mode)) return res.status(400).json({ error: 'mode must be full, reduced, or paused' });
    const resourceManager = require('../services/resourceManager');
    resourceManager.setManualMode(mode || null);
    res.json({ success: true, mode: resourceManager.getMode() });
  } catch (err) { res.status(500).json({ error: err.message }); }
});

// POST /api/bot/reset-password
// Reset a forgotten password by proving ownership via active key signature.
// Body: { username, challenge, signature }
// The caller:
//   1. Requests a challenge string (or generates one: sha256(username + timestamp))
//   2. Signs the challenge with their active private key (derived from mnemonic)
//   3. Sends { username, challenge, signature, new_password }
// The server verifies the signature against the on-chain active public key.
router.post('/reset-password', async (req, res) => {
  try {
    const { rejectObjectInputs, sanitizeString } = require('../middlewares/validate');
    const objErr = rejectObjectInputs(req.body, ['username', 'challenge', 'signature', 'new_password']);
    if (objErr) return res.status(400).json({ error: objErr });

    const username = sanitizeString(req.body.username, 20);
    const challenge = sanitizeString(req.body.challenge, 128);
    const signature = sanitizeString(req.body.signature, 256);
    const newPassword = req.body.new_password;

    if (!username || !challenge || !signature || !newPassword) {
      return res.status(400).json({ error: 'username, challenge, signature, and new_password required' });
    }
    if (typeof newPassword !== 'string' || newPassword.length < 8) {
      return res.status(400).json({ error: 'new_password must be at least 8 characters' });
    }

    // Verify the signature against on-chain active key
    const stateStore = require('../chain/stateStore');
    const account = stateStore.getAccount(username);
    if (!account || !account.public_keys || !account.public_keys.active) {
      return res.status(404).json({ error: 'account not found or has no active key on chain' });
    }

    const keyManager = require('../wallet/keyManager');
    const challengeHash = crypto.createHash('sha256').update(challenge).digest('hex');
    const valid = keyManager.verifySignature(challengeHash, signature, account.public_keys.active);
    if (!valid) {
      return res.status(401).json({ error: 'signature does not match on-chain active key' });
    }

    // Signature verified — reset the password
    const secretStore = require('../services/secretStore');
    try { await secretStore.load(); } catch (_) {}

    if (secretStore.hasUser && secretStore.hasUser(username)) {
      await secretStore.updateUser(username, { password: newPassword });
    } else {
      await secretStore.createUser(username, { password: newPassword });
    }

    res.json({ success: true, message: 'password reset for ' + username });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// ════════════════════════════════════════════════════════════════════
// KEY ROTATION
// ════════════════════════════════════════════════════════════════════

// POST /api/bot/rotate-keys
// Rotate active, posting, and memo keys — authenticated by password.
// Generates 3 new random secp256k1 keypairs server-side.
// New PRIVATE keys are returned ONCE in the response; the bot must show them
// to the user immediately.
// Body:    { username, password }
// Response:{ success, new_keys: { active:{pub,priv}, posting:{pub,priv}, memo:{pub,priv} } }
router.post('/rotate-keys', async (req, res) => {
  try {
    const objErr = rejectObjectInputs(req.body, ['username', 'password']);
    if (objErr) return res.status(400).json({ error: objErr });

    const username = sanitizeString(req.body.username, 20);
    const password = req.body.password;

    if (!username || !password) {
      return res.status(400).json({ error: 'username and password required' });
    }
    if (typeof password !== 'string') {
      return res.status(400).json({ error: 'invalid password' });
    }

    await ensureSecretStore();

    // Verify password
    const valid = await secretStore.verifyPassword(username, password);
    if (!valid) {
      return res.status(401).json({ error: 'invalid password' });
    }

    // Verify account exists on chain
    const account = stateStore.getAccount(username);
    if (!account) {
      return res.status(404).json({ error: 'account not found on chain' });
    }

    // Generate 3 new random secp256k1 keypairs (independent of mnemonic)
    function generateRandomKeypair() {
      const secp256k1 = require('secp256k1');
      let privKey;
      do { privKey = crypto.randomBytes(32); } while (!secp256k1.privateKeyVerify(privKey));
      const pubKey = Buffer.from(secp256k1.publicKeyCreate(privKey, true));
      return { privateKey: privKey.toString('hex'), publicKey: pubKey.toString('hex') };
    }

    const activeKp  = generateRandomKeypair();
    const postingKp = generateRandomKeypair();
    const memoKp    = generateRandomKeypair();

    const newPublicKeys = {
      active:  activeKp.publicKey,
      posting: postingKp.publicKey,
      memo:    memoKp.publicKey,
    };

    // Record on chain
    const epoch = await ledger.getCurrentEpoch();
    await ledger.recordKeyRotate(username, newPublicKeys, 'password', epoch);

    // Return new private keys (shown to user ONCE — bot must prompt for save)
    res.json({
      success: true,
      message: 'Keys rotated. Save the private keys shown below — they will not be shown again.',
      new_keys: {
        active:  { pub: activeKp.publicKey,  priv: activeKp.privateKey  },
        posting: { pub: postingKp.publicKey, priv: postingKp.privateKey },
        memo:    { pub: memoKp.publicKey,    priv: memoKp.privateKey    },
      },
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// POST /api/bot/rotate-owner
// Rotate the owner key — the private key NEVER touches the server.
// The client (CLI or btcpc.net/rotate) signs a challenge with the CURRENT
// owner private key locally, then sends only the new public key + signature.
// Body:    { username, new_owner_public_key, challenge, signature }
//   challenge = sha256(username + new_owner_public_key + timestamp)
//   signature = ecdsaSign(sha256(challenge), CURRENT_owner_private_key)
// Response:{ success }
router.post('/rotate-owner', async (req, res) => {
  try {
    const objErr = rejectObjectInputs(req.body, ['username', 'new_owner_public_key', 'challenge', 'signature']);
    if (objErr) return res.status(400).json({ error: objErr });

    const username       = sanitizeString(req.body.username, 20);
    const newOwnerPubKey = sanitizeString(req.body.new_owner_public_key, 128);
    const challenge      = sanitizeString(req.body.challenge, 256);
    const signature      = sanitizeString(req.body.signature, 512);

    if (!username || !newOwnerPubKey || !challenge || !signature) {
      return res.status(400).json({ error: 'username, new_owner_public_key, challenge, and signature required' });
    }

    // Verify the account exists and has an owner key on chain
    const account = stateStore.getAccount(username);
    if (!account || !account.public_keys || !account.public_keys.owner) {
      return res.status(404).json({ error: 'account not found or has no owner key on chain' });
    }

    // Verify the signature — must be signed by the CURRENT owner private key.
    // The client signs: ecdsaSign(sha256(challenge), ownerPrivKey)
    // where challenge itself is sha256(username + newOwnerPubKey + timestamp).
    // We verify against: sha256(challenge) as a 32-byte message.
    let valid = false;
    try {
      const secp256k1 = require('secp256k1');
      const msgBuf  = crypto.createHash('sha256').update(challenge).digest();
      const sigBuf  = Buffer.from(signature, 'hex');
      const pubBuf  = Buffer.from(account.public_keys.owner, 'hex');
      valid = secp256k1.ecdsaVerify(sigBuf, msgBuf, pubBuf);
    } catch (_) {
      valid = false;
    }
    if (!valid) {
      return res.status(401).json({ error: 'signature does not match on-chain owner key' });
    }

    // Record the owner key rotation on chain
    const epoch = await ledger.getCurrentEpoch();
    await ledger.recordKeyRotate(username, { owner: newOwnerPubKey }, 'owner_signature', epoch);

    res.json({
      success: true,
      message: 'Owner key rotated. The old owner key is now invalid.',
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// POST /api/bot/set-password
// Set password for an account that doesn't have one yet.
// Body: { username, password, telegramId? }
// Requires either: bot API key header, OR valid JWT, OR telegramId matching a linked account.
router.post('/set-password', async (req, res) => {
  try {
    const { sanitizeString } = require('../middlewares/validate');
    const username = sanitizeString(req.body.username, 20);
    const password = req.body.password;

    if (!username || !password) {
      return res.status(400).json({ error: 'username and password required' });
    }
    if (typeof password !== 'string' || password.length < 8) {
      return res.status(400).json({ error: 'password must be at least 8 characters' });
    }

    const secretStore = require('../services/secretStore');
    try { await secretStore.load(); } catch (_) {}

    if (secretStore.hasUser && secretStore.hasUser(username)) {
      // Account exists — check it doesn't already have a password
      const existing = secretStore.getUser(username);
      if (existing && existing.password_hash) {
        return res.status(409).json({ error: 'account already has a password — use change-password or reset-password' });
      }
      await secretStore.updateUser(username, { password: password });
    } else {
      await secretStore.createUser(username, { password: password });
    }

    res.json({ success: true, message: 'password set for ' + username });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// ════════════════════════════════════════════════════════════════════
// SEND / IMPORT / EXPORT-MNEMONIC
// ════════════════════════════════════════════════════════════════════

// POST /api/bot/send { from_telegram_id, to_account, amount, password }
// Transfer BTCPC between accounts. Requires password verification.
router.post('/send', async (req, res) => {
  try {
    var objErr = rejectObjectInputs(req.body, ['from_telegram_id', 'to_account', 'amount', 'password']);
    if (objErr) return res.status(400).json({ error: objErr });

    var fromTid = sanitizeTelegramId(req.body.from_telegram_id);
    var toAccount = sanitizeString(req.body.to_account, 20);
    var amount = sanitizeAmount(req.body.amount);
    var password = typeof req.body.password === 'string' ? req.body.password.slice(0, 200) : null;

    if (!fromTid) return res.status(400).json({ error: 'valid from_telegram_id required' });
    if (!toAccount || !validAccountName(toAccount)) return res.status(400).json({ error: 'valid to_account required' });
    if (!amount || amount <= 0) return res.status(400).json({ error: 'positive amount required' });
    if (!password) return res.status(400).json({ error: 'password required' });

    // Resolve sender
    var sender = await resolveUser(fromTid);
    if (!sender) return res.status(404).json({ error: 'Sender account not linked to Telegram' });

    // Verify recipient exists
    var recipientExists = stateStore.hasAccount(toAccount);
    if (!recipientExists) {
      var mongoRecip = await User.findOne({ username: toAccount });
      if (!mongoRecip) return res.status(404).json({ error: 'Recipient account not found' });
    }

    if (sender.username === toAccount) {
      return res.status(400).json({ error: 'Cannot send to yourself' });
    }

    // Verify password
    await ensureSecretStore();
    var pwOk = await secretStore.verifyPassword(sender.username, password);
    if (!pwOk) return res.status(401).json({ error: 'Invalid password' });

    // Check balance
    var balance = stateStore.getBalance(sender.username, 'BTCPC');
    if (balance < amount) {
      return res.status(400).json({ error: 'Insufficient balance: ' + balance + ' BTCPC available' });
    }

    // Execute transfer
    var epoch = stateStore.getChainHeight ? stateStore.getChainHeight() : 0;
    var entry = await ledger.recordTransfer(
      sender.username, toAccount, amount, 'BTCPC', null, epoch,
      'Telegram bot send'
    );

    var newBalance = stateStore.getBalance(sender.username, 'BTCPC');

    res.json({
      success: true,
      balance: newBalance,
      tx_hash: entry ? (entry.hash || entry.tx_hash || null) : null,
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// POST /api/bot/import { telegram_id, mnemonic }
// Import/link an existing BTCPC account by verifying mnemonic ownership.
// Mnemonic is used for verification only — never stored.
router.post('/import', async (req, res) => {
  try {
    var objErr = rejectObjectInputs(req.body, ['telegram_id', 'mnemonic']);
    if (objErr) return res.status(400).json({ error: objErr });

    var telegramId = sanitizeTelegramId(req.body.telegram_id);
    var mnemonic = typeof req.body.mnemonic === 'string' ? req.body.mnemonic.trim().slice(0, 500) : null;

    if (!telegramId) return res.status(400).json({ error: 'valid telegram_id required' });
    if (!mnemonic) return res.status(400).json({ error: 'mnemonic required' });

    // Validate mnemonic format
    var keyManager = require('../wallet/keyManager');
    if (!keyManager.validateMnemonic(mnemonic)) {
      return res.status(400).json({ error: 'Invalid BIP-39 mnemonic' });
    }

    // Check if this telegram_id is already linked
    var existingLink = await User.findOne({ telegramId: String(telegramId) });
    if (existingLink) {
      return res.status(400).json({ error: 'Telegram already linked to ' + existingLink.username + '. Unlink first.' });
    }

    // Derive keys from mnemonic
    var keys = await keyManager.mnemonicToKeys(mnemonic);
    var ownerPubKey = keys.owner ? keys.owner.publicKey : null;

    if (!ownerPubKey) {
      return res.status(400).json({ error: 'Could not derive keys from mnemonic' });
    }

    // Find the on-chain account whose owner public key matches
    var allAccounts = stateStore.getAllAccounts();
    var matchedUsername = null;
    for (var i = 0; i < allAccounts.length; i++) {
      if (allAccounts[i].public_keys && allAccounts[i].public_keys.owner === ownerPubKey) {
        matchedUsername = allAccounts[i].username;
        break;
      }
    }

    if (!matchedUsername) {
      return res.status(404).json({ error: 'No on-chain account matches this mnemonic. Create an account first.' });
    }

    // Link telegram_id to the matched account
    var mongoUser = await User.findOne({ username: matchedUsername });
    if (mongoUser) {
      if (mongoUser.telegramId && mongoUser.telegramId !== String(telegramId)) {
        return res.status(400).json({ error: 'Account already linked to a different Telegram user' });
      }
      mongoUser.telegramId = String(telegramId);
      await mongoUser.save();
    }

    // Also update secretStore
    try {
      await ensureSecretStore();
      if (secretStore.hasUser(matchedUsername)) {
        await secretStore.updateUser(matchedUsername, { telegram_id: String(telegramId) });
      }
    } catch (_) {}

    res.json({ success: true, username: matchedUsername });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// POST /api/bot/export-mnemonic — deliberately disabled.
// Mnemonics are never stored on BTCPC servers.
router.post('/export-mnemonic', async (_req, res) => {
  res.status(403).json({
    error: 'Mnemonics are never stored on BTCPC servers.',
    message: 'Your mnemonic was shown once when you created your account. '
      + 'BTCPC never stores it. If you lost it, use btcpc.net/rotate to generate new keys.',
  });
});

// ════════════════════════════════════════════════════════════════════
// EARNINGS BREAKDOWN
// ════════════════════════════════════════════════════════════════════

// GET /api/bot/earnings?telegramId=xxx  OR  ?account=xxx
// Returns a breakdown of how an account is earning BTCPC, plus recent epochs.
router.get('/earnings', async (req, res) => {
  try {
    let username;

    // Support lookup by telegramId (linked user) or direct account name
    const tid = sanitizeTelegramId(req.query.telegramId);
    const accountParam = sanitizeString(req.query.account, 40);

    if (tid) {
      const user = await resolveUser(tid);
      if (!user) return res.status(404).json({ error: 'Not linked' });
      username = user.username;
    } else if (accountParam && validAccountName(accountParam)) {
      username = accountParam;
    } else {
      return res.status(400).json({ error: 'telegramId or valid account name required' });
    }

    // Get cumulative earnings from stateStore
    const earningsData = stateStore.getEarnings(username);

    // Gather recent epoch reward data for this account
    const recentEpochs = stateStore.getRecentEpochs(20);
    const recentRewards = [];
    for (const ep of recentEpochs) {
      if (!ep.rewards_distributed) continue;
      for (const r of ep.rewards_distributed) {
        if (r.node_id === username) {
          recentRewards.push({
            epoch: ep.epoch_number,
            amount: r.amount,
            type: r.type || 'mining',
          });
        }
      }
    }

    res.json({
      account: username,
      total_earned: earningsData.total,
      breakdown: {
        mining: earningsData.mining,
        clock: earningsData.clock,
        storage: earningsData.storage,
        iot: earningsData.iot,
        verifier: earningsData.verifier,
        service: earningsData.service,
        inference_split: earningsData.inference_split,
      },
      recent_epochs: recentRewards,
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

module.exports = router;
