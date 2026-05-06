#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");
const { spawnSync } = require("child_process");

const repoRoot = path.resolve(__dirname, "..");
const docsRoot = path.join(repoRoot, "docs");
const wikiRoot = path.join(repoRoot, ".code-review-graph", "wiki");
const docsWikiRoot = path.join(docsRoot, "code-wiki");

const curatedPages = [
  { file: "p2p-server.md", title: "P2P server", note: "Peer socket lifecycle, relay bootstrap, reconnects, and inbound handling." },
  { file: "p2p-handle.md", title: "P2P message handling", note: "Peer announcements, heartbeat gossip, and message routing." },
  { file: "chain-epoch.md", title: "Clock / epoch logic", note: "Epoch timing, authority rotation, and chain-time behavior." },
  { file: "chain-node.md", title: "Node registry", note: "Registered miners, clocks, storage hosts, and device roles." },
  { file: "chain-state.md", title: "Chain state", note: "State store shape, derived balances, and on-disk chain truth." },
  { file: "services-epoch.md", title: "Epoch services", note: "Reward settlement and per-epoch service accounting." },
  { file: "services-stateful.md", title: "Stateful hosting", note: "Host lifecycle for services that need durable state." },
  { file: "mining-finalization.md", title: "Mining finalization", note: "How work proofs, rewards, and block finalization fit together." },
  { file: "tests-heartbeat.md", title: "Heartbeat tests", note: "Coverage around clock liveness, witness checks, and reward eligibility." },
  { file: "tests-stateful.md", title: "Stateful tests", note: "Coverage around storage and stateful service behavior." },
];

function ensureWikiBuilt() {
  const cli = process.platform === "win32" ? "code-review-graph.cmd" : "code-review-graph";
  const result = spawnSync(cli, ["wiki", "--repo", repoRoot, "--force"], { stdio: "inherit" });
  if (result.error && result.error.code !== "ENOENT") {
    throw result.error;
  }
  if (result.error && result.error.code === "ENOENT") {
    console.warn("[code-wiki] code-review-graph CLI not found; reusing existing wiki files");
  } else if (result.status !== 0) {
    throw new Error("code-review-graph wiki failed with exit code " + result.status);
  }
}

function generateLandingPage() {
  if (!fs.existsSync(wikiRoot)) {
    throw new Error("Wiki directory not found: " + wikiRoot);
  }

  fs.mkdirSync(docsWikiRoot, { recursive: true });
  const pageCount = fs.readdirSync(wikiRoot).filter((f) => f.endsWith(".md")).length;
  const selected = curatedPages.filter((entry) => fs.existsSync(path.join(wikiRoot, entry.file)));

  const lines = [];
  lines.push("# Code Wiki");
  lines.push("");
  lines.push("Auto-generated documentation from the code knowledge graph community structure.");
  lines.push("");
  lines.push(`**Pages available**: ${pageCount}`);
  lines.push("");
  lines.push("## Quick Start");
  lines.push("");
  lines.push("- [Full generated index](index.md) - all community pages in one place.");
  lines.push("- [BTCPC docs index](../INDEX.md) - product and architecture docs that are hand-curated.");
  lines.push("");
  lines.push("## Most Useful Pages");
  lines.push("");
  for (const entry of selected) {
    lines.push(`- [${entry.title}](${entry.file}) - ${entry.note}`);
  }
  lines.push("");
  lines.push("## Browse By Community");
  lines.push("");
  lines.push("- `p2p-*` - peer discovery, sockets, relay bootstrap, gossip.");
  lines.push("- `chain-*` - epoch sealing, state, validation, and node registry.");
  lines.push("- `services-*` - hosting, rewards, stateful services, and payouts.");
  lines.push("- `tests-*` - the fastest way to see what is covered today.");
  lines.push("");
  lines.push("The pages below are the exact generated community docs served by the website.");
  lines.push("");

  fs.writeFileSync(path.join(docsWikiRoot, "README.md"), lines.join("\n"));
  fs.copyFileSync(path.join(wikiRoot, "index.md"), path.join(docsWikiRoot, "index.md"));
}

try {
  ensureWikiBuilt();
  generateLandingPage();
  console.log("[code-wiki] landing page written to docs/code-wiki/README.md");
} catch (err) {
  console.error("[code-wiki] " + err.message);
  process.exit(1);
}
