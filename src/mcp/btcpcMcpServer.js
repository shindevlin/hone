"use strict";

/**
 * BTCPC MCP Server
 * Shin Devlin
 *
 * Runs as a sidecar on each mining node. Exposes tools to the inference
 * runtime via the Model Context Protocol (MCP) JSON-RPC interface.
 *
 * Miners start this server with:
 *   btcpc-mine --tools            (auto-starts on port BTCPC_MCP_PORT)
 *   btcpc tool mcp start          (start standalone)
 *
 * Protocol:
 *   POST /mcp/call   { tool: string, input: object } → { output: any, trace_cid?: string }
 *   GET  /mcp/list                                   → { tools: ToolRecord[] }
 *   GET  /mcp/health                                 → { ok: true, node_id: string }
 *
 * Non-deterministic tools (web_fetch, sensor_read) automatically commit their
 * output to BTCPC-FS and return a trace_cid instead of raw output, so
 * verifiers can check the committed data without re-running the fetch.
 *
 * Adding a custom tool:
 *   Register a handler in LOCAL_HANDLERS below, or configure a CLI passthrough
 *   via BTCPC_MCP_CLI_TOOLS env var (JSON array of { name, command } objects).
 */

const http = require("http");
const crypto = require("crypto");
const { exec } = require("child_process");
const { promisify } = require("util");
const { isPublicHttpUrl } = require("../services/urlSafety");
const execAsync = promisify(exec);

const PORT = parseInt(process.env.BTCPC_MCP_PORT) || 3101;
const NODE_ID = process.env.BTCPC_NODE_ID || process.env.BTCPC_MINER || "unknown";
const CLI_TOOLS = _parseCliTools();

// ── Built-in tool handlers ────────────────────────────────────────────────────

const LOCAL_HANDLERS = {

  calculator: async ({ expr }) => {
    if (!expr || typeof expr !== "string") throw new Error("expr required");
    // Safe eval: only allow math tokens
    if (!/^[0-9+\-*/().%\s^e]+$/i.test(expr.trim())) {
      throw new Error("unsafe expression — only math operators allowed");
    }
    // eslint-disable-next-line no-eval
    const result = Function('"use strict"; return (' + expr + ')')();
    if (!Number.isFinite(result)) throw new Error("non-finite result");
    return { result, expr };
  },

  hash: async ({ input }) => {
    if (typeof input !== "string") throw new Error("input must be string");
    return { sha256: crypto.createHash("sha256").update(input).digest("hex"), input };
  },

  btcpc_fs_read: async ({ cid }) => {
    if (!cid || typeof cid !== "string") throw new Error("cid required");
    // Lazy require to avoid circular dep at module load
    try {
      const blobStore = require("../services/blobStore");
      const data = await blobStore.get(cid);
      return { cid, data };
    } catch (e) {
      throw new Error("btcpc_fs_read: " + e.message);
    }
  },

  epoch_info: async () => {
    try {
      const stateStore = require("../chain/stateStore");
      const { getBlockReward } = require("../services/emissionSchedule");
      const height = stateStore.getChainHeight();
      return {
        epoch: height,
        block_reward: getBlockReward(height),
        timestamp: Date.now(),
      };
    } catch (e) {
      throw new Error("epoch_info: " + e.message);
    }
  },

  web_fetch: async ({ url, method, headers }) => {
    if (!url || typeof url !== "string") throw new Error("url required");
    // Only allow http/https
    if (!/^https?:\/\//i.test(url)) throw new Error("url must be http/https");
    if (!await isPublicHttpUrl(url)) throw new Error("url must resolve to a public address");
    const verb = String(method || "GET").toUpperCase();
    if (!["GET", "HEAD"].includes(verb)) {
      throw new Error("web_fetch only allows GET or HEAD");
    }

    const https = url.startsWith("https") ? require("https") : require("http");
    const result = await new Promise((resolve, reject) => {
      const safeHeaders = {};
      const allowedHeaders = headers && typeof headers === "object" ? headers : {};
      for (const key of ["accept", "user-agent"]) {
        if (allowedHeaders[key]) safeHeaders[key] = String(allowedHeaders[key]).slice(0, 256);
      }
      const req = https.request(url, { method: verb, headers: safeHeaders, timeout: 8000 }, res => {
        const chunks = [];
        res.on("data", c => chunks.push(c));
        res.on("end", () => resolve({
          status: res.statusCode,
          headers: res.headers,
          body: Buffer.concat(chunks).toString("utf8").slice(0, 32768), // 32KB cap
        }));
      });
      req.on("error", reject);
      req.on("timeout", () => { req.destroy(); reject(new Error("web_fetch timeout")); });
      req.end();
    });
    // Commit to BTCPC-FS and return CID (non-deterministic — must be committed)
    const traceCid = await _commitTrace("web_fetch", { url }, result);
    return { status: result.status, body_preview: result.body.slice(0, 512), trace_cid: traceCid };
  },

};

// ── CLI passthrough tools ─────────────────────────────────────────────────────

function _parseCliTools() {
  try {
    return JSON.parse(process.env.BTCPC_MCP_CLI_TOOLS || "[]");
  } catch (_) { return []; }
}

async function _runCliTool(toolDef, input) {
  // Replace {{key}} placeholders in command with sanitized input values
  let cmd = toolDef.command;
  for (const [k, v] of Object.entries(input || {})) {
    const safe = String(v).replace(/[`$\\;"'|&<>(){}[\]!]/g, "");
    cmd = cmd.replace("{{" + k + "}}", safe);
  }
  const { stdout, stderr } = await execAsync(cmd, { timeout: 15000 });
  return { stdout: stdout.slice(0, 16384), stderr: stderr.slice(0, 1024) };
}

// ── Trace commit ──────────────────────────────────────────────────────────────

async function _commitTrace(toolName, input, output) {
  try {
    const blobStore = require("../services/blobStore");
    const payload = JSON.stringify({ tool: toolName, input, output, ts: Date.now() });
    const cid = await blobStore.put(Buffer.from(payload), {
      content_type: "application/json",
      meta: { tool: toolName },
    });
    return cid;
  } catch (_) {
    // If blobStore not available, return a content-hash as fallback CID
    return "sha256:" + crypto.createHash("sha256").update(JSON.stringify(output)).digest("hex");
  }
}

// ── Request handling ──────────────────────────────────────────────────────────

async function _handleCall(body) {
  const { tool, input } = body;
  if (!tool) throw new Error("tool name required");

  // Built-in
  if (LOCAL_HANDLERS[tool]) {
    return await LOCAL_HANDLERS[tool](input || {});
  }

  // CLI passthrough
  const cliDef = CLI_TOOLS.find(t => t.name === tool);
  if (cliDef) {
    return await _runCliTool(cliDef, input || {});
  }

  throw new Error("unknown tool: " + tool);
}

function _listTools() {
  const tools = Object.keys(LOCAL_HANDLERS).map(name => ({
    name,
    kind: name === "web_fetch" ? "mcp" : "builtin",
    deterministic: name !== "web_fetch" && name !== "epoch_info",
  }));
  for (const t of CLI_TOOLS) {
    tools.push({ name: t.name, kind: "cli", description: t.description || "", deterministic: false });
  }
  return tools;
}

// ── HTTP server ───────────────────────────────────────────────────────────────

let _server = null;

function start() {
  if (_server) return _server;

  _server = http.createServer(async (req, res) => {
    res.setHeader("Content-Type", "application/json");

    if (req.method === "GET" && req.url === "/mcp/health") {
      res.end(JSON.stringify({ ok: true, node_id: NODE_ID, tools: _listTools().length }));
      return;
    }

    if (req.method === "GET" && req.url === "/mcp/list") {
      res.end(JSON.stringify({ tools: _listTools(), node_id: NODE_ID }));
      return;
    }

    if (req.method === "POST" && req.url === "/mcp/call") {
      let body;
      try {
        const raw = await _readBody(req);
        body = JSON.parse(raw);
      } catch (_) {
        res.statusCode = 400;
        res.end(JSON.stringify({ error: "invalid JSON body" }));
        return;
      }
      try {
        const output = await _handleCall(body);
        res.end(JSON.stringify({ ok: true, output }));
      } catch (e) {
        res.statusCode = 400;
        res.end(JSON.stringify({ ok: false, error: e.message }));
      }
      return;
    }

    res.statusCode = 404;
    res.end(JSON.stringify({ error: "not found" }));
  });

  _server.listen(PORT, "127.0.0.1", () => {
    console.log("[BTCPC MCP] Tool server listening on 127.0.0.1:" + PORT +
      " | node=" + NODE_ID +
      " | tools=" + _listTools().map(t => t.name).join(","));
  });

  _server.on("error", err => {
    console.error("[BTCPC MCP] Server error:", err.message);
  });

  return _server;
}

function stop() {
  if (_server) { _server.close(); _server = null; }
}

function _readBody(req) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    req.on("data", c => chunks.push(c));
    req.on("end", () => resolve(Buffer.concat(chunks).toString("utf8")));
    req.on("error", reject);
  });
}

/**
 * Call a tool on the local MCP server (used by inference runtime).
 * Returns { output, trace_cid? } or throws.
 */
async function callTool(toolName, input) {
  return await _handleCall({ tool: toolName, input });
}

module.exports = { start, stop, callTool, listTools: _listTools, PORT };
