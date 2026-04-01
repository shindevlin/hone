"use strict";

/**
 * BTCPC Auto-Updater
 *
 * Stage-and-notify model: checks for updates, downloads them,
 * but does NOT auto-restart. Notifies the miner that an update
 * is available. Applied on next manual restart.
 *
 * Set BTCPC_AUTO_UPDATE=false to disable.
 */

const { execSync } = require('child_process');
const path = require('path');

const REPO_DIR = path.resolve(__dirname, '../..');
const CHECK_INTERVAL_MS = parseInt(process.env.BTCPC_UPDATE_INTERVAL_MS) || 900000; // 15 min
const IS_WINDOWS = process.platform === 'win32';

let updateTimer = null;
let pendingUpdate = null; // { version, commits, pulledAt }

function run(cmd) {
  // Use platform-appropriate shell
  const shell = IS_WINDOWS ? { shell: 'cmd.exe' } : {};
  return execSync(cmd, { cwd: REPO_DIR, encoding: 'utf8', timeout: 120000, ...shell }).trim();
}

function log(msg) {
  console.log(`[auto-updater] ${new Date().toISOString()} -- ${msg}`);
}

/**
 * Check for updates. If found, stage them (git pull) but do NOT restart.
 * Returns update info or null.
 */
function checkAndStage() {
  try {
    run('git fetch origin');

    const localHash = run('git rev-parse HEAD');
    const remoteHash = run('git rev-parse origin/main');

    if (localHash === remoteHash) {
      return null;
    }

    const behindCount = run('git rev-list HEAD..origin/main --count');
    log(`${behindCount} new commit(s) available`);

    const commitLog = run('git log HEAD..origin/main --oneline');
    log('Available:\n' + commitLog);

    // Get remote version
    let remoteVersion = 'unknown';
    try {
      const pkgJson = run('git show origin/main:package.json');
      remoteVersion = JSON.parse(pkgJson).version;
    } catch (_) {}

    // Stage: pull the code but don't restart
    // Cross-platform: avoid shell redirects that break on Windows
    try { run('git stash'); } catch (_) {} // stash any local changes
    run('git pull origin main');
    log(`Staged v${remoteVersion} (${behindCount} commits). Will apply on next restart.`);

    // Install deps if package.json changed
    try {
      const changed = run('git diff HEAD~' + behindCount + ' --name-only');
      if (changed.includes('package.json') || changed.includes('package-lock.json')) {
        log('Installing new dependencies...');
        run('npm install');
      }
    } catch (_) {}

    pendingUpdate = {
      version: remoteVersion,
      commits: parseInt(behindCount),
      commitLog,
      pulledAt: new Date().toISOString()
    };

    return pendingUpdate;

  } catch (err) {
    log('Update check failed: ' + err.message);
    return null;
  }
}

/**
 * Start the auto-update loop.
 */
function startAutoUpdater() {
  if (process.env.BTCPC_AUTO_UPDATE === 'false') {
    log('Disabled (BTCPC_AUTO_UPDATE=false)');
    return;
  }

  const currentVersion = (() => {
    try { return require('../../package.json').version; } catch (_) { return 'unknown'; }
  })();

  log(`Running v${currentVersion}, checking every ${CHECK_INTERVAL_MS / 60000}min (stage-only, no auto-restart)`);

  // Check once on startup (delayed 60s)
  setTimeout(() => checkAndStage(), 60000);

  // Then check periodically
  updateTimer = setInterval(() => checkAndStage(), CHECK_INTERVAL_MS);
}

function stopAutoUpdater() {
  if (updateTimer) {
    clearInterval(updateTimer);
    updateTimer = null;
  }
}

function getPendingUpdate() {
  return pendingUpdate;
}

module.exports = { startAutoUpdater, stopAutoUpdater, checkAndStage, getPendingUpdate };
