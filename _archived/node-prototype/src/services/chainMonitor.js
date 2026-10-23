"use strict";

/**
 * chainMonitor — cross-chain address activity watcher
 * Shin Devlin
 *
 * Polls EVM chains (Base, Arbitrum, Ethereum), Solana, and Bitcoin for
 * balance/activity changes on addresses registered to HONE accounts.
 * When a change is detected, emits a CROSS_CHAIN_ACTIVITY ledger entry.
 *
 * Design goals:
 *  - No API keys required (public RPCs only)
 *  - Self-healing: RPC errors are logged and retried next interval
 *  - Minimal resource use: one HTTP request per address per chain per poll
 *
 * Usage:
 *   const monitor = createChainMonitor({ ledger, stateStore });
 *   monitor.start();  // begins polling
 *   monitor.stop();   // cancels timer
 *   monitor.getActivity(username, chain); // returns recorded activities
 *   monitor.resetForTests(); // clears all cached state
 */

var POLL_INTERVAL_MS = parseInt(process.env.HONE_MONITOR_INTERVAL_MS || "60000", 10);
var shouldStartBackgroundTimers = require("./backgroundTimers").shouldStartBackgroundTimers;

var EVM_CHAINS = [
  { name: "base",     rpcUrl: "https://mainnet.base.org" },
  { name: "arbitrum", rpcUrl: "https://arb1.arbitrum.io/rpc" },
  { name: "ethereum", rpcUrl: "https://eth.llamarpc.com" },
];

var SOLANA_RPC = "https://api.mainnet-beta.solana.com";
var BITCOIN_API = "https://mempool.space/api/address";

// ─────────────────────────────────────────────────────────────────
// EVMMonitor
// ─────────────────────────────────────────────────────────────────

class EVMMonitor {
  constructor() {
    // key: "chainName:address" → last known balance (integer wei)
    this.lastBalances = new Map();
  }

  /**
   * Check one address on one EVM chain via eth_getBalance.
   * Returns { changed, chain, address, oldBalance, newBalance } or { changed: false }.
   * Never throws — all errors are returned as { changed: false, error }.
   */
  async checkAddress(address, rpcUrl, chainName) {
    var key = chainName + ":" + address;
    try {
      var response = await _fetchWithTimeout(rpcUrl, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          jsonrpc: "2.0",
          method: "eth_getBalance",
          params: [address, "latest"],
          id: 1,
        }),
      }, 10000);

      if (!response.ok) {
        return { changed: false, error: "HTTP " + response.status };
      }

      var data = await response.json();
      if (!data || !data.result) {
        return { changed: false, error: "empty result" };
      }

      var balance = parseInt(data.result, 16);
      if (isNaN(balance)) {
        return { changed: false, error: "bad balance: " + data.result };
      }

      var prev = this.lastBalances.get(key);
      this.lastBalances.set(key, balance);

      if (prev !== undefined && prev !== balance) {
        return {
          changed: true,
          chain: chainName,
          address: address,
          oldBalance: prev,
          newBalance: balance,
        };
      }

      return { changed: false };
    } catch (err) {
      return { changed: false, error: err.message };
    }
  }

  reset() {
    this.lastBalances.clear();
  }
}

// ─────────────────────────────────────────────────────────────────
// SolanaMonitor
// ─────────────────────────────────────────────────────────────────

class SolanaMonitor {
  constructor() {
    // key: "solana:address" → last known balance (lamports)
    this.lastBalances = new Map();
  }

  /**
   * Check a Solana address via getBalance RPC call.
   * Returns { changed, chain, address, oldBalance, newBalance } or { changed: false }.
   */
  async checkAddress(address) {
    var key = "solana:" + address;
    try {
      var response = await _fetchWithTimeout(SOLANA_RPC, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          jsonrpc: "2.0",
          id: 1,
          method: "getBalance",
          params: [address],
        }),
      }, 10000);

      if (!response.ok) {
        return { changed: false, error: "HTTP " + response.status };
      }

      var data = await response.json();
      if (!data || !data.result || typeof data.result.value === "undefined") {
        return { changed: false, error: "empty result" };
      }

      var balance = data.result.value;
      if (typeof balance !== "number") {
        return { changed: false, error: "bad balance" };
      }

      var prev = this.lastBalances.get(key);
      this.lastBalances.set(key, balance);

      if (prev !== undefined && prev !== balance) {
        return {
          changed: true,
          chain: "solana",
          address: address,
          oldBalance: prev,
          newBalance: balance,
        };
      }

      return { changed: false };
    } catch (err) {
      return { changed: false, error: err.message };
    }
  }

  reset() {
    this.lastBalances.clear();
  }
}

// ─────────────────────────────────────────────────────────────────
// BitcoinMonitor
// ─────────────────────────────────────────────────────────────────

class BitcoinMonitor {
  constructor() {
    // key: "bitcoin:address" → last known funded_txo_sum (sats)
    this.lastFundedSums = new Map();
  }

  /**
   * Check a Bitcoin address via mempool.space REST API.
   * Tracks funded_txo_sum; any increase indicates incoming activity.
   * Returns { changed, chain, address, oldSum, newSum } or { changed: false }.
   */
  async checkAddress(address) {
    var key = "bitcoin:" + address;
    try {
      var url = BITCOIN_API + "/" + encodeURIComponent(address);
      var response = await _fetchWithTimeout(url, {
        method: "GET",
        headers: { "Accept": "application/json" },
      }, 10000);

      if (!response.ok) {
        return { changed: false, error: "HTTP " + response.status };
      }

      var data = await response.json();
      if (!data || !data.chain_stats) {
        return { changed: false, error: "missing chain_stats" };
      }

      var funded = data.chain_stats.funded_txo_sum;
      if (typeof funded !== "number") {
        return { changed: false, error: "bad funded_txo_sum" };
      }

      var prev = this.lastFundedSums.get(key);
      this.lastFundedSums.set(key, funded);

      if (prev !== undefined && prev !== funded) {
        return {
          changed: true,
          chain: "bitcoin",
          address: address,
          oldSum: prev,
          newSum: funded,
        };
      }

      return { changed: false };
    } catch (err) {
      return { changed: false, error: err.message };
    }
  }

  reset() {
    this.lastFundedSums.clear();
  }
}

// ─────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────

/**
 * fetch() with a timeout. Returns the Response or throws on timeout/error.
 */
async function _fetchWithTimeout(url, opts, timeoutMs) {
  var controller = new AbortController();
  var timer = setTimeout(function () { controller.abort(); }, timeoutMs || 10000);
  try {
    var res = await fetch(url, Object.assign({}, opts, { signal: controller.signal }));
    return res;
  } finally {
    clearTimeout(timer);
  }
}

// ─────────────────────────────────────────────────────────────────
// createChainMonitor — the public factory
// ─────────────────────────────────────────────────────────────────

/**
 * Create a chain monitor instance.
 *
 * @param {object} options
 *   ledger    — src/services/ledger.js (must export recordCrossChainActivity)
 *   stateStore — src/chain/stateStore.js (must export getAllAccounts)
 *   fetch     — optional fetch override for testing
 *   evmChains — optional array of { name, rpcUrl } overrides for testing
 *   solanaRpc — optional Solana RPC URL override for testing
 *   bitcoinApi — optional Bitcoin API base URL override for testing
 */
function createChainMonitor(options) {
  options = options || {};

  var _ledger = options.ledger;
  var _stateStore = options.stateStore;

  // Allow tests to inject a custom fetch
  if (options.fetch) {
    global._honeMonitorFetch = options.fetch;
  }

  // Allow tests to override chain endpoints
  var _evmChains = options.evmChains || EVM_CHAINS;
  var _solanaRpc = options.solanaRpc || SOLANA_RPC;
  var _bitcoinApi = options.bitcoinApi || BITCOIN_API;

  var evmMonitor = new EVMMonitor();
  var solanaMonitor = new SolanaMonitor();
  var bitcoinMonitor = new BitcoinMonitor();

  // Per-user activity log: username → [{ chain, address, activity_type, data, epoch, timestamp }]
  var activityLog = new Map();

  var _timer = null;
  var _running = false;

  function _log(msg) {
    console.log("[chain-monitor] " + msg);
  }

  function _warn(msg) {
    console.warn("[chain-monitor] WARN: " + msg);
  }

  function _recordActivity(username, chain, address, activityType, data, epoch) {
    var list = activityLog.get(username) || [];
    list.push({
      chain: chain,
      address: address,
      activity_type: activityType,
      data: data,
      epoch: epoch,
      timestamp: Date.now(),
    });
    // Keep last 1000 activities per user to bound memory
    if (list.length > 1000) list = list.slice(-1000);
    activityLog.set(username, list);
  }

  async function _pollAllAccounts() {
    if (!_stateStore) return;
    var accounts;
    try {
      accounts = _stateStore.getAllAccounts();
    } catch (err) {
      _warn("getAllAccounts failed: " + err.message);
      return;
    }

    var epoch = 0;
    try {
      if (_ledger && typeof _ledger.getCurrentEpoch === "function") {
        epoch = (await _ledger.getCurrentEpoch()) || 0;
      }
    } catch (_) {}

    for (var i = 0; i < accounts.length; i++) {
      var account = accounts[i];
      var username = account.username;
      var addrs = account.chain_addresses || {};

      // ── EVM (Base, Arbitrum, Ethereum) ──────────────────────────
      if (addrs.evm) {
        for (var ci = 0; ci < _evmChains.length; ci++) {
          var chain = _evmChains[ci];
          try {
            var evmResult = await evmMonitor.checkAddress(addrs.evm, chain.rpcUrl, chain.name);
            if (evmResult.changed) {
              var evmData = {
                old_balance: evmResult.oldBalance,
                new_balance: evmResult.newBalance,
                delta: evmResult.newBalance - evmResult.oldBalance,
              };
              _log(
                chain.name + ": " + addrs.evm +
                " balance changed " + (evmData.delta >= 0 ? "+" : "") + evmData.delta +
                " wei (user: " + username + ")"
              );
              _recordActivity(username, chain.name, addrs.evm, "balance_change", evmData, epoch);
              if (_ledger && typeof _ledger.recordCrossChainActivity === "function") {
                await _ledger.recordCrossChainActivity(
                  username, chain.name, addrs.evm, "balance_change", evmData, epoch
                );
              }
            } else if (evmResult.error) {
              _warn(chain.name + " RPC error for " + addrs.evm + ": " + evmResult.error);
            }
          } catch (err) {
            _warn(chain.name + " poll error for " + username + ": " + err.message);
          }
        }
      }

      // ── Solana ──────────────────────────────────────────────────
      if (addrs.solana) {
        try {
          var solResult = await solanaMonitor.checkAddress(addrs.solana);
          if (solResult.changed) {
            var solData = {
              old_balance: solResult.oldBalance,
              new_balance: solResult.newBalance,
              delta: solResult.newBalance - solResult.oldBalance,
            };
            _log(
              "solana: " + addrs.solana +
              " balance changed " + (solData.delta >= 0 ? "+" : "") + solData.delta +
              " lamports (user: " + username + ")"
            );
            _recordActivity(username, "solana", addrs.solana, "balance_change", solData, epoch);
            if (_ledger && typeof _ledger.recordCrossChainActivity === "function") {
              await _ledger.recordCrossChainActivity(
                username, "solana", addrs.solana, "balance_change", solData, epoch
              );
            }
          } else if (solResult.error) {
            _warn("solana RPC error for " + addrs.solana + ": " + solResult.error);
          }
        } catch (err) {
          _warn("solana poll error for " + username + ": " + err.message);
        }
      }

      // ── Bitcoin ─────────────────────────────────────────────────
      if (addrs.bitcoin) {
        try {
          var btcResult = await bitcoinMonitor.checkAddress(addrs.bitcoin);
          if (btcResult.changed) {
            var btcData = {
              old_funded_txo_sum: btcResult.oldSum,
              new_funded_txo_sum: btcResult.newSum,
              delta: btcResult.newSum - btcResult.oldSum,
            };
            _log(
              "bitcoin: " + addrs.bitcoin +
              " funded_txo_sum changed +" + btcData.delta +
              " sats (user: " + username + ")"
            );
            _recordActivity(username, "bitcoin", addrs.bitcoin, "balance_change", btcData, epoch);
            if (_ledger && typeof _ledger.recordCrossChainActivity === "function") {
              await _ledger.recordCrossChainActivity(
                username, "bitcoin", addrs.bitcoin, "balance_change", btcData, epoch
              );
            }
          } else if (btcResult.error) {
            _warn("bitcoin API error for " + addrs.bitcoin + ": " + btcResult.error);
          }
        } catch (err) {
          _warn("bitcoin poll error for " + username + ": " + err.message);
        }
      }
    }
  }

  function start() {
    if (_running) return;
    _running = true;
    _log("starting — poll interval=" + POLL_INTERVAL_MS + "ms");

    // Run first poll immediately, then schedule subsequent polls
    _pollAllAccounts().catch(function (err) {
      _warn("initial poll error: " + err.message);
    });

    if (shouldStartBackgroundTimers()) {
      _timer = setInterval(function () {
        _pollAllAccounts().catch(function (err) {
          _warn("poll error: " + err.message);
        });
      }, POLL_INTERVAL_MS);
    }
  }

  function stop() {
    _running = false;
    if (_timer) {
      clearInterval(_timer);
      _timer = null;
    }
    _log("stopped");
  }

  function getActivity(username, chain) {
    var list = activityLog.get(username) || [];
    if (chain) {
      return list.filter(function (a) { return a.chain === chain; });
    }
    return list.slice();
  }

  function resetForTests() {
    stop();
    activityLog.clear();
    evmMonitor.reset();
    solanaMonitor.reset();
    bitcoinMonitor.reset();
  }

  return {
    start: start,
    stop: stop,
    getActivity: getActivity,
    resetForTests: resetForTests,
    // Expose sub-monitors for unit testing
    _evmMonitor: evmMonitor,
    _solanaMonitor: solanaMonitor,
    _bitcoinMonitor: bitcoinMonitor,
    // Expose poll function for unit testing
    _pollAllAccounts: _pollAllAccounts,
  };
}

module.exports = {
  createChainMonitor: createChainMonitor,
  EVMMonitor: EVMMonitor,
  SolanaMonitor: SolanaMonitor,
  BitcoinMonitor: BitcoinMonitor,
  POLL_INTERVAL_MS: POLL_INTERVAL_MS,
  EVM_CHAINS: EVM_CHAINS,
};
