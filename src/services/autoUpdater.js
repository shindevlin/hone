"use strict";

/**
 * BTCPC Auto-Updater
 *
 * Checks for updates, stages them, notifies miner via Telegram,
 * and waits for confirmation before restarting.
 *
 * Flow: check → pull → notify bot → miner confirms → restart
 *
 * Set BTCPC_AUTO_UPDATE=false to disable.
 */

const { execSync, spawn } = require('child_process');
const path = require('path');
const axios = require('axios');

const REPO_DIR = path.resolve(__dirname, '../..');
const CHECK_INTERVAL_MS = parseInt(process.env.BTCPC_UPDATE_INTERVAL_MS) || 900000; // 15 min
const IS_WINDOWS = process.platform === 'win32';
const BOT_API_URL = process.env.BTCPC_API_URL || 'http://localhost:3100';
const BOT_API_KEY = process.env.BOT_API_KEY;

let updateTimer = null;
let pendingUpdate = null;
let restartApproved = false;

function run(cmd) {
  // windowsHide: true prevents cmd.exe windows from flashing on Windows
  const opts = {
    cwd: REPO_DIR,
    encoding: 'utf8',
    timeout: 120000,
    windowsHide: true,
    stdio: ['pipe', 'pipe', 'pipe']
  };
  if (IS_WINDOWS) opts.shell = 'cmd.exe';
  return execSync(cmd, opts).trim();
}

function log(msg) {
  console.log(`[auto-updater] ${new Date().toISOString()} -- ${msg}`);
}

/**
 * Send a notification to the miner via the bot API alert endpoint.
 */
async function notifyMiner(message) {
  if (!BOT_API_KEY) return;
  try {
    await axios.post(`${BOT_API_URL}/api/bot/linked-users`, {}, {
      headers: { 'x-bot-key': BOT_API_KEY },
      timeout: 5000
    }).then(async (res) => {
      // Use the walletbot alert webhook to notify all linked miners
      const alertUrl = process.env.ALERTBOT_URL || 'http://localhost:9900';
      const alertKey = process.env.ALERTBOT_API_KEY;
      if (alertKey) {
        await axios.post(`${alertUrl}/alert`, {
          project: 'btcpc-updater',
          severity: 'info',
          message
        }, {
          headers: { 'x-api-key': alertKey },
          timeout: 5000
        }).catch(() => {});
      }
    });
  } catch (_) {
    log('Could not notify via bot (bot may not be running)');
  }
}

/**
 * Check for updates and stage if found. Notify miner via Telegram.
 */
async function checkAndStage() {
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

    let remoteVersion = 'unknown';
    try {
      const pkgJson = run('git show origin/main:package.json');
      remoteVersion = JSON.parse(pkgJson).version;
    } catch (_) {}

    const currentVersion = (() => {
      try { return require('../../package.json').version; } catch (_) { return 'unknown'; }
    })();

    // Stage: pull but don't restart
    try { run('git stash'); } catch (_) {}
    run('git pull origin main');
    log(`Staged v${remoteVersion} (${behindCount} commits)`);

    // Install deps if needed
    try {
      const changed = run('git diff HEAD~' + behindCount + ' --name-only');
      if (changed.includes('package.json') || changed.includes('package-lock.json')) {
        log('Installing new dependencies...');
        run('npm install');
      }
    } catch (_) {}

    pendingUpdate = {
      version: remoteVersion,
      previousVersion: currentVersion,
      commits: parseInt(behindCount),
      commitLog,
      pulledAt: new Date().toISOString()
    };

    // Notify miner via Telegram + desktop notification
    const msg = [
      `Update available: v${currentVersion} → v${remoteVersion}`,
      `${behindCount} commit(s):`,
      commitLog.split('\n').slice(0, 5).join('\n'),
      '',
      'Update to keep earning. Reply /approve-update to restart.'
    ].join('\n');

    await notifyMiner(msg);

    // Desktop notification
    try {
      const { notifyUpdate } = require('./systemNotify');
      notifyUpdate(currentVersion, remoteVersion, behindCount);
    } catch (_) {}

    log(`Miner notified. Waiting for approval or manual restart.`);

    return pendingUpdate;

  } catch (err) {
    log('Update check failed: ' + err.message);
    return null;
  }
}

/**
 * Approve and apply the pending update (restart the process).
 * Called when miner confirms via Telegram.
 */
function approveUpdate() {
  if (!pendingUpdate) {
    log('No pending update to approve');
    return false;
  }

  log(`Update approved. Restarting to v${pendingUpdate.version}...`);
  restartApproved = true;

  // Restart: spawn a new process and exit
  const args = process.argv.slice(1);
  const child = spawn(process.argv[0], args, {
    cwd: REPO_DIR,
    detached: true,
    stdio: 'inherit',
    env: process.env
  });
  child.unref();

  setTimeout(() => process.exit(0), 2000);
  return true;
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

  log(`Running v${currentVersion}, checking every ${CHECK_INTERVAL_MS / 60000}min`);

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

module.exports = { startAutoUpdater, stopAutoUpdater, checkAndStage, getPendingUpdate, approveUpdate };
