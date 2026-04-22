"use strict";

/**
 * BTCPC Tool Executor
 * Shin Devlin
 *
 * Shared utility for requester-side tool execution across all BTCPC engines:
 *   - Inference / Mining  (enrich prompt context before sending to miner)
 *   - Agent turns         (tool calls within a session)
 *   - Sensor readings     (enrich readings: geocode, unit-convert, validate)
 *   - Storage             (hash/process data before committing to BTCPC-FS)
 *   - Verifier            (re-run deterministic tools to check tool_trace_hash)
 *   - Gateway             (transform/route sensor data)
 *
 * BTCPC builtins are called in-process (no HTTP, no SSRF).
 * External MCP servers are called via JSON-RPC HTTP (user-hosted only —
 * private/loopback addresses are blocked).
 *
 * Usage:
 *   const { executeTools } = require('./mcp/toolExecutor');
 *   const result = await executeTools({
 *     tools: ['calculator', 'web_fetch'],   // tool names to call
 *     toolContext: { expr: '2+2' },         // arguments for builtin tools
 *     mcpServers: [{ url, tools }],         // optional user-provided MCP servers
 *     savedMcpServers: [],                  // optional saved from user profile
 *   });
 *   // result: { results, toolTraceHash, toolsUsed, contextText }
 */

const axios = require("axios").default;
const { hashToolTrace } = require("./toolRegistry");
const mcpServer = require("./btcpcMcpServer");
const { isPublicHttpUrl } = require("../services/urlSafety");

const BUILTIN_TOOLS = new Set([
  "calculator", "hash", "btcpc_fs_read", "epoch_info", "web_fetch",
]);

const SSRF_BLOCKED = [
  "localhost", "127.0.0.1", "0.0.0.0", "::1",
  "169.254", "10.", "192.168.", "172.16.", "172.17.",
];

/**
 * Execute a set of tools on the requester's machine and return enriched results.
 *
 * @param {object} opts
 * @param {string[]}  opts.tools             — tool names to call
 * @param {object}    opts.toolContext        — arguments passed to each tool (shared)
 * @param {object[]}  [opts.mcpServers]       — [{ url, tools: string[] }]
 * @param {object[]}  [opts.savedMcpServers]  — same format, from user profile
 *
 * @returns {Promise<{
 *   results: ToolResult[],     — [{ tool, output, kind, error? }]
 *   toolTraceHash: string|null, — sha256 of the call log (null if no tools)
 *   toolsUsed: string[],        — names of tools that were called
 *   contextText: string,        — formatted text ready to inject into a prompt/entry
 * }>}
 */
async function executeTools({ tools, toolContext, mcpServers, savedMcpServers } = {}) {
  const toolNames = Array.isArray(tools) ? tools : [];
  const ctx = toolContext || {};

  if (toolNames.length === 0) {
    return { results: [], toolTraceHash: null, toolsUsed: [], contextText: "" };
  }

  // Split into builtins vs external
  const builtinNames = toolNames.filter(t => BUILTIN_TOOLS.has(t));
  const externalNames = toolNames.filter(t => !BUILTIN_TOOLS.has(t));

  // ── 1. BTCPC builtins — in-process, no HTTP ──────────────────────────────
  const builtinPromises = builtinNames.map(async name => {
    try {
      const output = await mcpServer.callTool(name, ctx);
      return { tool: name, output, kind: "builtin" };
    } catch (e) {
      return { tool: name, output: null, error: e.message, kind: "builtin" };
    }
  });

  // ── 2. External MCP servers — SSRF-safe ──────────────────────────────────
  const allServers = await _mergeServers(mcpServers, savedMcpServers);

  const externalPromises = externalNames.map(async name => {
    const server = allServers.find(s => s.tools && s.tools.includes(name));
    if (!server) {
      return { tool: name, output: null, error: "no server registered for tool", kind: "mcp" };
    }
    try {
      const r = await axios.post(server.url, {
        jsonrpc: "2.0",
        method: "tools/call",
        params: { name, arguments: ctx },
        id: Math.random().toString(36).slice(2),
      }, { timeout: 15000 });
      const output = r.data?.result?.content?.[0]?.text || JSON.stringify(r.data?.result || r.data);
      return { tool: name, output, kind: "mcp", server: server.url };
    } catch (e) {
      return { tool: name, output: null, error: e.message, kind: "mcp" };
    }
  });

  const results = await Promise.all([...builtinPromises, ...externalPromises]);
  const toolsUsed = results.map(r => r.tool);

  // ── 3. Hash the trace ────────────────────────────────────────────────────
  const toolTraceHash = hashToolTrace(results.map(r => ({
    tool: r.tool,
    input: ctx,
    output: r.output,
    trace_cid: r.output && r.output.trace_cid ? r.output.trace_cid : undefined,
  })));

  // ── 4. Format context text ────────────────────────────────────────────────
  const contextText = results.map(r => {
    const text = r.error
      ? `[Error: ${r.error}]`
      : typeof r.output === "string" ? r.output : JSON.stringify(r.output);
    return `[Tool: ${r.tool}]\n${text}`;
  }).join("\n\n");

  return { results, toolTraceHash, toolsUsed, contextText };
}

/**
 * Verify a tool_trace_hash by re-running all deterministic tools and
 * accepting the committed CID for non-deterministic ones.
 * Returns { valid: bool, reason: string }
 */
async function verifyToolTrace(toolTraceHash, toolCallLog) {
  if (!toolTraceHash || !toolCallLog) return { valid: false, reason: "missing_inputs" };

  // Re-run deterministic tools; accept trace_cid for non-deterministic
  const replayLog = await Promise.all(toolCallLog.map(async call => {
    if (call.trace_cid) {
      // Non-deterministic — accept the CID as committed truth
      return { tool: call.tool, input: call.input, output: { cid: call.trace_cid }, trace_cid: call.trace_cid };
    }
    // Deterministic — replay and compare
    try {
      const output = await mcpServer.callTool(call.tool, call.input || {});
      return { tool: call.tool, input: call.input, output };
    } catch (e) {
      return { tool: call.tool, input: call.input, output: { error: e.message } };
    }
  }));

  const replayHash = hashToolTrace(replayLog);
  if (replayHash === toolTraceHash) return { valid: true, reason: "hash_match" };
  return { valid: false, reason: "hash_mismatch", expected: toolTraceHash, got: replayHash };
}

// ── Helpers ───────────────────────────────────────────────────────────────────

async function _mergeServers(inline, saved) {
  const result = [];

  for (const list of [inline, saved]) {
    for (const s of (list || [])) {
      if (!s || !s.url) continue;
      try {
        const parsed = new URL(s.url);
        if (!["http:", "https:"].includes(parsed.protocol)) continue;
        if (SSRF_BLOCKED.some(b => parsed.hostname.startsWith(b) || parsed.hostname === b)) continue;
        if (!await isPublicHttpUrl(s.url)) continue;
        result.push(s);
      } catch (_) {}
    }
  }

  return result;
}

module.exports = { executeTools, verifyToolTrace, BUILTIN_TOOLS };
