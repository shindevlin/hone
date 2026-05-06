#!/usr/bin/env node

const { spawnSync } = require("child_process");

let jestEntry = null;
try {
  jestEntry = require.resolve("jest/bin/jest");
} catch (_) {
  jestEntry = null;
}

const args = process.argv.slice(2);

if (jestEntry) {
  const result = spawnSync(process.execPath, [jestEntry, ...args], {
    stdio: "inherit",
  });
  process.exit(result.status === null ? 1 : result.status);
}

const fallback = spawnSync("npx", ["jest", ...args], {
  stdio: "inherit",
});

process.exit(fallback.status === null ? 1 : fallback.status);
