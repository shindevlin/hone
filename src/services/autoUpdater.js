"use strict";

/**
 * BTCPC Auto-Updater
 *
 * Runs inside the miner process. Checks GitHub for new commits
 * every CHECK_INTERVAL. If updates found, pulls and restarts
 * the process automatically.
 *
 * Set BTCPC_AUTO_UPDATE=false to disable.
 */

const { execSync } = require('child_process');
const path = require('path');

const REPO_DIR = path.resolve(__dirname, '../..');
const CHECK_INTERVAL_MS = parseInt(process.env.BTCPC_UPDATE_INTERVAL_MS) || 900000; // 15 min

let updateTimer = null;

function run(cmd) {
  return execSync(cmd, { cwd: REPO_DIR, encoding: 'utf8', timeout: 120000 }).trim();
}

function log(msg) {
  console.log(`[BTCPC-UPDATE] ${new Date().toISOString()} -- ${msg}`);
}

/**
 * Check for updates and apply if found.
 * Returns true if an update was applied (process will restart).
 */
function checkAndUpdate() {
  try {
    run('git fetch origin');

    const localHash = run('git rev-parse HEAD');
    const remoteHash = run('git rev-parse origin/main');

    if (localHash === remoteHash) {
      return false;
    }

    const behindCount = run('git rev-list HEAD..origin/main --count');
    log(`${behindCount} new commit(s) available`);

    // Show what's coming
    const commitLog = run('git log HEAD..origin/main --oneline');
    log('Incoming:\n' + commitLog);

    // Check the remote version
    try {
      const remoteVersion = run('git show origin/main:package.json | node -e "process.stdin.resume();let d=\'\';process.stdin.on(\'data\',c=>d+=c);process.stdin.on(\'end\',()=>console.log(JSON.parse(d).version))"');
      log(`Updating to v${remoteVersion}`);
    } catch (_) {}

    // Pull
    run('git stash 2>/dev/null; git pull origin main');
    log('Pulled latest code');

    // Install deps if needed
    const changed = run('git diff HEAD~' + behindCount + ' --name-only');
    if (changed.includes('package.json') || changed.includes('package-lock.json')) {
      log('Installing new dependencies...');
      run('npm install');
    }

    log('Restarting process...');

    // Restart: spawn a new process and exit this one
    const { spawn } = require('child_process');
    const args = process.argv.slice(1);
    const child = spawn(process.argv[0], args, {
      cwd: REPO_DIR,
      detached: true,
      stdio: 'inherit',
      env: process.env
    });
    child.unref();

    // Give the new process a moment to start, then exit
    setTimeout(() => process.exit(0), 2000);
    return true;

  } catch (err) {
    log('Update check failed: ' + err.message);
    return false;
  }
}

/**
 * Start the auto-update loop.
 */
function startAutoUpdater() {
  if (process.env.BTCPC_AUTO_UPDATE === 'false') {
    log('Auto-update disabled (BTCPC_AUTO_UPDATE=false)');
    return;
  }

  const currentVersion = (() => {
    try { return require('../../package.json').version; } catch (_) { return 'unknown'; }
  })();

  log(`Running v${currentVersion}, checking for updates every ${CHECK_INTERVAL_MS / 60000}min`);

  // Check once on startup (delayed 30s to let the miner stabilize)
  setTimeout(() => {
    checkAndUpdate();
  }, 30000);

  // Then check periodically
  updateTimer = setInterval(() => {
    checkAndUpdate();
  }, CHECK_INTERVAL_MS);
}

function stopAutoUpdater() {
  if (updateTimer) {
    clearInterval(updateTimer);
    updateTimer = null;
  }
}

module.exports = { startAutoUpdater, stopAutoUpdater, checkAndUpdate };
