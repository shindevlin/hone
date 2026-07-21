"use strict";

/**
 * HONE Tool Registry
 * Shin Devlin
 *
 * Tracks tool capabilities declared by mining nodes.
 * Tools come in three kinds:
 *   cli   — shell command exposed as a tool (sandboxed, no network writes)
 *   mcp   — MCP server endpoint the node runs locally
 *   builtin — deterministic tools baked into the runtime (calc, hash, hone_fs_read)
 *
 * On-chain registration: TOOL_CAPABILITY_REGISTER ledger entry.
 * In-memory: this module holds the live index, hydrated from replay.
 *
 * Work value multiplier per tool kind:
 *   builtin : 1.10  (cheap, deterministic)
 *   cli     : 1.25  (local compute, sandboxed)
 *   mcp     : 1.50  (full MCP call round-trip)
 *
 * Tool traces: non-deterministic tool results (web_fetch, sensor_read) must be
 * committed as HONE-FS blobs before the proof is submitted. The proof carries
 * tool_trace_hash = sha256(JSON.stringify(toolCallLog)) so verifiers can check
 * the trace CID on-chain without re-running the fetch.
 */

const crypto = require("crypto");

// Deterministic tools — no external network needed; verifiers can replay exactly
const BUILTIN_TOOLS = {
  calculator: {
    name: "calculator",
    kind: "builtin",
    description: "Evaluate a math expression. Input: { expr: string }",
    deterministic: true,
    multiplier: 1.10,
  },
  hash: {
    name: "hash",
    kind: "builtin",
    description: "SHA256 of a string. Input: { input: string }",
    deterministic: true,
    multiplier: 1.10,
  },
  hone_fs_read: {
    name: "hone_fs_read",
    kind: "builtin",
    description: "Read a pinned HONE-FS blob by CID. Input: { cid: string }",
    deterministic: true, // CID is content-addressed
    multiplier: 1.10,
  },
  epoch_info: {
    name: "epoch_info",
    kind: "builtin",
    description: "Current epoch number, block reward, and timestamp.",
    deterministic: true,
    multiplier: 1.10,
  },
};

// Work value multipliers by tool kind
const TOOL_MULTIPLIER = {
  builtin: 1.10,
  cli: 1.25,
  mcp: 1.50,
};

// In-memory tool registry
// Map<nodeId, { node_id, tools: Map<toolName, ToolRecord>, registered_epoch, stake }>
const nodeTools = new Map();

// Map<toolName, ToolRecord> for builtins (always available)
const builtinIndex = new Map(Object.entries(BUILTIN_TOOLS));

// ── Registration ──────────────────────────────────────────────────────────────

/**
 * Register tool capabilities for a node from a TOOL_CAPABILITY_REGISTER ledger entry.
 */
function registerFromEntry(entry) {
  if (!entry || entry.type !== "TOOL_CAPABILITY_REGISTER") return;
  const nodeId = entry.from;
  if (!nodeId) return;

  const data = entry.tool_data || {};
  const tools = (data.tools || []).map(t => ({
    name: t.name,
    kind: t.kind || "cli",
    description: t.description || "",
    command: t.command || null,       // CLI: shell command template
    mcp_url: t.mcp_url || null,       // MCP: local server URL (e.g. http://localhost:3100)
    deterministic: t.deterministic === true,
    multiplier: TOOL_MULTIPLIER[t.kind || "cli"] || 1.25,
    registered_epoch: entry.epoch || 0,
  }));

  const existing = nodeTools.get(nodeId) || { node_id: nodeId, tools: new Map(), registered_epoch: entry.epoch || 0 };
  for (const tool of tools) {
    existing.tools.set(tool.name, tool);
  }
  nodeTools.set(nodeId, existing);
}

/**
 * Compute the composite work_value multiplier for a given set of tool names
 * used in a proof. Uses the highest-multiplier tool that was actually called.
 */
function computeToolMultiplier(toolNamesUsed) {
  if (!toolNamesUsed || toolNamesUsed.length === 0) return 1.0;

  let max = 1.0;
  for (const name of toolNamesUsed) {
    const builtin = builtinIndex.get(name);
    if (builtin) { max = Math.max(max, builtin.multiplier); continue; }
    // Check all registered nodes for this tool
    for (const [, node] of nodeTools) {
      const tool = node.tools.get(name);
      if (tool) { max = Math.max(max, tool.multiplier); break; }
    }
  }
  return max;
}

/**
 * Hash a tool call log for inclusion in a compute proof.
 * toolCallLog: [{ tool: string, input: any, output: any, trace_cid?: string }]
 */
function hashToolTrace(toolCallLog) {
  const canonical = JSON.stringify(
    (toolCallLog || []).map(c => ({
      tool: c.tool,
      input: c.input,
      // Non-deterministic tools use trace_cid instead of raw output
      output: c.trace_cid ? { cid: c.trace_cid } : c.output,
    }))
  );
  return crypto.createHash("sha256").update(canonical).digest("hex");
}

// ── Queries ───────────────────────────────────────────────────────────────────

function getNodeTools(nodeId) {
  const node = nodeTools.get(nodeId);
  if (!node) return [];
  return Array.from(node.tools.values());
}

function getAllRegisteredNodes() {
  return Array.from(nodeTools.values()).map(n => ({
    node_id: n.node_id,
    tools: Array.from(n.tools.values()),
    registered_epoch: n.registered_epoch,
  }));
}

function getBuiltins() {
  return Array.from(builtinIndex.values());
}

function getAllAvailableTools() {
  const result = new Map(builtinIndex);
  for (const [, node] of nodeTools) {
    for (const [name, tool] of node.tools) {
      if (!result.has(name)) result.set(name, tool);
    }
  }
  return Array.from(result.values());
}

/**
 * Find nodes that support a given set of tool names.
 */
function findCapableNodes(requiredTools) {
  const required = new Set(requiredTools || []);
  if (required.size === 0) return [];

  return Array.from(nodeTools.values()).filter(node => {
    for (const name of required) {
      if (!node.tools.has(name) && !builtinIndex.has(name)) return false;
    }
    return true;
  }).map(n => n.node_id);
}

module.exports = {
  registerFromEntry,
  computeToolMultiplier,
  hashToolTrace,
  getNodeTools,
  getAllRegisteredNodes,
  getBuiltins,
  getAllAvailableTools,
  findCapableNodes,
  BUILTIN_TOOLS,
  TOOL_MULTIPLIER,
};
