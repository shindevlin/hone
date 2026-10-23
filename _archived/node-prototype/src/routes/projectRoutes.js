"use strict";
const express = require('express');
const crypto = require('crypto');
const axios = require('axios');
const router = express.Router();
const { authenticateToken } = require('../middlewares/auth');
const Project = require('../models/Project');
// Transaction model removed in Phase E — ledger entries are canonical
const ledger = require('../services/ledger');
const stateStore = require('../chain/stateStore');
const { rejectObjectInputs, sanitizeString, sanitizeAmount, validUrl, validAccountName, validAddress } = require('../middlewares/validate');

// D.5-delta: secretStore-first project lookups, Mongo fallback.
// secretStore has: getProject(name), getProjectByApiKey(plaintext),
//   createProject(fields) → { project, apiKey }, rotateProjectKey(name),
//   getAllProjects() → [{ name, owner, repo_url, repo, wallet_address, created_at }]
// There is no updateProject — writes still go to Mongo (dropped in Phase E).
let _secretStoreLoaded = false;
async function getSecretStore() {
  const secretStore = require('../services/secretStore');
  if (!_secretStoreLoaded) {
    try {
      await secretStore.load();
      _secretStoreLoaded = true;
    } catch (err) {
      console.warn('[projectRoutes] secretStore load failed: ' + err.message);
    }
  }
  return secretStore;
}

// Look up project by name: secretStore first, Mongo fallback.
async function _findProjectByName(name) {
  try {
    const ss = await getSecretStore();
    if (ss && typeof ss.getProject === 'function') {
      const ssProject = ss.getProject(name);
      if (ssProject) return { _source: 'secretStore', name, ...ssProject };
    }
  } catch (_) {}
  return Project.findOne({ name });
}

// Look up project by plaintext API key: secretStore first, Mongo fallback.
// Returns a normalised project object or null.
async function _findProjectByApiKey(plaintextKey) {
  try {
    const ss = await getSecretStore();
    if (ss && typeof ss.getProjectByApiKey === 'function') {
      const ssProject = ss.getProjectByApiKey(plaintextKey);
      if (ssProject) {
        // Resolve the name via getAllProjects (wallet_address is unique)
        const name = _resolveProjectName(ss, ssProject);
        return { _source: 'secretStore', name, ...ssProject };
      }
    }
  } catch (_) {}
  return Project.findOne({ apiKey: plaintextKey });
}

// Resolve the project name from a secretStore project record.
// getAllProjects() strips api_key_hash but keeps all other fields including name.
function _resolveProjectName(ss, projectRecord) {
  try {
    const all = ss.getAllProjects ? ss.getAllProjects() : [];
    const entry = all.find(p => p.wallet_address === projectRecord.wallet_address);
    return entry ? entry.name : (projectRecord.owner + '/' + (projectRecord.repo || ''));
  } catch (_) {
    return projectRecord.owner + '/' + (projectRecord.repo || '');
  }
}

// Create project in secretStore (gets plaintext key) AND mirror to Mongo.
// secretStore is written first so the plaintext key is generated there.
async function _createProjectBothStores({ name, owner, repo, repoUrl, walletAddress }) {
  let plaintextKey = null;
  let ss;

  try {
    ss = await getSecretStore();
    if (ss && typeof ss.createProject === 'function') {
      const result = await ss.createProject({
        name,
        owner,
        repo,
        repo_url: repoUrl,
        wallet_address: walletAddress,
      });
      plaintextKey = result.apiKey; // shown once, never stored in plaintext
    }
  } catch (err) {
    if (err && !err.message.includes('project exists')) throw err;
    // Already exists in secretStore — fall through
  }

  // Mirror to Mongo for backwards compat (Phase E will drop this write).
  // Use the secretStore-generated key if available, otherwise generate a new one.
  const mongoApiKey = plaintextKey || ('hone_' + crypto.randomBytes(32).toString('hex'));
  const project = new Project({
    name,
    repoUrl,
    owner,
    repo,
    apiKey: mongoApiKey,
    walletAddress
  });
  await project.save();

  return { project, apiKey: plaintextKey || mongoApiKey };
}

/**
 * POST /api/projects/register
 * Register a GitHub repository to use HONE inference.
 * Body: { repoUrl: "https://github.com/owner/repo" }
 * Returns: API key + wallet address
 */
router.post('/register', authenticateToken, async (req, res) => {
  if (typeof req.body.repoUrl === 'object') return res.status(400).json({ error: 'repoUrl must be a string' });
  const repoUrl = sanitizeString(req.body.repoUrl, 500);
  if (!repoUrl) return res.status(400).json({ error: 'repoUrl is required' });
  if (!validUrl(repoUrl)) return res.status(400).json({ error: 'invalid URL format' });

  const match = repoUrl.match(/github\.com\/([^\/]+)\/([^\/\s\.]+)/);
  if (!match) return res.status(400).json({ error: 'Invalid GitHub repository URL' });

  const owner = match[1];
  const repo = match[2];
  const projectName = `${owner}/${repo}`;

  try {
    // Check repo exists on GitHub
    const ghRes = await axios.get(`https://api.github.com/repos/${owner}/${repo}`, {
      timeout: 10000,
      validateStatus: s => s < 500
    });
    if (ghRes.status === 404) {
      return res.status(404).json({ error: 'Repository not found on GitHub' });
    }

    // Check not already registered — secretStore first, Mongo fallback
    const existing = await _findProjectByName(projectName);
    if (existing) {
      const existingKey = existing._source === 'secretStore' ? '[rotate key to retrieve]' : existing.apiKey;
      return res.status(400).json({ error: 'Repository already registered', apiKey: existingKey });
    }

    const walletAddress = 'hone_proj_' + crypto.randomBytes(16).toString('hex');
    const { project, apiKey } = await _createProjectBothStores({
      name: projectName,
      owner,
      repo,
      repoUrl: `https://github.com/${owner}/${repo}`,
      walletAddress
    });

    res.status(201).json({
      success: true,
      project: {
        name: project.name,
        repoUrl: project.repoUrl,
        apiKey,
        walletAddress,
        verified: false,
        balance: 0
      },
      next_steps: [
        `Add a .hone file to your repo root containing: ${walletAddress}`,
        'Push it to your default branch',
        'Call POST /api/projects/verify with your API key to verify ownership',
        'Send HONE tokens to your project wallet to start using inference'
      ]
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * POST /api/projects/verify
 * Verify repo ownership by checking for .hone file containing the wallet address.
 * Auth: Bearer hone_... (project API key)
 */
router.post('/verify', async (req, res) => {
  const authHeader = req.headers.authorization;
  if (!authHeader) return res.status(401).json({ error: 'API key required' });
  const apiKey = authHeader.replace('Bearer ', '').trim();

  try {
    // secretStore-first lookup
    const found = await _findProjectByApiKey(apiKey);
    if (!found) return res.status(404).json({ error: 'Project not found' });

    const isSecretStore = found._source === 'secretStore';
    const pOwner = found.owner;
    const pRepo = isSecretStore ? found.repo : found.repo;
    const walletAddress = isSecretStore ? found.wallet_address : found.walletAddress;
    const verified = found.verified !== undefined ? found.verified : (isSecretStore ? false : found.verified);

    if (verified) return res.json({ success: true, message: 'Already verified' });

    // Fetch .hone file from repo default branch
    const rawUrl = `https://raw.githubusercontent.com/${pOwner}/${pRepo}/HEAD/.hone`;
    const fileRes = await axios.get(rawUrl, { timeout: 10000, validateStatus: s => s < 500 });

    if (fileRes.status === 404) {
      return res.status(400).json({
        error: 'No .hone file found in repository root',
        expected_content: walletAddress,
        help: 'Create a file named .hone in your repo root containing your wallet address, then push to your default branch.'
      });
    }

    const content = (fileRes.data || '').toString().trim();
    if (content !== walletAddress) {
      return res.status(400).json({
        error: '.hone file content does not match wallet address',
        expected: walletAddress,
        found: content.slice(0, 80)
      });
    }

    // Write verification to Mongo (secretStore has no updateProject yet)
    let mongoProject = isSecretStore ? await Project.findOne({ walletAddress }) : found;
    if (mongoProject && typeof mongoProject.save === 'function') {
      mongoProject.verified = true;
      mongoProject.verifiedAt = new Date();
      await mongoProject.save();
    }

    const projectName = isSecretStore ? found.name : found.name;
    const balance = isSecretStore ? (found.balance || 0) : found.balance;

    res.json({
      success: true,
      message: 'Repository verified. You can now use HONE inference with your API key.',
      project: { name: projectName, verified: true, balance }
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * GET /api/projects/me
 * Get project info for the authenticated API key.
 */
router.get('/me', async (req, res) => {
  const authHeader = req.headers.authorization;
  if (!authHeader) return res.status(401).json({ error: 'API key required' });
  const apiKey = authHeader.replace('Bearer ', '').trim();

  try {
    const found = await _findProjectByApiKey(apiKey);

    if (!found) return res.status(404).json({ error: 'Project not found' });

    if (found._source === 'secretStore') {
      return res.json({
        name: found.name,
        repoUrl: found.repo_url,
        walletAddress: found.wallet_address,
        balance: found.balance || 0,
        verified: found.verified !== false,
        totalSpent: found.totalSpent || 0,
        totalRequests: found.totalRequests || 0,
        createdAt: found.created_at
      });
    }

    // Mongo result
    res.json({
      name: found.name,
      repoUrl: found.repoUrl,
      walletAddress: found.walletAddress,
      balance: found.balance,
      verified: found.verified,
      totalSpent: found.totalSpent,
      totalRequests: found.totalRequests,
      createdAt: found.createdAt
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * POST /api/projects/fund
 * Send HONE tokens to a project wallet. Requires user auth (JWT).
 * Body: { walletAddress: "hone_proj_...", amount: 10 }
 */
router.post('/fund', authenticateToken, async (req, res) => {
  const objErr = rejectObjectInputs(req.body, ['walletAddress', 'amount']);
  if (objErr) return res.status(400).json({ error: objErr });
  const walletAddress = sanitizeString(req.body.walletAddress, 200);
  const amount = sanitizeAmount(req.body.amount);
  if (!walletAddress || !amount) {
    return res.status(400).json({ error: 'walletAddress and positive amount required' });
  }
  if (!validAddress(walletAddress)) return res.status(400).json({ error: 'invalid wallet address format' });

  try {
    const project = await Project.findOne({ walletAddress });
    if (!project) return res.status(404).json({ error: 'Project wallet not found' });

    // Resolve username for ledger + balance lookup
    const User = require('../models/User');
    const senderUser = await User.findById(req.user.id);
    if (!senderUser) return res.status(404).json({ error: 'User not found' });
    const senderName = senderUser.username;

    const senderBalance = stateStore.getBalance(senderName, 'HONE');
    if (senderBalance < amount) return res.status(400).json({ error: 'Insufficient balance' });

    // Record on permanent ledger
    const epoch = await ledger.getCurrentEpoch();
    await ledger.recordTransfer(senderName, 'project:' + project.name, amount, 'HONE', null, epoch, `Fund project: ${project.name}`);

    project.balance += amount;
    await project.save();

    res.json({
      success: true,
      funded: amount,
      projectBalance: project.balance,
      yourBalance: senderBalance - amount
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * POST /api/projects/transfer
 * Transfer project ownership to another HONE user.
 * Rotates API key (old owner loses access immediately).
 *
 * Auth: JWT (current owner must be authenticated)
 * Body: { projectName, newOwner }
 */
router.post('/transfer', authenticateToken, async (req, res) => {
  const objErr = rejectObjectInputs(req.body, ['projectName', 'newOwner']);
  if (objErr) return res.status(400).json({ error: objErr });
  const projectName = sanitizeString(req.body.projectName, 200);
  const newOwner = sanitizeString(req.body.newOwner, 20);
  if (!projectName || !newOwner) {
    return res.status(400).json({ error: 'projectName and newOwner required' });
  }
  if (!validAccountName(newOwner)) return res.status(400).json({ error: 'invalid newOwner account name' });

  try {
    const User = require('../models/User');

    // Find the project — secretStore first, then Mongo
    const project = await _findProjectByName(projectName);
    if (!project) return res.status(404).json({ error: 'Project not found' });

    // Verify current owner is the authenticated user
    const currentUser = await User.findById(req.user.id);
    if (!currentUser) return res.status(401).json({ error: 'User not found' });

    if (project.owner !== currentUser.username) {
      return res.status(403).json({ error: 'You do not own this project' });
    }

    // Verify new owner exists and is active
    const buyer = await User.findOne({ username: newOwner, isActive: true });
    if (!buyer) {
      return res.status(404).json({ error: 'New owner account not found or inactive' });
    }
    if (buyer.username === currentUser.username) {
      return res.status(400).json({ error: 'Cannot transfer to yourself' });
    }

    const previousOwner = project.owner;
    let newApiKey;

    // Rotate key in secretStore if available
    if (project._source === 'secretStore') {
      try {
        const ss = await getSecretStore();
        if (ss && typeof ss.rotateProjectKey === 'function') {
          const rotated = await ss.rotateProjectKey(projectName);
          newApiKey = rotated.apiKey;
        }
      } catch (err) {
        console.warn('[projectRoutes] secretStore rotateProjectKey failed: ' + err.message);
      }
    }

    if (!newApiKey) {
      newApiKey = 'hone_' + crypto.randomBytes(32).toString('hex');
    }

    // Mirror transfer to Mongo
    const mongoProject = await Project.findOne({ name: projectName });
    const projectBalance = mongoProject ? mongoProject.balance : (project.balance || 0);
    if (mongoProject) {
      mongoProject.owner = buyer.username;
      mongoProject.apiKey = newApiKey;
      await mongoProject.save();
    }

    // Record transfer on permanent ledger (Transaction model removed in Phase E)
    try {
      const epoch = await ledger.getCurrentEpoch();
      await ledger.recordTransfer(
        currentUser.username, buyer.username, 0, 'HONE', null, epoch,
        `Project transfer: ${projectName} (${previousOwner} → ${buyer.username})`
      );
    } catch (_) {}

    res.json({
      success: true,
      project: projectName,
      previousOwner,
      newOwner: buyer.username,
      newApiKey,
      balance: projectBalance,
      warning: 'Old API key has been revoked. New owner must use the new API key.'
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * POST /api/projects/revenue-split
 * Set revenue split configuration for a project.
 * Body: { project: "nsfwotica", splits: [{ account: "natoshisakamoto", percent: 70 }, { account: "hone_recycle", percent: 30 }] }
 * Splits must total exactly 100%.
 * Auth: JWT (project owner must be authenticated).
 */
router.post('/revenue-split', authenticateToken, async (req, res) => {
  const objErr = rejectObjectInputs(req.body, ['project', 'splits']);
  if (objErr) return res.status(400).json({ error: objErr });
  const projectName = sanitizeString(req.body.project, 200);
  if (!projectName) return res.status(400).json({ error: 'project name required' });

  const splits = req.body.splits;
  if (!Array.isArray(splits) || splits.length === 0) {
    return res.status(400).json({ error: 'splits must be a non-empty array of { account, percent }' });
  }

  // Validate each split entry
  let totalPercent = 0;
  const cleanSplits = [];
  for (const s of splits) {
    if (!s || typeof s !== 'object') return res.status(400).json({ error: 'each split must be an object with account and percent' });
    const account = sanitizeString(s.account, 40);
    const percent = Number(s.percent);
    if (!account) return res.status(400).json({ error: 'each split must have an account name' });
    if (!validAccountName(account) && account !== 'hone_recycle') {
      return res.status(400).json({ error: 'invalid account name: ' + account });
    }
    if (!Number.isFinite(percent) || percent <= 0 || percent > 100) {
      return res.status(400).json({ error: 'percent must be between 0 and 100 for account: ' + account });
    }
    totalPercent += percent;
    cleanSplits.push({ account, percent });
  }

  // Splits must total exactly 100%
  if (Math.abs(totalPercent - 100) > 0.001) {
    return res.status(400).json({ error: 'splits must total exactly 100%, got ' + totalPercent + '%' });
  }

  try {
    // Verify the project exists
    const project = await _findProjectByName(projectName);
    if (!project) return res.status(404).json({ error: 'Project not found' });

    // Verify the authenticated user is the project owner
    const User = require('../models/User');
    const currentUser = await User.findById(req.user.id);
    if (!currentUser) return res.status(401).json({ error: 'User not found' });
    if (project.owner !== currentUser.username) {
      return res.status(403).json({ error: 'Only the project owner can set revenue splits' });
    }

    // Record on permanent ledger
    const epoch = await ledger.getCurrentEpoch();
    await ledger.recordProjectRevenueSplit(currentUser.username, projectName, cleanSplits, epoch);

    // Read back from stateStore for confirmation
    const stored = stateStore.getProjectRevenueSplit(projectName);

    res.json({
      success: true,
      project: projectName,
      splits: stored ? stored.splits : cleanSplits,
      set_epoch: stored ? stored.set_epoch : epoch,
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

module.exports = router;
