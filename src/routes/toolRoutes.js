"use strict";

/**
 * BTCPC Tool API Routes
 * Shin Devlin
 *
 * REST API for the native tool use system.
 * Called by: commandRouter, btcpc-cli, Telegram bot.
 *
 * All tool execution runs on the REQUESTER'S machine.
 * These routes expose the local MCP server and tool registry.
 *
 * GET  /api/tools/list           — list all available tools
 * POST /api/tools/call           — call a builtin tool
 * POST /api/tools/register       — register a CLI tool capability on-chain
 * GET  /api/tools/nodes          — list nodes with registered tool capabilities
 * GET  /api/tools/mcp/status     — MCP server health
 * POST /api/tools/mcp/start      — start local MCP server
 * POST /api/tools/mcp/stop       — stop local MCP server
 */

const express = require("express");
const router = express.Router();
const toolRegistry = require("../mcp/toolRegistry");
const mcpServer = require("../mcp/btcpcMcpServer");

// ── List available tools ──────────────────────────────────────────────────────

router.get("/api/tools/list", (req, res) => {
  const tools = toolRegistry.getAllAvailableTools();
  res.json({ tools, count: tools.length });
});

// ── Call a builtin tool ───────────────────────────────────────────────────────

router.post("/api/tools/call", async (req, res) => {
  const { tool, input } = req.body;
  if (!tool) return res.status(400).json({ error: "tool name required" });

  try {
    const output = await mcpServer.callTool(tool, input || {});
    res.json({ ok: true, tool, output });
  } catch (e) {
    res.status(400).json({ ok: false, error: e.message });
  }
});

// ── Register CLI tool capability on-chain ─────────────────────────────────────

router.post("/api/tools/register", async (req, res) => {
  const { name, command, kind, description, account } = req.body;
  if (!name || !command) return res.status(400).json({ error: "name and command required" });
  if (!account) return res.status(400).json({ error: "account required" });

  try {
    const ledger = require("../services/ledger");
    const stateStore = require("../chain/stateStore");
    const epoch = stateStore.getChainHeight() || 0;

    const tools = [{ name, kind: kind || "cli", command, description: description || "", deterministic: false }];
    await ledger.recordToolCapabilityRegister(account, tools, epoch);

    res.json({ ok: true, name, kind: kind || "cli", message: "Tool capability registered on-chain" });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// ── List nodes with tool capabilities ─────────────────────────────────────────

router.get("/api/tools/nodes", (req, res) => {
  const { account } = req.query;
  let nodes = toolRegistry.getAllRegisteredNodes();
  if (account) nodes = nodes.filter(n => n.node_id === account);
  res.json({ nodes, count: nodes.length });
});

// ── MCP server management ─────────────────────────────────────────────────────

router.get("/api/tools/mcp/status", (req, res) => {
  // Check if server is running by attempting to reach it
  const http = require("http");
  const req2 = http.request(
    { hostname: "127.0.0.1", port: mcpServer.PORT, path: "/mcp/health", method: "GET", timeout: 2000 },
    (r) => {
      const chunks = [];
      r.on("data", c => chunks.push(c));
      r.on("end", () => {
        try {
          const data = JSON.parse(Buffer.concat(chunks).toString());
          res.json({ ok: true, port: mcpServer.PORT, ...data });
        } catch (_) {
          res.json({ ok: true, port: mcpServer.PORT });
        }
      });
    }
  );
  req2.on("error", () => res.json({ ok: false, port: mcpServer.PORT, message: "MCP server not running" }));
  req2.on("timeout", () => { req2.destroy(); res.json({ ok: false, port: mcpServer.PORT }); });
  req2.end();
});

router.post("/api/tools/mcp/start", (req, res) => {
  try {
    mcpServer.start();
    res.json({ ok: true, port: mcpServer.PORT, message: "MCP server started" });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

router.post("/api/tools/mcp/stop", (req, res) => {
  mcpServer.stop();
  res.json({ ok: true, message: "MCP server stopped" });
});

module.exports = router;
