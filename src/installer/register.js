"use strict";

// Registers the BTCPC skill/plugin into whichever agentic engines are present.
// Called at the end of every install run so the user can talk to their BTCPC
// node through whatever AI assistant they already use.

const hermes   = require("./engines/hermes");
const openclaw = require("./engines/openclaw");
const { spawnSync } = require("child_process");
const path = require("path");
const fs   = require("fs");
const os   = require("os");

// Engines where we know how to register a skill/command
const ADAPTERS = [
  {
    id: "zeroclaw",
    label: "ZeroClaw",
    check: () => spawnSync(process.env.HOME + "/.cargo/bin/zeroclaw", ["--version"], { encoding: "utf8" }).status === 0
               || spawnSync("zeroclaw", ["--version"], { encoding: "utf8" }).status === 0,
    register: () => _installZeroClawSkill(),
    hint: "zeroclaw agent  →  'show my btcpc balance'",
  },
  {
    id: "hermes",
    label: "Hermes Agent",
    check: () => hermes.isAvailable(),
    register: () => hermes.installSkill(),
    hint: "hermes 'show my btcpc balance'",
  },
  {
    id: "openclaw",
    label: "OpenClaw",
    check: () => openclaw.isAvailable(),
    register: () => openclaw.installSkill(),
    hint: "openclaw 'show my btcpc balance'",
  },
  {
    id: "crewai",
    label: "CrewAI",
    check: () => spawnSync("python3", ["-c", "import crewai"], { encoding: "utf8" }).status === 0,
    register: () => _writeCrewAITool(),
    hint: "See ~/.crewai/tools/btcpc_tool.py",
  },
  {
    id: "autogen",
    label: "AutoGen",
    check: () => spawnSync("python3", ["-c", "import autogen"], { encoding: "utf8" }).status === 0,
    register: () => _writeAutoGenTool(),
    hint: "See ~/.autogen/tools/btcpc.py",
  },
];

function registerAll() {
  const registered = [];
  for (const adapter of ADAPTERS) {
    try {
      if (adapter.check()) {
        const ok = adapter.register();
        if (ok) {
          registered.push(adapter);
          console.log(`  ✓ ${adapter.label}: ${adapter.hint}`);
        }
      }
    } catch (_) {}
  }
  return registered;
}

// ─── Python tool stubs ───────────────────────────────────────────

function _writeCrewAITool() {
  const dir = path.join(os.homedir(), ".crewai", "tools");
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, "btcpc_tool.py"), _crewaiPy(), "utf8");
  return true;
}

function _writeAutoGenTool() {
  const dir = path.join(os.homedir(), ".autogen", "tools");
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, "btcpc.py"), _autogenPy(), "utf8");
  return true;
}

function _installZeroClawSkill() {
  const skillSrc = path.resolve(__dirname, "skills/zeroclaw-skill");
  const zc = process.env.HOME + "/.cargo/bin/zeroclaw";
  const bin = fs.existsSync(zc) ? zc : "zeroclaw";
  const r = spawnSync(bin, ["skills", "install", skillSrc], { encoding: "utf8" });
  if (r.status === 0) {
    console.log("  ✓ BTCPC skill installed in ZeroClaw");
    console.log("  Try: zeroclaw agent  then ask about your balance");
    return true;
  }
  console.warn("  ⚠ ZeroClaw skill install failed:", r.stderr?.trim());
  return false;
}

function _crewaiPy() {
  const btcpcDir = path.resolve(__dirname, "../..");
  return `from crewai_tools import tool
import subprocess, json

BTCPC_DIR = "${btcpcDir}"

@tool("btcpc_balance")
def btcpc_balance(account: str = "") -> str:
    """Show BTCPC token balance for an account."""
    cmd = ["node", f"{BTCPC_DIR}/bin/btcpc-cli", "balance"] + ([account] if account else [])
    return subprocess.check_output(cmd, text=True)

@tool("btcpc_status")
def btcpc_status() -> str:
    """Show status of BTCPC nodes running on this device."""
    return subprocess.check_output(["node", f"{BTCPC_DIR}/bin/btcpc-cli", "status"], text=True)

@tool("btcpc_send")
def btcpc_send(to: str, amount: float) -> str:
    """Send BTCPC tokens to another account."""
    return subprocess.check_output(
        ["node", f"{BTCPC_DIR}/bin/btcpc-cli", "transfer", to, str(amount)], text=True
    )
`;
}

function _autogenPy() {
  const btcpcDir = path.resolve(__dirname, "../..");
  return `import subprocess

BTCPC_DIR = "${btcpcDir}"

def btcpc_balance(account: str = "") -> str:
    """Show BTCPC token balance."""
    cmd = ["node", f"{BTCPC_DIR}/bin/btcpc-cli", "balance"] + ([account] if account else [])
    return subprocess.check_output(cmd, text=True)

def btcpc_status() -> str:
    """Show BTCPC node status."""
    return subprocess.check_output(["node", f"{BTCPC_DIR}/bin/btcpc-cli", "status"], text=True)

def btcpc_send(to: str, amount: float) -> str:
    """Send BTCPC tokens."""
    return subprocess.check_output(
        ["node", f"{BTCPC_DIR}/bin/btcpc-cli", "transfer", to, str(amount)], text=True
    )

# Register with AutoGen function map:
# function_map = {"btcpc_balance": btcpc_balance, "btcpc_status": btcpc_status, "btcpc_send": btcpc_send}
`;
}

module.exports = { registerAll };
