"use strict";

/**
 * MCP Tool Registry — register external tool webhooks usable in agentic jobs.
 * Shin Devlin
 *
 * Third-party developers register a tool name + JSON schema + webhook URL.
 * When an agentic job calls that tool, the marketplace POSTs to the webhook
 * with { tool_use_id, name, input } and expects { content } back.
 *
 * Routes:
 *   GET   /api/tools              — list all registered tools (public)
 *   POST  /api/tools              — register a tool (auth)
 *   GET   /api/tools/mine         — tools registered by caller (auth)
 *   GET   /api/tools/:name        — tool schema detail (public)
 *   DELETE /api/tools/:name       — unregister (auth, owner only)
 */

const express = require("express");
const router = express.Router();
const { authenticateToken } = require("../middlewares/auth");
const { sanitizeString } = require("../middlewares/validate");
const stateStore = require("../chain/stateStore");
const ledger = require("../services/ledger");

// ── GET /api/tools ────────────────────────────────────────────────────────────
router.get("/", (req, res) => {
  try {
    const tools = stateStore.getAllRegisteredTools();
    // Strip webhook URLs from public listing (security)
    const sanitized = tools.map((t) => ({
      name: t.name,
      description: t.description,
      schema: t.schema,
      owner: t.owner,
      registered_epoch: t.registered_epoch,
    }));
    res.json({ tools: sanitized });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// ── POST /api/tools ───────────────────────────────────────────────────────────
router.post("/", authenticateToken, async (req, res) => {
  try {
    const owner = req.user.username;
    const name = sanitizeString(req.body.name, 64);
    const description = sanitizeString(req.body.description, 500);
    const webhookUrl = req.body.webhook_url ? sanitizeString(req.body.webhook_url, 512) : null;
    const schema = req.body.schema || null;

    if (!name) return res.status(400).json({ error: "name required" });
    if (!webhookUrl) return res.status(400).json({ error: "webhook_url required" });
    if (!/^https?:\/\//i.test(webhookUrl)) {
      return res.status(400).json({ error: "webhook_url must be http/https" });
    }

    // Prevent overwriting another owner's tool
    const existing = stateStore.getRegisteredTool(name);
    if (existing && existing.owner !== owner) {
      return res.status(409).json({ error: `Tool '${name}' already registered by ${existing.owner}` });
    }

    const epoch = await ledger.getCurrentEpoch();
    await ledger.recordToolRegister(owner, { name, description, schema, webhook_url: webhookUrl }, epoch);

    res.json({ name, owner, description, schema, registered_epoch: epoch });
  } catch (err) {
    res.status(400).json({ error: err.message });
  }
});

// ── GET /api/tools/mine ───────────────────────────────────────────────────────
router.get("/mine", authenticateToken, (req, res) => {
  try {
    const tools = stateStore.getToolsByOwner(req.user.username);
    res.json({ tools });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// ── GET /api/tools/:name ──────────────────────────────────────────────────────
router.get("/:name", (req, res) => {
  const tool = stateStore.getRegisteredTool(req.params.name);
  if (!tool) return res.status(404).json({ error: "Tool not found" });
  res.json({
    name: tool.name,
    description: tool.description,
    schema: tool.schema,
    owner: tool.owner,
    registered_epoch: tool.registered_epoch,
  });
});

// ── DELETE /api/tools/:name ───────────────────────────────────────────────────
router.delete("/:name", authenticateToken, async (req, res) => {
  try {
    const tool = stateStore.getRegisteredTool(req.params.name);
    if (!tool) return res.status(404).json({ error: "Tool not found" });
    if (tool.owner !== req.user.username) {
      return res.status(403).json({ error: "Not your tool" });
    }
    const epoch = await ledger.getCurrentEpoch();
    await ledger.recordToolUnregister(req.user.username, req.params.name, epoch);
    res.json({ deleted: req.params.name });
  } catch (err) {
    res.status(400).json({ error: err.message });
  }
});

module.exports = router;
