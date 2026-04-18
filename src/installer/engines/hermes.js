"use strict";

// Hermes Agent (Nous Research) adapter — https://github.com/nousresearch/hermes-agent
// Hermes runs as a CLI: `hermes` interactive session.
// We install our SOUL.md as a Hermes profile and register btcpc as a skill.

const { spawnSync, spawn } = require("child_process");
const path = require("path");
const fs = require("fs");
const os = require("os");

const SKILL_SRC = path.resolve(__dirname, "../skills/hermes-skill.json");
const HERMES_SKILLS_DIR = path.join(os.homedir(), ".hermes", "skills");

function isAvailable() {
  const r = spawnSync("hermes", ["--version"], { encoding: "utf8" });
  return r.status === 0;
}

function installSkill() {
  try {
    fs.mkdirSync(HERMES_SKILLS_DIR, { recursive: true });
    const dest = path.join(HERMES_SKILLS_DIR, "btcpc.json");
    fs.copyFileSync(SKILL_SRC, dest);
    console.log(`  ✓ BTCPC skill installed in Hermes at ${dest}`);
    console.log("  Try: hermes 'what is my btcpc balance'");
    return true;
  } catch (e) {
    console.warn(`  ⚠ Could not install Hermes skill: ${e.message}`);
    return false;
  }
}

function install() {
  console.log("\n  Installing Hermes Agent (Nous Research)...");
  const r = spawnSync("bash", ["-c", "curl -fsSL https://get.hermes.ai | bash"], {
    stdio: "inherit", timeout: 300000,
  });
  return r.status === 0;
}

// Launch an interactive Hermes session pre-loaded with our SOUL.md as context.
// Returns a child process handle for the session.
function startSession(soulPath) {
  const args = ["--profile", soulPath, "--skill", "btcpc"];
  return spawn("hermes", args, { stdio: "inherit" });
}

module.exports = { isAvailable, install, installSkill, startSession };
