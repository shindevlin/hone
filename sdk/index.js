"use strict";

/**
 * @btcpc/sdk — Universal BTCPC client
 * Natoshi Sakamoto
 *
 * One import for any project that uses BTCPC:
 *
 *   const BTCPC = require('@btcpc/sdk');
 *   const btcpc = new BTCPC({ apiKey: process.env.BTCPC_API_KEY });
 *   const answer = await btcpc.ask({ prompt: 'What is 2+2?' });
 *
 * Auto-discovers the API, handles auth, claims faucet when needed,
 * and warns about common security mistakes in your code.
 *
 * Environment:
 *   BTCPC_API_URL   — API base URL (auto-discovers if not set)
 *   BTCPC_API_KEY   — project API key (btcpc_...)
 *   BTCPC_ACCOUNT   — account name
 *   BTCPC_MODEL     — preferred model (default: auto)
 */

const https = require("https");
const http = require("http");

const SDK_VERSION = "3.0.0";
const GENESIS_TIMESTAMP = 1776193200000; // 2026-04-14T13:30:00.000Z
const EPOCH_DURATION_MS = 30000;

// Auto-discover order
var DISCOVER_URLS = [
  "http://localhost:3000",
  "http://localhost:3100",
  "http://localhost:4242",
  "https://btcpc.net",
];

class BTCPC {
  /**
   * @param {Object} options
   * @param {string} [options.apiKey] — btcpc_ API key (env: BTCPC_API_KEY)
   * @param {string} [options.baseUrl] — API URL (env: BTCPC_API_URL, or auto-discover)
   * @param {string} [options.account] — account name (env: BTCPC_ACCOUNT)
   * @param {string} [options.model] — preferred model (env: BTCPC_MODEL)
   */
  constructor(options) {
    var opts = options || {};
    this.apiKey = opts.apiKey || process.env.BTCPC_API_KEY || process.env.BTCPC_PROJECT_KEY || null;
    this.baseUrl = (opts.baseUrl || process.env.BTCPC_API_URL || process.env.BTCPC_URL || process.env.BTCPC_NODE_URL || null);
    this.account = opts.account || process.env.BTCPC_ACCOUNT || process.env.BTCPC_MINER || null;
    this.model = opts.model || process.env.BTCPC_MODEL || null;
    this._discovered = false;

    // ── Security checks on init ──
    this._securityCheck(opts);
  }

  // ─── Security audit on init ─────────────────────────────────────

  _securityCheck(opts) {
    var warnings = [];

    // Check for hardcoded API keys (common vibe-coder mistake)
    if (opts.apiKey && opts.apiKey.startsWith("btcpc_") && opts.apiKey.length > 20) {
      // Check if the key appears to be a literal in source (not from env)
      try {
        var stack = new Error().stack || "";
        // Can't reliably detect this, but warn about best practice
      } catch (_) {}
      if (!process.env.BTCPC_API_KEY && !process.env.BTCPC_PROJECT_KEY) {
        warnings.push("API key passed directly — use BTCPC_API_KEY env var instead of hardcoding keys in source code");
      }
    }

    // Check for private keys in env (should NEVER be here)
    var dangerousEnvVars = [
      "BTCPC_PRIVATE_KEY", "BTCPC_OWNER_KEY", "BTCPC_ACTIVE_KEY",
      "BTCPC_POSTING_KEY", "BTCPC_MEMO_KEY", "BTCPC_MNEMONIC",
    ];
    for (var i = 0; i < dangerousEnvVars.length; i++) {
      if (process.env[dangerousEnvVars[i]]) {
        warnings.push("DANGER: " + dangerousEnvVars[i] + " found in environment. Private keys should NEVER be in env vars or code. Use a hardware wallet or btcpc.net/rotate.");
      }
    }

    // Check for .envbtcpc with secrets
    try {
      var fs = require("fs");
      var path = require("path");
      var envFile = path.resolve(process.cwd(), ".envbtcpc");
      if (fs.existsSync(envFile)) {
        var content = fs.readFileSync(envFile, "utf8");
        if (/private_key|mnemonic|seed/i.test(content)) {
          warnings.push("DANGER: .envbtcpc contains private keys or mnemonic. Move secrets to a hardware wallet. Never commit .envbtcpc to git.");
        }
        // Check if .envbtcpc is in .gitignore
        var gitignore = "";
        try { gitignore = fs.readFileSync(path.resolve(process.cwd(), ".gitignore"), "utf8"); } catch (_) {}
        if (!gitignore.includes(".envbtcpc")) {
          warnings.push("WARNING: .envbtcpc is not in .gitignore — secrets may be committed to git");
        }
      }
    } catch (_) {}

    // Check for common injection vulnerabilities
    if (this.account && /[;&|`$(){}]/.test(this.account)) {
      throw new Error("Account name contains shell metacharacters — possible injection attack");
    }

    for (var w = 0; w < warnings.length; w++) {
      console.warn("[btcpc-sdk] " + warnings[w]);
    }
  }

  // ─── Auto-discover API URL ──────────────────────────────────────

  async _discover() {
    if (this.baseUrl && this._discovered) return this.baseUrl;
    if (this.baseUrl) {
      this.baseUrl = this.baseUrl.replace(/\/+$/, "");
      this._discovered = true;
      return this.baseUrl;
    }

    for (var i = 0; i < DISCOVER_URLS.length; i++) {
      try {
        var res = await this._rawRequest("GET", DISCOVER_URLS[i] + "/health");
        if (res.status === 200) {
          this.baseUrl = DISCOVER_URLS[i];
          this._discovered = true;
          return this.baseUrl;
        }
      } catch (_) {}
    }

    throw new Error("BTCPC API not reachable. Set BTCPC_API_URL in your environment.");
  }

  // ─── Inference ──────────────────────────────────────────────────

  /**
   * Send a chat completion (OpenAI-compatible).
   */
  async chat(options) {
    var opts = options || {};
    var messages = opts.messages;
    if (opts.prompt && !messages) {
      messages = [{ role: "user", content: opts.prompt }];
    }
    if (opts.system) {
      messages = [{ role: "system", content: opts.system }].concat(messages || []);
    }
    if (!messages || messages.length === 0) {
      throw new Error("Either messages or prompt is required");
    }

    // Sanitize inputs
    for (var i = 0; i < messages.length; i++) {
      if (typeof messages[i].content !== "string") {
        throw new Error("Message content must be a string (index " + i + ")");
      }
      if (messages[i].content.length > 100000) {
        throw new Error("Message too long (max 100KB, index " + i + ")");
      }
    }

    var body = { messages: messages, stream: false };
    if (opts.model || this.model) body.model = opts.model || this.model;
    if (typeof opts.maxTokens === "number") body.max_tokens = opts.maxTokens;
    if (typeof opts.max_tokens === "number") body.max_tokens = opts.max_tokens;
    if (typeof opts.temperature === "number") body.temperature = opts.temperature;

    var res = await this._request("POST", "/v1/chat/completions", body);

    // Auto-claim faucet on insufficient balance
    if (res.status === 402 && this.account) {
      try {
        await this.faucetClaim();
        res = await this._request("POST", "/v1/chat/completions", body);
      } catch (_) {}
    }

    if (res.status >= 400) {
      var err = new Error((res.data && res.data.error && res.data.error.message) || "HTTP " + res.status);
      err.status = res.status;
      err.response = res.data;
      throw err;
    }

    return res.data;
  }

  /**
   * Convenience: get just the text response.
   */
  async ask(options) {
    var res = await this.chat(options);
    return (res.choices && res.choices[0] && res.choices[0].message && res.choices[0].message.content) || "";
  }

  // ─── Models ─────────────────────────────────────────────────────

  async models() {
    return (await this._request("GET", "/v1/models")).data;
  }

  async pricing(model) {
    var path = model ? "/v1/pricing?model=" + encodeURIComponent(model) : "/v1/pricing";
    return (await this._request("GET", path)).data;
  }

  // ─── Wallet ─────────────────────────────────────────────────────

  async balance(account) {
    account = account || this.account;
    if (!account) throw new Error("account required");
    var res = await this._request("GET", "/api/bot/balance?account=" + encodeURIComponent(account));
    return res.data;
  }

  async faucetClaim(account) {
    account = account || this.account;
    if (!account) throw new Error("account required");
    var res = await this._request("POST", "/api/faucet/claim", { account: account });
    return res.data;
  }

  // ─── Network ────────────────────────────────────────────────────

  async network() {
    return (await this._request("GET", "/public/network")).data;
  }

  currentEpoch() {
    var elapsed = Date.now() - GENESIS_TIMESTAMP;
    return elapsed > 0 ? Math.floor(elapsed / EPOCH_DURATION_MS) : 0;
  }

  // ─── Sensors ────────────────────────────────────────────────────

  async sensorReading(sensorId, value, metadata) {
    var account = sensorId.split("/")[0];
    return (await this._request("POST", "/api/sensors/" + encodeURIComponent(sensorId) + "/readings", {
      account: account, value: value, metadata: metadata || {},
    })).data;
  }

  async sensors() {
    return (await this._request("GET", "/api/sensors")).data;
  }

  async gateways() {
    return (await this._request("GET", "/api/gateways")).data;
  }

  // ─── Projects ───────────────────────────────────────────────────

  async project() {
    return (await this._request("GET", "/api/projects/me")).data;
  }

  /**
   * Set revenue split for a project.
   * @param {string} project — project name
   * @param {Array} splits — [{ account: "user", percent: 70 }, ...]
   */
  async setRevenueSplit(project, splits) {
    var total = 0;
    for (var i = 0; i < splits.length; i++) total += (splits[i].percent || 0);
    if (Math.abs(total - 100) > 0.01) {
      throw new Error("Revenue splits must total 100% (got " + total + "%)");
    }
    return (await this._request("POST", "/api/projects/revenue-split", {
      project: project, splits: splits,
    })).data;
  }

  // ─── Version / Health ───────────────────────────────────────────

  async version() {
    await this._discover();
    return {
      sdk: SDK_VERSION,
      api_url: this.baseUrl,
      epoch: this.currentEpoch(),
      genesis: new Date(GENESIS_TIMESTAMP).toISOString(),
    };
  }

  // ─── HTTP internals ─────────────────────────────────────────────

  async _request(method, path, body) {
    await this._discover();
    return this._rawRequest(method, this.baseUrl + path, body);
  }

  _rawRequest(method, url, body) {
    var self = this;
    return new Promise(function (resolve, reject) {
      var parsed = new URL(url);
      var transport = parsed.protocol === "https:" ? https : http;
      var payload = body ? JSON.stringify(body) : null;

      var headers = {
        "Content-Type": "application/json",
        "User-Agent": "@btcpc/sdk/" + SDK_VERSION,
      };
      if (self.apiKey) headers["Authorization"] = "Bearer " + self.apiKey;
      if (payload) headers["Content-Length"] = Buffer.byteLength(payload);

      var req = transport.request({
        hostname: parsed.hostname,
        port: parsed.port || (parsed.protocol === "https:" ? 443 : 80),
        path: parsed.pathname + parsed.search,
        method: method,
        headers: headers,
        timeout: 600000,
      }, function (res) {
        var data = "";
        res.on("data", function (chunk) { data += chunk; });
        res.on("end", function () {
          var json = null;
          try { json = JSON.parse(data); } catch (_) {}
          resolve({ status: res.statusCode, data: json, raw: data });
        });
      });

      req.on("error", reject);
      req.on("timeout", function () { req.destroy(); reject(new Error("Request timed out (180s)")); });
      if (payload) req.write(payload);
      req.end();
    });
  }
}

// ─── Exports ──────────────────────────────────────────────────────

module.exports = BTCPC;
module.exports.BTCPC = BTCPC;
module.exports.default = BTCPC;
module.exports.SDK_VERSION = SDK_VERSION;
module.exports.GENESIS_TIMESTAMP = GENESIS_TIMESTAMP;
module.exports.EPOCH_DURATION_MS = EPOCH_DURATION_MS;
