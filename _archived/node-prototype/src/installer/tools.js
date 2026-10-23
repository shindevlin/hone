"use strict";

// Shared tool implementations used by all engine adapters.
// Each tool returns { ok: bool, result: any, error?: string }.

const os = require("os");
const fs = require("fs");
const path = require("path");
const { execSync, spawnSync } = require("child_process");
const readline = require("readline");

const REPO_DIR = path.resolve(__dirname, "..", "..");
const ENV_PATH = path.join(REPO_DIR, ".env");

// ─── Hardware ────────────────────────────────────────────────────

function check_hardware() {
  const totalRamGb = os.totalmem() / (1024 ** 3);
  const freeRamGb = os.freemem() / (1024 ** 3);
  const cpuModel = os.cpus()[0]?.model || "unknown";
  const cpuCount = os.cpus().length;
  const platform = os.platform();
  const arch = os.arch();

  let diskFreeGb = null;
  try {
    const df = execSync("df -BG / 2>/dev/null || df -BG $HOME 2>/dev/null", { encoding: "utf8" });
    const match = df.match(/(\d+)G\s+(\d+)G\s+(\d+)G/);
    if (match) diskFreeGb = parseInt(match[3]);
  } catch (_) {}

  let gpuInfo = null;
  try {
    const nv = spawnSync("nvidia-smi", ["--query-gpu=name,memory.total", "--format=csv,noheader"], { encoding: "utf8" });
    if (nv.status === 0 && nv.stdout.trim()) gpuInfo = nv.stdout.trim();
  } catch (_) {}
  if (!gpuInfo) {
    try {
      const roc = spawnSync("rocm-smi", ["--showmeminfo", "vram"], { encoding: "utf8" });
      if (roc.status === 0) gpuInfo = "AMD ROCm GPU detected";
    } catch (_) {}
  }

  const ollamaRunning = (() => {
    try {
      const r = spawnSync("curl", ["-sf", "http://localhost:11434/api/tags"], { encoding: "utf8", timeout: 2000 });
      return r.status === 0;
    } catch (_) { return false; }
  })();

  return {
    ok: true,
    result: { totalRamGb, freeRamGb, cpuModel, cpuCount, platform, arch, diskFreeGb, gpuInfo, ollamaRunning },
  };
}

// ─── Confirmation prompt ─────────────────────────────────────────

async function ask_confirm(message, defaultYes = false) {
  const prompt = `\n  ${message} [${defaultYes ? "Y/n" : "y/N"}] `;
  return new Promise((resolve) => {
    const rl = readline.createInterface({ input: process.stdin, output: process.stdout });
    rl.question(prompt, (answer) => {
      rl.close();
      const a = answer.trim().toLowerCase();
      if (a === "") resolve(defaultYes);
      else resolve(a === "y" || a === "yes");
    });
  });
}

async function ask_input(message, defaultVal = "") {
  const prompt = `\n  ${message}${defaultVal ? ` [${defaultVal}]` : ""}: `;
  return new Promise((resolve) => {
    const rl = readline.createInterface({ input: process.stdin, output: process.stdout });
    rl.question(prompt, (answer) => {
      rl.close();
      resolve(answer.trim() || defaultVal);
    });
  });
}

async function ask_choice(message, choices) {
  console.log(`\n  ${message}`);
  choices.forEach((c, i) => console.log(`    ${i + 1}. ${c.label}${c.desc ? " — " + c.desc : ""}`));
  const prompt = `\n  Enter number [1-${choices.length}]: `;
  return new Promise((resolve) => {
    const rl = readline.createInterface({ input: process.stdin, output: process.stdout });
    rl.question(prompt, (answer) => {
      rl.close();
      const n = parseInt(answer.trim());
      if (n >= 1 && n <= choices.length) resolve(choices[n - 1]);
      else resolve(null);
    });
  });
}

// ─── Run a shell step with confirmation ─────────────────────────

async function run_step(description, cmd, opts = {}) {
  if (!opts.skipConfirm) {
    const ok = await ask_confirm(`  About to: ${description}. Proceed?`, true);
    if (!ok) return { ok: false, result: "skipped" };
  }
  console.log(`  → ${cmd}`);
  try {
    const out = execSync(cmd, { encoding: "utf8", stdio: ["inherit", "pipe", "pipe"], timeout: opts.timeout || 120000 });
    if (out && out.trim()) console.log("  " + out.trim().split("\n").join("\n  "));
    return { ok: true, result: out };
  } catch (e) {
    console.error(`  ✗ Failed: ${e.message}`);
    return { ok: false, error: e.message };
  }
}

// ─── Write .env key ──────────────────────────────────────────────

function write_config(key, value, explanation) {
  let content = "";
  try { content = fs.readFileSync(ENV_PATH, "utf8"); } catch (_) {}
  const re = new RegExp(`^${key}=.*$`, "m");
  const line = `${key}=${value}`;
  if (re.test(content)) content = content.replace(re, line);
  else content += (content.endsWith("\n") || content === "" ? "" : "\n") + line + "\n";
  fs.writeFileSync(ENV_PATH, content, { mode: 0o600 });
  if (explanation) console.log(`  ✓ Set ${key} — ${explanation}`);
  return { ok: true, result: { key, value } };
}

// ─── Verify a node process is alive ─────────────────────────────

function verify_running(bin) {
  try {
    const r = spawnSync("pgrep", ["-f", bin], { encoding: "utf8" });
    return { ok: r.status === 0, result: r.stdout.trim() };
  } catch (_) {
    return { ok: false };
  }
}

// ─── Detect which agentic engines are installed ──────────────────

function detect_engines() {
  const found = [];

  const bins = [
    { id: "hermes",    cmd: "hermes",    version_arg: "--version",  label: "Hermes Agent (Nous Research)" },
    { id: "openclaw",  cmd: "openclaw",  version_arg: "--version",  label: "OpenClaw" },
    { id: "crewai",    cmd: "crewai",    version_arg: "--version",  label: "CrewAI (Python)" },
    { id: "autogen",   cmd: "python3",   check: "import autogen",   label: "AutoGen (Microsoft)" },
    { id: "langgraph", cmd: "python3",   check: "import langgraph", label: "LangGraph" },
    { id: "n8n",       cmd: "n8n",       version_arg: "--version",  label: "n8n" },
    { id: "flowise",   cmd: "flowise",   version_arg: "--version",  label: "Flowise" },
    { id: "dify",      port: 3000,       label: "Dify" },
  ];

  for (const e of bins) {
    try {
      if (e.check) {
        const r = spawnSync("python3", ["-c", e.check], { encoding: "utf8" });
        if (r.status === 0) found.push({ id: e.id, label: e.label });
      } else if (e.port) {
        const r = spawnSync("curl", ["-sf", "--connect-timeout", "1", `http://localhost:${e.port}/health`], { encoding: "utf8" });
        if (r.status === 0) found.push({ id: e.id, label: e.label });
      } else {
        const r = spawnSync(e.cmd, [e.version_arg || "--version"], { encoding: "utf8" });
        if (r.status === 0) found.push({ id: e.id, label: e.label });
      }
    } catch (_) {}
  }

  return { ok: true, result: found };
}

module.exports = {
  check_hardware,
  ask_confirm,
  ask_input,
  ask_choice,
  run_step,
  write_config,
  verify_running,
  detect_engines,
};
