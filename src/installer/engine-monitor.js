"use strict";

// Tracks which agentic engines are installed and how they're performing
// within the HONE ecosystem. Stored in ~/.hone/engine-stats.json.
// Surfaced in hone-cli status and the explorer dashboard.

const fs   = require("fs");
const path = require("path");
const os   = require("os");
const { spawnSync } = require("child_process");

const STATS_PATH = path.join(os.homedir(), ".hone", "engine-stats.json");

const KNOWN_ENGINES = [
  { id: "hermes",          label: "Hermes Agent",      check: () => _binExists("hermes") },
  { id: "openclaw",        label: "OpenClaw",           check: () => _binExists("openclaw") },
  { id: "crewai",          label: "CrewAI",             check: () => _pyModule("crewai") },
  { id: "autogen",         label: "AutoGen",            check: () => _pyModule("autogen") },
  { id: "langgraph",       label: "LangGraph",          check: () => _pyModule("langgraph") },
  { id: "n8n",             label: "n8n",                check: () => _binExists("n8n") },
  { id: "flowise",         label: "Flowise",            check: () => _binExists("flowise") },
  { id: "dify",            label: "Dify",               check: () => _httpAlive("localhost", 3000) },
  { id: "llamaindex",      label: "LlamaIndex",         check: () => _pyModule("llama_index") },
  { id: "semantic-kernel", label: "Semantic Kernel",    check: () => _pyModule("semantic_kernel") },
];

function _binExists(cmd) {
  return spawnSync("which", [cmd], { encoding: "utf8" }).status === 0;
}

function _pyModule(mod) {
  return spawnSync("python3", ["-c", `import ${mod}`], { encoding: "utf8" }).status === 0;
}

function _httpAlive(host, port) {
  const r = spawnSync("curl", ["-sf", "--connect-timeout", "1", `http://${host}:${port}/health`], { encoding: "utf8" });
  return r.status === 0;
}

function _load() {
  try { return JSON.parse(fs.readFileSync(STATS_PATH, "utf8")); }
  catch (_) { return {}; }
}

function _save(stats) {
  try {
    fs.mkdirSync(path.dirname(STATS_PATH), { recursive: true });
    fs.writeFileSync(STATS_PATH, JSON.stringify(stats, null, 2), { mode: 0o600 });
  } catch (_) {}
}

// Scan and update engine stats. Called on hone-cli status and at startup.
function scan() {
  const stats = _load();
  const now = Date.now();

  for (const engine of KNOWN_ENGINES) {
    const present = engine.check();
    if (!stats[engine.id]) stats[engine.id] = { label: engine.label, installs: 0, last_seen: null, skill_registered: false };

    if (present) {
      stats[engine.id].present = true;
      stats[engine.id].last_seen = new Date(now).toISOString();
    } else {
      stats[engine.id].present = false;
    }
  }

  stats._scanned_at = new Date(now).toISOString();
  _save(stats);
  return stats;
}

// Record that a skill was successfully registered in an engine
function recordSkillRegistered(engineId) {
  const stats = _load();
  if (!stats[engineId]) stats[engineId] = {};
  stats[engineId].skill_registered = true;
  stats[engineId].skill_registered_at = new Date().toISOString();
  _save(stats);
}

// Record a command invocation through an engine (called from hone-cli)
function recordInvocation(engineId, command, ok) {
  const stats = _load();
  if (!stats[engineId]) stats[engineId] = {};
  if (!stats[engineId].invocations) stats[engineId].invocations = {};
  const inv = stats[engineId].invocations;
  inv.total = (inv.total || 0) + 1;
  inv.ok    = (inv.ok    || 0) + (ok ? 1 : 0);
  inv.fail  = (inv.fail  || 0) + (ok ? 0 : 1);
  inv.last  = new Date().toISOString();
  if (command) inv.last_command = command;
  _save(stats);
}

// Return a summary for hone-cli status output
function summary() {
  const stats = _load();
  const results = [];
  for (const engine of KNOWN_ENGINES) {
    const s = stats[engine.id];
    if (!s || !s.present) continue;
    const inv = s.invocations || {};
    results.push({
      label: engine.label,
      skill_registered: s.skill_registered || false,
      invocations: inv.total || 0,
      success_rate: inv.total ? Math.round((inv.ok / inv.total) * 100) : null,
      last_used: inv.last || s.last_seen || null,
    });
  }
  return results;
}

module.exports = { scan, recordSkillRegistered, recordInvocation, summary, KNOWN_ENGINES };
