"use strict";
const express = require('express');
const crypto = require('crypto');
const axios = require('axios');
const router = express.Router();
const { authenticateToken } = require('../middlewares/auth');
const Project = require('../models/Project');
const Transaction = require('../models/Transaction');
const ledger = require('../services/ledger');

/**
 * POST /api/projects/register
 * Register a GitHub repository to use BTCPC inference.
 * Body: { repoUrl: "https://github.com/owner/repo" }
 * Returns: API key + wallet address
 */
router.post('/register', authenticateToken, async (req, res) => {
  const { repoUrl } = req.body;
  if (!repoUrl) return res.status(400).json({ error: 'repoUrl is required' });

  // Parse GitHub URL
  const match = repoUrl.match(/github\.com\/([^\/]+)\/([^\/\s\.]+)/);
  if (!match) return res.status(400).json({ error: 'Invalid GitHub repository URL' });

  const owner = match[1];
  const repo = match[2];

  try {
    // Check repo exists on GitHub
    const ghRes = await axios.get(`https://api.github.com/repos/${owner}/${repo}`, {
      timeout: 10000,
      validateStatus: s => s < 500
    });
    if (ghRes.status === 404) {
      return res.status(404).json({ error: 'Repository not found on GitHub' });
    }

    // Check not already registered
    const existing = await Project.findOne({ owner, repo });
    if (existing) {
      return res.status(400).json({ error: 'Repository already registered', apiKey: existing.apiKey });
    }

    // Generate API key and wallet
    const apiKey = 'btcpc_' + crypto.randomBytes(32).toString('hex');
    const walletAddress = 'btcpc_proj_' + crypto.randomBytes(16).toString('hex');

    const project = new Project({
      name: `${owner}/${repo}`,
      repoUrl: `https://github.com/${owner}/${repo}`,
      owner,
      repo,
      apiKey,
      walletAddress
    });
    await project.save();

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
        `Add a .btcpc file to your repo root containing: ${walletAddress}`,
        'Push it to your default branch',
        'Call POST /api/projects/verify with your API key to verify ownership',
        'Send BTCPC tokens to your project wallet to start using inference'
      ]
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * POST /api/projects/verify
 * Verify repo ownership by checking for .btcpc file containing the wallet address.
 * Auth: Bearer btcpc_... (project API key)
 */
router.post('/verify', async (req, res) => {
  const authHeader = req.headers.authorization;
  if (!authHeader) return res.status(401).json({ error: 'API key required' });
  const apiKey = authHeader.replace('Bearer ', '').trim();

  try {
    const project = await Project.findOne({ apiKey });
    if (!project) return res.status(404).json({ error: 'Project not found' });
    if (project.verified) return res.json({ success: true, message: 'Already verified' });

    // Fetch .btcpc file from repo default branch
    const rawUrl = `https://raw.githubusercontent.com/${project.owner}/${project.repo}/HEAD/.btcpc`;
    const fileRes = await axios.get(rawUrl, { timeout: 10000, validateStatus: s => s < 500 });

    if (fileRes.status === 404) {
      return res.status(400).json({
        error: 'No .btcpc file found in repository root',
        expected_content: project.walletAddress,
        help: 'Create a file named .btcpc in your repo root containing your wallet address, then push to your default branch.'
      });
    }

    const content = (fileRes.data || '').toString().trim();
    if (content !== project.walletAddress) {
      return res.status(400).json({
        error: '.btcpc file content does not match wallet address',
        expected: project.walletAddress,
        found: content.slice(0, 80)
      });
    }

    project.verified = true;
    project.verifiedAt = new Date();
    await project.save();

    res.json({
      success: true,
      message: 'Repository verified. You can now use BTCPC inference with your API key.',
      project: { name: project.name, verified: true, balance: project.balance }
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
    const project = await Project.findOne({ apiKey }).select('-apiKey');
    if (!project) return res.status(404).json({ error: 'Project not found' });

    res.json({
      name: project.name,
      repoUrl: project.repoUrl,
      walletAddress: project.walletAddress,
      balance: project.balance,
      verified: project.verified,
      totalSpent: project.totalSpent,
      totalRequests: project.totalRequests,
      createdAt: project.createdAt
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * POST /api/projects/fund
 * Send BTCPC tokens to a project wallet. Requires user auth (JWT).
 * Body: { walletAddress: "btcpc_proj_...", amount: 10 }
 */
router.post('/fund', authenticateToken, async (req, res) => {
  const { walletAddress, amount } = req.body;
  if (!walletAddress || !amount || amount <= 0) {
    return res.status(400).json({ error: 'walletAddress and positive amount required' });
  }

  try {
    const project = await Project.findOne({ walletAddress });
    if (!project) return res.status(404).json({ error: 'Project wallet not found' });

    // Find sender's wallet
    const Wallet = require('../models/Wallet');
    const senderWallet = await Wallet.findOne({ userId: req.user.id, chain: 'btcpc' });
    if (!senderWallet) return res.status(404).json({ error: 'Your BTCPC wallet not found' });

    const senderBalance = senderWallet.balance.get('BTCPC') || 0;
    if (senderBalance < amount) return res.status(400).json({ error: 'Insufficient balance' });

    // Resolve username for ledger
    const User = require('../models/User');
    const senderUser = await User.findById(req.user.id);
    const senderName = senderUser?.username || senderWallet.address;

    // Record on permanent ledger
    const epoch = await ledger.getCurrentEpoch();
    await ledger.recordTransfer(senderName, 'project:' + project.name, amount, 'BTCPC', null, epoch, `Fund project: ${project.name}`);

    // Update wallet cache
    senderWallet.balance.set('BTCPC', senderBalance - amount);
    await senderWallet.save();

    project.balance += amount;
    await project.save();

    // Record transaction (legacy index)
    const tx = new Transaction({
      from: senderWallet.address,
      to: walletAddress,
      amount,
      type: 'transfer',
      memo: `Fund project: ${project.name}`
    });
    await tx.save();

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
 * Transfer project ownership to another BTCPC user.
 * Rotates API key (old owner loses access immediately).
 * Transfers project wallet, balance, billing history, and all future revenue.
 *
 * Auth: Bearer btcpc_... (current project API key)
 * Body: { newOwner: "buyerusername" }
 */
router.post('/transfer', authenticateToken, async (req, res) => {
  const { projectName, newOwner } = req.body;
  if (!projectName || !newOwner) {
    return res.status(400).json({ error: 'projectName and newOwner required' });
  }

  try {
    const User = require('../models/User');

    // Find the project — must be owned by the authenticated user
    const project = await Project.findOne({ name: projectName });
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

    // Rotate API key — old owner loses access immediately
    const oldApiKey = project.apiKey;
    const newApiKey = 'btcpc_' + crypto.randomBytes(32).toString('hex');

    // Transfer ownership
    const previousOwner = project.owner;
    project.owner = buyer.username;
    project.apiKey = newApiKey;
    await project.save();

    // Record transfer on-chain as a transaction
    const tx = new Transaction({
      from: currentUser.username,
      to: buyer.username,
      amount: project.balance,
      type: 'transfer',
      memo: `Project transfer: ${project.name} (${previousOwner} → ${buyer.username})`
    });
    await tx.save();

    res.json({
      success: true,
      project: project.name,
      previousOwner: previousOwner,
      newOwner: buyer.username,
      newApiKey: newApiKey,
      balance: project.balance,
      warning: 'Old API key has been revoked. New owner must use the new API key.'
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

module.exports = router;
