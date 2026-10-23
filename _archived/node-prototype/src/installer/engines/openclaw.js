"use strict";

// OpenClaw adapter — https://github.com/openclaw/openclaw
// OpenClaw uses a SOUL.md personality file and HTTP REST for tool calls.
// Default port: 3333. Skill is a Markdown file placed in ~/.openclaw/skills/.

const { spawnSync } = require("child_process");
const path = require("path");
const fs = require("fs");
const os = require("os");
const http = require("http");

const SKILL_SRC = path.resolve(__dirname, "../skills/openclaw-skill.md");
const SOUL_SRC  = path.resolve(__dirname, "../SOUL.md");
const OC_DIR    = path.join(os.homedir(), ".openclaw");
const OC_SKILLS = path.join(OC_DIR, "skills");

function isAvailable() {
  const r = spawnSync("openclaw", ["--version"], { encoding: "utf8" });
  return r.status === 0;
}

function installSkill() {
  try {
    fs.mkdirSync(OC_SKILLS, { recursive: true });

    // HONE tool skill
    const skillDest = path.join(OC_SKILLS, "hone.md");
    fs.copyFileSync(SKILL_SRC, skillDest);

    // SOUL.md as setup profile
    const soulDest = path.join(OC_DIR, "hone-setup-soul.md");
    fs.copyFileSync(SOUL_SRC, soulDest);

    console.log(`  ✓ HONE skill installed in OpenClaw at ${skillDest}`);
    console.log("  Try: openclaw 'what is my hone balance'");
    return true;
  } catch (e) {
    console.warn(`  ⚠ Could not install OpenClaw skill: ${e.message}`);
    return false;
  }
}

function install() {
  console.log("\n  Installing OpenClaw...");
  const r = spawnSync("bash", ["-c", "curl -fsSL https://get.openclaw.ai | bash"], {
    stdio: "inherit", timeout: 300000,
  });
  return r.status === 0;
}

// Send a message to a running OpenClaw HTTP gateway and stream the response.
async function send(message, port = 3333) {
  return new Promise((resolve, reject) => {
    const body = JSON.stringify({ message, skill: "hone" });
    const req = http.request({
      hostname: "localhost", port, path: "/chat",
      method: "POST",
      headers: { "Content-Type": "application/json", "Content-Length": Buffer.byteLength(body) },
    }, (res) => {
      let out = "";
      res.on("data", (d) => { out += d; process.stdout.write(d); });
      res.on("end", () => resolve(out));
    });
    req.on("error", reject);
    req.write(body);
    req.end();
  });
}

module.exports = { isAvailable, install, installSkill, send };
