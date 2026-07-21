#!/usr/bin/env node
"use strict";

/**
 * HONE natoshi vouching script
 * Shin Devlin
 *
 * natoshisakamoto vouches for all known sensors, miners, and accounts on the
 * network by staking HONE on each one. This signals network legitimacy and
 * means natoshi's staked amounts are at risk if any vouched entity is slashed.
 *
 * Run: node scripts/vouch-natoshi.js [--dry-run]
 *
 * Requires the node API to be running at localhost:4242 with a valid natoshi JWT.
 * Set HONE_NATOSHI_TOKEN env var, or it will try to auth from secrets.json.
 */

const https = require('https');
const http = require('http');
const path = require('path');
const fs = require('fs');

const API_BASE = process.env.HONE_API_BASE || 'http://localhost:4242';
const DRY_RUN = process.argv.includes('--dry-run');

// ── Entities to vouch for ───────────────────────────────────────────────────
// Each entry: { target_id, target_type, stake_amount, note }
//
// Stake amounts per type (HONE):
//   Full miner PC (grouchly, beastly):   1000
//   Clock node (any PC running clock):    500
//   GNSS sensor (Hyfix, high hardware):   500
//   LoRa gateway / Nebra:                 300
//   Standard sensor:                      100
//   User account (josh, shindevlin, etc): 200

const VOUCHES = [
  // ── Accounts ──────────────────────────────────────────────────────────────
  { target_id: 'shindevlin',     target_type: 'account', stake_amount: 200, note: 'genesis_miner' },
  { target_id: 'josh',           target_type: 'account', stake_amount: 200, note: 'founding_member' },
  { target_id: 'sovai',          target_type: 'account', stake_amount: 200, note: 'founding_member' },

  // ── Miner PCs ─────────────────────────────────────────────────────────────
  { target_id: 'grouchly',       target_type: 'miner',   stake_amount: 1000, note: 'genesis_miner_node' },
  { target_id: 'beastly',        target_type: 'miner',   stake_amount: 1000, note: 'gpu_miner_node' },

  // ── Sensors ───────────────────────────────────────────────────────────────
  { target_id: 'shindevlin/gnss-base', target_type: 'sensor', stake_amount: 500, note: 'hyfix_mobilecm_gnss' },
  { target_id: 'shindevlin/nebra-1',   target_type: 'sensor', stake_amount: 300, note: 'nebra_lora_gateway' },
];

// ── Auth token ──────────────────────────────────────────────────────────────
function getToken() {
  if (process.env.HONE_NATOSHI_TOKEN) return process.env.HONE_NATOSHI_TOKEN;

  // Try secrets.json
  const secretsPath = path.join(process.env.HOME || '/root', '.hone', 'secrets.json');
  if (fs.existsSync(secretsPath)) {
    try {
      const secrets = JSON.parse(fs.readFileSync(secretsPath, 'utf8'));
      if (secrets.natoshisakamoto && secrets.natoshisakamoto.jwt) {
        return secrets.natoshisakamoto.jwt;
      }
      // Try any admin account token
      for (const [acct, data] of Object.entries(secrets)) {
        if (data && data.jwt && (acct === 'shindevlin' || acct === 'natoshisakamoto')) {
          return data.jwt;
        }
      }
    } catch (_) {}
  }
  return null;
}

function request(method, path, body, token) {
  return new Promise((resolve, reject) => {
    const url = new URL(API_BASE + path);
    const isHttps = url.protocol === 'https:';
    const mod = isHttps ? https : http;
    const data = body ? JSON.stringify(body) : null;

    const options = {
      hostname: url.hostname,
      port: url.port || (isHttps ? 443 : 80),
      path: url.pathname + url.search,
      method,
      headers: {
        'Content-Type': 'application/json',
        ...(token ? { 'Authorization': 'Bearer ' + token } : {}),
        ...(data ? { 'Content-Length': Buffer.byteLength(data) } : {}),
      },
    };

    const req = mod.request(options, (res) => {
      let body = '';
      res.on('data', chunk => body += chunk);
      res.on('end', () => {
        try {
          resolve({ status: res.statusCode, data: JSON.parse(body) });
        } catch (_) {
          resolve({ status: res.statusCode, data: body });
        }
      });
    });
    req.on('error', reject);
    if (data) req.write(data);
    req.end();
  });
}

async function main() {
  const token = getToken();
  if (!token) {
    console.error('[vouch-natoshi] No token found. Set HONE_NATOSHI_TOKEN or add natoshisakamoto.jwt to ~/.hone/secrets.json');
    process.exit(1);
  }

  console.log('[vouch-natoshi] Starting' + (DRY_RUN ? ' (DRY RUN)' : '') + '...');
  console.log('[vouch-natoshi] Target: ' + API_BASE);
  console.log('[vouch-natoshi] Vouches to issue: ' + VOUCHES.length);
  console.log('');

  let ok = 0;
  let skipped = 0;
  let failed = 0;

  for (const vouch of VOUCHES) {
    const label = '[' + vouch.target_id + ' (' + vouch.target_type + ')]';

    if (DRY_RUN) {
      console.log('  DRY RUN ' + label + ' stake=' + vouch.stake_amount + ' HONE  note=' + vouch.note);
      ok++;
      continue;
    }

    try {
      const resp = await request('POST', '/api/vouch', {
        voucher: 'natoshisakamoto',
        target_id: vouch.target_id,
        target_type: vouch.target_type,
        stake_amount: vouch.stake_amount,
        note: vouch.note,
      }, token);

      if (resp.status === 201 || resp.status === 200) {
        console.log('  ✓ ' + label + ' — vouched with ' + vouch.stake_amount + ' HONE');
        ok++;
      } else if (resp.status === 409) {
        console.log('  ~ ' + label + ' — already vouched (skipped)');
        skipped++;
      } else {
        console.error('  ✗ ' + label + ' — HTTP ' + resp.status + ': ' + JSON.stringify(resp.data));
        failed++;
      }
    } catch (err) {
      console.error('  ✗ ' + label + ' — ' + err.message);
      failed++;
    }

    // Small delay between requests
    await new Promise(r => setTimeout(r, 200));
  }

  console.log('');
  console.log('[vouch-natoshi] Done: ' + ok + ' ok, ' + skipped + ' skipped, ' + failed + ' failed');
  if (failed > 0) process.exit(1);
}

main().catch(err => {
  console.error('[vouch-natoshi] Fatal:', err.message);
  process.exit(1);
});
