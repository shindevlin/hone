"use strict";

// BTCPC Intelligent Installer
// Detects agentic engines, offers to install one if none found,
// then guides the user through node type selection via that engine
// (or plain wizard fallback). Registers BTCPC as a skill when done.

const tools   = require("./tools");
const { registerAll } = require("./register");
const wizard  = require("./engines/wizard");
const hermes  = require("./engines/hermes");
const openclaw = require("./engines/openclaw");

const ORANGE = "\x1b[38;5;208m";
const BOLD   = "\x1b[1m";
const RESET  = "\x1b[0m";
const DIM    = "\x1b[2m";

function say(msg) { console.log(`${ORANGE}[btcpc]${RESET} ${msg}`); }

// All engines we know about, shown as install options when nothing is detected.
// Three are shown upfront; user can pick "more" to see the full list.
const ALL_ENGINES = [
  {
    id: "zeroclaw",
    label: "ZeroClaw",
    desc: "Rust — fastest, smallest, hardware integration, local-first",
    install: () => _shellInstall("curl -fsSL https://get.zeroclaw.ai | bash"),
    homepage: "https://zeroclaw.ai",
    featured: true,
  },
  {
    id: "hermes",
    label: "Hermes Agent",
    desc: "Nous Research — self-improving, 40+ tools, any device",
    install: () => hermes.install(),
    homepage: "https://github.com/nousresearch/hermes-agent",
    featured: true,
  },
  {
    id: "openclaw",
    label: "OpenClaw",
    desc: "Lightweight, SOUL.md personality, any LLM backend",
    install: () => openclaw.install(),
    homepage: "https://github.com/openclaw/openclaw",
    featured: true,
  },
  {
    id: "autogen",
    label: "AutoGen",
    desc: "Microsoft — agent-to-agent dialogue, code execution",
    install: () => _pipInstall("pyautogen"),
    homepage: "https://github.com/microsoft/autogen",
  },
  {
    id: "langgraph",
    label: "LangGraph",
    desc: "Graph-based agents, fine-grained control, Python",
    install: () => _pipInstall("langgraph"),
    homepage: "https://langchain-ai.github.io/langgraph",
  },
  {
    id: "n8n",
    label: "n8n",
    desc: "Visual workflow automation, low-code, self-hosted",
    install: () => _npmInstall("n8n"),
    homepage: "https://n8n.io",
  },
  {
    id: "flowise",
    label: "Flowise",
    desc: "Visual LLM pipeline builder, Ollama native",
    install: () => _npmInstall("flowise"),
    homepage: "https://flowiseai.com",
  },
  {
    id: "dify",
    label: "Dify",
    desc: "Low-code AI app builder, Docker, all providers",
    install: () => _dockerInstall("langgenius/dify"),
    homepage: "https://dify.ai",
  },
  {
    id: "llamaindex",
    label: "LlamaIndex",
    desc: "Document-centric agents, great for RAG workflows",
    install: () => _pipInstall("llama-index"),
    homepage: "https://www.llamaindex.ai",
  },
  {
    id: "semantic-kernel",
    label: "Semantic Kernel",
    desc: "Microsoft — plugin architecture, Python/.NET",
    install: () => _pipInstall("semantic-kernel"),
    homepage: "https://github.com/microsoft/semantic-kernel",
  },
];

async function run() {
  console.log(`\n${BOLD}  Bitcoin Proof of Compute — Intelligent Installer${RESET}`);
  console.log(`${DIM}  btcpc.net  •  sovereign chain for AI inference${RESET}\n`);

  // 1. Detect which engines are already installed
  say("Scanning for agentic AI engines on this device...\n");
  const detected = tools.detect_engines().result;

  let engine = null;

  if (detected.length > 0) {
    say(`Found: ${detected.map(e => e.label).join(", ")}`);
    if (detected.length === 1) {
      say(`Will use ${detected[0].label} to guide setup.`);
      engine = detected[0].id;
    } else {
      const choice = await tools.ask_choice(
        "Which engine should guide your BTCPC setup?",
        detected.map(e => ({ label: e.label, desc: e.id, value: e.id }))
      );
      engine = choice ? choice.value : null;
    }
  } else {
    // 2. No engine found — offer to install one
    console.log();
    say("No agentic AI engine found on this device.");
    say("An agentic engine lets you manage BTCPC — and everything else — through");
    say("natural conversation. After setup you could say: \"hey hermes, check my balance\"\n");

    const wantEngine = await tools.ask_confirm(
      "Would you like to install a local agentic AI engine?", true
    );

    if (wantEngine) {
      engine = await _offerEngineInstall();
    }
  }

  // 3. Run setup via chosen engine, or wizard fallback
  if (!engine || engine === "wizard") {
    say("Using the built-in setup wizard.\n");
    await wizard.run();
  } else {
    await _runViaEngine(engine);
  }

  // 4. Register BTCPC skill in all detected+installed engines
  console.log(`\n${BOLD}  Registering BTCPC commands in your AI engines...${RESET}`);
  const registered = registerAll();
  if (registered.length === 0) {
    say("No engines detected for skill registration. Run btcpc-install again after installing an engine.");
  }

  console.log("\n");
  say("Done. Run `node bin/btcpc-cli status` to check your nodes.\n");
}

async function _offerEngineInstall() {
  const featured = ALL_ENGINES.filter(e => e.featured);
  const rest     = ALL_ENGINES.filter(e => !e.featured);

  const choices = [
    ...featured.map(e => ({ label: e.label, desc: e.desc, value: e.id })),
    { label: "More options...", desc: `${rest.length} additional engines`, value: "__more__" },
    { label: "Skip — use plain wizard instead", desc: "no AI, just questions", value: "wizard" },
  ];

  let choice = await tools.ask_choice("Which agentic engine would you like?", choices);
  if (!choice) return "wizard";

  if (choice.value === "__more__") {
    const allChoices = [
      ...ALL_ENGINES.map(e => ({ label: e.label, desc: e.desc, value: e.id })),
      { label: "Skip — use plain wizard instead", value: "wizard" },
    ];
    choice = await tools.ask_choice("All available engines:", allChoices);
    if (!choice) return "wizard";
  }

  if (choice.value === "wizard") return "wizard";

  const engine = ALL_ENGINES.find(e => e.id === choice.value);
  if (!engine) return "wizard";

  say(`\nInstalling ${engine.label}... (${engine.homepage})`);
  const ok = await tools.ask_confirm(`About to install ${engine.label}. Proceed?`, true);
  if (!ok) return "wizard";

  const success = engine.install();
  if (success) {
    say(`${engine.label} installed.\n`);
    return engine.id;
  } else {
    say(`Install failed. Falling back to plain wizard.\n`);
    return "wizard";
  }
}

async function _runViaEngine(engineId) {
  const soulPath = require("path").resolve(__dirname, "SOUL.md");

  if (engineId === "hermes") {
    say("Launching Hermes with BTCPC setup profile...\n");
    say("Type naturally — Hermes will ask about each node type and set it up step by step.\n");
    const proc = hermes.startSession(soulPath);
    await new Promise((resolve) => proc.on("exit", resolve));
    return;
  }

  if (engineId === "openclaw") {
    say("Launching OpenClaw with BTCPC SOUL...\n");
    say("Type naturally — OpenClaw will guide you through node setup.\n");
    const { spawnSync } = require("child_process");
    spawnSync("openclaw", ["--soul", soulPath], { stdio: "inherit" });
    return;
  }

  // Python engines: generate a minimal bootstrap script and exec it
  if (["crewai", "autogen", "langgraph"].includes(engineId)) {
    say(`Launching ${engineId} setup agent...\n`);
    const script = _pythonBootstrap(engineId, soulPath);
    const tmpPath = require("os").tmpdir() + `/btcpc_${engineId}_install.py`;
    require("fs").writeFileSync(tmpPath, script, "utf8");
    const { spawnSync } = require("child_process");
    spawnSync("python3", [tmpPath], { stdio: "inherit" });
    return;
  }

  // Anything else: wizard fallback
  await wizard.run();
}

function _pythonBootstrap(engine, soulPath) {
  const soul = require("fs").readFileSync(soulPath, "utf8").replace(/\\/g, "\\\\").replace(/"/g, '\\"');
  return `
import subprocess, sys, json

SOUL = """${soul}"""
BTCPC_DIR = "${require("path").resolve(__dirname, "../..")}"

print("[btcpc] Launching ${engine} agent for BTCPC setup...")
print("[btcpc] The agent will ask what you want to run, one step at a time.")
print()

try:
    import ${engine.replace("-", "_")}
    print(f"[btcpc] Using ${engine}. Ask it: 'help me set up a BTCPC node'")
    print("[btcpc] (Full ${engine} integration coming in a future release — wizard fallback active)")
except ImportError:
    print("[btcpc] ${engine} import failed — using wizard fallback")

subprocess.run(["node", BTCPC_DIR + "/bin/btcpc-install", "--wizard"], check=False)
`;
}

function _shellInstall(cmd) {
  const { spawnSync } = require("child_process");
  const r = spawnSync("bash", ["-c", cmd], { stdio: "inherit", timeout: 300000 });
  return r.status === 0;
}

function _pipInstall(pkg) {
  const { spawnSync } = require("child_process");
  const r = spawnSync("pip3", ["install", pkg], { stdio: "inherit", timeout: 300000 });
  return r.status === 0;
}

function _npmInstall(pkg) {
  const { spawnSync } = require("child_process");
  const r = spawnSync("npm", ["install", "-g", pkg], { stdio: "inherit", timeout: 300000 });
  return r.status === 0;
}

function _dockerInstall(image) {
  const { spawnSync } = require("child_process");
  const r = spawnSync("docker", ["pull", image], { stdio: "inherit", timeout: 300000 });
  return r.status === 0;
}

module.exports = { run };
