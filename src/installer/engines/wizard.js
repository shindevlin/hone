"use strict";

// Plain readline wizard — no AI required. Always available as fallback.

const { NODE_TYPES } = require("../nodes");
const tools = require("../tools");
const { registerSkill } = require("../register");

const ORANGE = "\x1b[38;5;208m";
const GREEN  = "\x1b[32m";
const BOLD   = "\x1b[1m";
const RESET  = "\x1b[0m";

function say(msg) { console.log(`${ORANGE}[btcpc]${RESET} ${msg}`); }
function ok(msg)  { console.log(`${GREEN}✓${RESET} ${msg}`); }
function hr()     { console.log("\n" + "─".repeat(60)); }

async function run() {
  hr();
  console.log(`${BOLD}  Bitcoin Proof of Compute — Node Setup${RESET}`);
  console.log("  btcpc.net  •  sovereign chain for AI inference\n");
  say("Let's figure out what you'd like to run on this device.");
  say("I'll ask about each node type. You choose what fits your hardware.\n");

  const hw = tools.check_hardware().result;
  say(`Detected: ${hw.platform} ${hw.arch}, ${hw.totalRamGb.toFixed(1)} GB RAM` +
      (hw.gpuInfo ? `, GPU: ${hw.gpuInfo}` : ", no GPU detected") +
      (hw.diskFreeGb ? `, ${hw.diskFreeGb} GB disk free` : ""));

  let username = await tools.ask_input(
    "Choose a username for this node (3-20 chars, lowercase letters/numbers/hyphens)",
    "node-" + require("os").hostname().toLowerCase().replace(/[^a-z0-9]/g, "").slice(0, 10)
  );
  username = username.toLowerCase().replace(/[^a-z0-9-]/g, "").slice(0, 20);
  tools.write_config("BTCPC_MINER", username, "your node identity on the chain");

  const chosen = [];

  for (const node of NODE_TYPES) {
    hr();
    console.log(`\n${BOLD}  ${node.name}${RESET}`);
    for (const line of node.explain) say(line);

    const meetsReqs = _meetsRequirements(hw, node.requires);
    if (!meetsReqs.ok) {
      say(`Skipping — your device doesn't meet the minimum for this node: ${meetsReqs.reason}`);
      continue;
    }

    const want = await tools.ask_confirm(node.ask);
    if (!want) { say("Got it, skipping.\n"); continue; }

    const success = await _setupNode(node, hw, username);
    if (success) {
      chosen.push(node);
      ok(`${node.name} configured.`);
      if (node.systemd) {
        const autostart = await tools.ask_confirm(
          `Start ${node.name} automatically every time this device reboots?`, true
        );
        if (autostart) {
          const { execSync } = require("child_process");
          try {
            execSync(`systemctl --user enable ${node.systemd}`, { stdio: "pipe" });
            ok(`${node.name} will autostart on reboot.`);
          } catch (e) {
            say(`Could not enable autostart: ${e.message}`);
          }
        }
      }
      console.log();
    }
  }

  hr();
  if (chosen.length === 0) {
    say("No nodes configured. Run btcpc-install again whenever you're ready.");
    return;
  }

  console.log(`\n${BOLD}  Setup complete!${RESET}`);
  say(`Configured: ${chosen.map(n => n.name).join(", ")}`);
  say("Check status any time:  node bin/btcpc-cli status");
  say("View miner logs:        journalctl --user -u btcpc-miner -n 50\n");
}

function _meetsRequirements(hw, requires) {
  if (requires.ram_gb && hw.totalRamGb < requires.ram_gb) {
    return { ok: false, reason: `needs ${requires.ram_gb} GB RAM, device has ${hw.totalRamGb.toFixed(1)} GB` };
  }
  if (requires.disk_gb && hw.diskFreeGb !== null && hw.diskFreeGb < requires.disk_gb) {
    return { ok: false, reason: `needs ${requires.disk_gb} GB free disk, device has ${hw.diskFreeGb} GB` };
  }
  return { ok: true };
}

async function _setupNode(node, hw, username) {
  const steps = _stepsFor(node, hw, username);
  for (const step of steps) {
    const r = await tools.run_step(step.desc, step.cmd);
    if (!r.ok && step.required !== false) {
      const retry = await tools.ask_confirm(`That step failed. Skip ${node.name} setup?`, false);
      return !retry;
    }
  }
  return true;
}

function _stepsFor(node, hw, username) {
  const repoDir = require("path").resolve(__dirname, "../../..");

  if (node.id === "inference") {
    const steps = [];
    if (!hw.ollamaRunning) {
      steps.push({
        desc: "install Ollama (the AI model runner BTCPC uses for inference)",
        cmd: "curl -fsSL https://ollama.com/install.sh | sh",
      });
      steps.push({ desc: "start Ollama service", cmd: "ollama serve &>/dev/null &" });
    }
    const model = hw.gpuInfo ? "qwen3:8b" : "qwen3:4b";
    steps.push({ desc: `pull ${model} as your default mining model`, cmd: `ollama pull ${model}` });
    tools.write_config("BTCPC_MODEL", model, "default inference model");
    steps.push({
      desc: "install the miner as a systemd user service",
      cmd: `cp ${repoDir}/etc/btcpc-miner.service ~/.config/systemd/user/ && systemctl --user daemon-reload && systemctl --user enable --now btcpc-miner`,
      required: false,
    });
    return steps;
  }

  if (node.id === "clock") {
    tools.write_config("BTCPC_CLOCK_ACCOUNT", username, "your clock node identity");
    return [{
      desc: "install the clock node as a systemd user service",
      cmd: `cp ${repoDir}/etc/btcpc-clock.service ~/.config/systemd/user/ && systemctl --user daemon-reload && systemctl --user enable --now btcpc-clock`,
      required: false,
    }];
  }

  if (node.id === "storage") {
    return [
      { desc: "create blob storage directory at ~/.btcpc/blobs", cmd: "mkdir -p ~/.btcpc/blobs", required: false },
      {
        desc: "install the storage daemon as a systemd user service",
        cmd: `cp ${repoDir}/etc/btcpc-storage.service ~/.config/systemd/user/ && systemctl --user daemon-reload && systemctl --user enable --now btcpc-storage`,
        required: false,
      },
    ];
  }

  if (node.id === "verifier") {
    tools.write_config("BTCPC_ROLES", "verifier", "enables verifier role");
    return [];
  }

  if (node.id === "sensor") {
    return [
      { desc: "install sensor bridge (auto-detects GPS, Meshtastic, Flipper)", cmd: `node ${repoDir}/bin/btcpc-gnss-bridge --auto`, required: false },
    ];
  }

  return [];
}

module.exports = { run };
