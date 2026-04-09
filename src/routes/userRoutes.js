"use strict";
const express = require('express');
const router = express.Router();
const { registerUser, loginUser, linkTelegram, enable2FA } = require('../controllers/authController');
const { authenticateToken } = require('../middlewares/auth');

// Public routes
router.post('/register', registerUser);
router.post('/login', loginUser);

// Protected routes
router.post('/link-telegram', authenticateToken, linkTelegram);
router.post('/enable-2fa', authenticateToken, enable2FA);

// On-chain Telegram verification
const { postVerification } = require('../services/telegramVerify');
router.post('/verify-telegram', authenticateToken, async (req, res) => {
  try {
    const { challenge } = req.body;
    if (!challenge) return res.status(400).json({ error: 'challenge is required' });
    const result = await postVerification(req.user.id, challenge);
    res.json(result);
  } catch (err) {
    res.status(400).json({ error: err.message });
  }
});

// ── MCP Server management ──

// GET /api/user/mcp-servers — list saved MCP servers
router.get('/mcp-servers', authenticateToken, async (req, res) => {
  try {
    const user = await require('../models/User').findById(req.user.id);
    res.json({ servers: user.mcpServers || [] });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// POST /api/user/mcp-servers — save an MCP server to profile
router.post('/mcp-servers', authenticateToken, async (req, res) => {
  const { name, url, tools, description } = req.body;
  if (!name || !url) return res.status(400).json({ error: 'name and url required' });

  // Validate URL — must be HTTPS, no internal/localhost addresses
  try {
    const parsed = new URL(url);
    if (!['https:', 'http:'].includes(parsed.protocol)) {
      return res.status(400).json({ error: 'MCP server URL must use http or https' });
    }
    const blocked = ['localhost', '127.0.0.1', '0.0.0.0', '::1', '169.254', '10.', '192.168.', '172.16.'];
    if (blocked.some(b => parsed.hostname.startsWith(b) || parsed.hostname === b)) {
      return res.status(400).json({ error: 'MCP server URL cannot point to internal addresses' });
    }
  } catch (_) {
    return res.status(400).json({ error: 'Invalid URL format' });
  }

  try {
    const user = await require('../models/User').findById(req.user.id);
    // Replace if same name exists, otherwise add
    const idx = user.mcpServers.findIndex(s => s.name === name);
    const server = { name, url, tools: tools || [], description: description || null };
    if (idx >= 0) {
      user.mcpServers[idx] = server;
    } else {
      user.mcpServers.push(server);
    }
    await user.save();
    res.json({ success: true, servers: user.mcpServers });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// DELETE /api/user/mcp-servers/:name — remove a saved MCP server
router.delete('/mcp-servers/:name', authenticateToken, async (req, res) => {
  try {
    const user = await require('../models/User').findById(req.user.id);
    user.mcpServers = user.mcpServers.filter(s => s.name !== req.params.name);
    await user.save();
    res.json({ success: true, servers: user.mcpServers });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

module.exports = router;