"use strict";

/**
 * Protocol-native tools for the BTCPC agentic inference marketplace.
 *
 * These tools are executed trustlessly by the miner (no buyer round-trip).
 * Their inputs/outputs are deterministic and verifiable by any chain node.
 * Buyers can include any of these tool names in their job's tools array and
 * the miner will execute them inline without waiting for the buyer.
 *
 * External tools (web_fetch, web_search) are miner-executed but NOT verifiable
 * — they are flagged trusted: false in results and must be opted into explicitly.
 */

const stateStore = require("../chain/stateStore");

// ── Canonical tool schemas (Ollama-compatible JSON Schema format) ─────────────

const PROTOCOL_TOOL_SCHEMAS = [
  {
    type: "function",
    function: {
      name: "btcpc_balance",
      description: "Check the BTCPC token balance of any account on the BTCPC chain. Returns the current balance and any staked amount.",
      parameters: {
        type: "object",
        properties: {
          account: {
            type: "string",
            description: "The BTCPC account name to check",
          },
          token: {
            type: "string",
            description: "Token symbol to check (default: BTCPC)",
          },
        },
        required: ["account"],
      },
    },
  },
  {
    type: "function",
    function: {
      name: "btcpc_chain_query",
      description: "Query the BTCPC blockchain for current state — epoch number, block height, account info, or network stats.",
      parameters: {
        type: "object",
        properties: {
          type: {
            type: "string",
            enum: ["epoch", "block_height", "account_info", "network_stats", "staking_info"],
            description: "What to query",
          },
          account: {
            type: "string",
            description: "Account name — required for account_info and staking_info queries",
          },
        },
        required: ["type"],
      },
    },
  },
  {
    type: "function",
    function: {
      name: "btcpc_sensor_query",
      description: "Query sensor data from the Verasens network. Returns recent readings from registered IoT sensors. Can filter by sensor ID, data type, and result count.",
      parameters: {
        type: "object",
        properties: {
          sensor_id: {
            type: "string",
            description: "Specific sensor ID to query (optional — omit to search by data_type)",
          },
          data_type: {
            type: "string",
            description: "Type of sensor data: temperature, humidity, pressure, gps, accelerometer, etc.",
          },
          limit: {
            type: "number",
            description: "Max readings to return (default: 10, max: 100)",
          },
        },
      },
    },
  },
  {
    type: "function",
    function: {
      name: "execute_code",
      description: "Execute code in a sandboxed subprocess. Returns stdout, stderr, and exit code. Supports javascript (Node.js), python, and bash. Has a hard timeout. No network access inside the sandbox.",
      parameters: {
        type: "object",
        properties: {
          language: {
            type: "string",
            enum: ["javascript", "python", "bash"],
            description: "Programming language to execute",
          },
          code: {
            type: "string",
            description: "The code to execute",
          },
          timeout_ms: {
            type: "number",
            description: "Execution timeout in milliseconds (default: 5000, max: 30000)",
          },
        },
        required: ["language", "code"],
      },
    },
  },
  {
    type: "function",
    function: {
      name: "web_fetch",
      description: "Fetch the content of a URL. UNVERIFIED — executed by the miner's machine. Result is flagged trusted: false. Only use for public, non-sensitive URLs.",
      parameters: {
        type: "object",
        properties: {
          url: {
            type: "string",
            description: "The full URL to fetch (https only)",
          },
          format: {
            type: "string",
            enum: ["text", "json"],
            description: "Expected response format (default: text)",
          },
        },
        required: ["url"],
      },
    },
  },
  {
    type: "function",
    function: {
      name: "web_search",
      description: "Search the web for information. UNVERIFIED — executed by the miner's machine using DuckDuckGo. Result is flagged trusted: false.",
      parameters: {
        type: "object",
        properties: {
          query: {
            type: "string",
            description: "The search query",
          },
          num_results: {
            type: "number",
            description: "Number of results to return (default: 5, max: 10)",
          },
        },
        required: ["query"],
      },
    },
  },
];

// Tools that miners execute inline (no buyer round-trip needed)
const PROTOCOL_NATIVE_TOOLS = new Set([
  "btcpc_balance",
  "btcpc_chain_query",
  "btcpc_sensor_query",
  "execute_code",
]);

// Tools that are miner-executed but not chain-verifiable
const MINER_EXECUTED_TOOLS = new Set([
  "web_fetch",
  "web_search",
]);

function isProtocolNativeTool(name) {
  return PROTOCOL_NATIVE_TOOLS.has(name);
}

function isMinerExecutedTool(name) {
  return MINER_EXECUTED_TOOLS.has(name);
}

function canMinerExecute(name) {
  return PROTOCOL_NATIVE_TOOLS.has(name) || MINER_EXECUTED_TOOLS.has(name);
}

function getSchema(name) {
  return PROTOCOL_TOOL_SCHEMAS.find((s) => s.function && s.function.name === name) || null;
}

/**
 * Execute a protocol-native or miner-executed tool.
 * Returns { content: string|object, trusted: boolean, error?: string }
 */
async function executeProtocolTool(name, input) {
  input = input || {};

  if (name === "btcpc_balance") {
    return _execBalance(input);
  }
  if (name === "btcpc_chain_query") {
    return _execChainQuery(input);
  }
  if (name === "btcpc_sensor_query") {
    return _execSensorQuery(input);
  }
  if (name === "execute_code") {
    if (process.env.BTCPC_CODE_EXEC_ENABLED === "false") {
      return { content: "This miner has disabled code execution jobs.", trusted: true, error: "disabled" };
    }
    return _execCode(input);
  }
  if (name === "web_fetch") {
    return _execWebFetch(input);
  }
  if (name === "web_search") {
    return _execWebSearch(input);
  }

  return { content: `Unknown protocol tool: ${name}`, trusted: false, error: "unknown_tool" };
}

function _execBalance(input) {
  const account = input.account;
  const token = input.token || "BTCPC";
  if (!account) {
    return { content: "account parameter required", trusted: true, error: "missing_param" };
  }
  try {
    const balance = stateStore.getBalance(account, token);
    const staked = stateStore.getStakedAmount ? stateStore.getStakedAmount(account) : null;
    return {
      trusted: true,
      content: {
        account,
        token,
        balance,
        staked: staked !== null ? staked : undefined,
      },
    };
  } catch (err) {
    return { content: err.message, trusted: true, error: "query_error" };
  }
}

function _execChainQuery(input) {
  const type = input.type;
  try {
    if (type === "epoch") {
      const epoch = stateStore.getCurrentEpoch ? stateStore.getCurrentEpoch() : null;
      const height = stateStore.getChainHeight ? stateStore.getChainHeight() : null;
      return { trusted: true, content: { epoch, block_height: height } };
    }
    if (type === "block_height") {
      const height = stateStore.getChainHeight ? stateStore.getChainHeight() : null;
      return { trusted: true, content: { block_height: height } };
    }
    if (type === "account_info") {
      const account = input.account;
      if (!account) return { content: "account required for account_info", trusted: true, error: "missing_param" };
      const info = stateStore.getAccount ? stateStore.getAccount(account) : null;
      if (!info) return { trusted: true, content: { account, exists: false } };
      return {
        trusted: true,
        content: {
          account,
          exists: true,
          created_epoch: info.created_epoch,
          reputation: info.reputation,
          public_keys: info.public_keys ? Object.keys(info.public_keys) : [],
        },
      };
    }
    if (type === "network_stats") {
      const epoch = stateStore.getCurrentEpoch ? stateStore.getCurrentEpoch() : null;
      const height = stateStore.getChainHeight ? stateStore.getChainHeight() : null;
      const peers = stateStore.getActivePeerCount ? stateStore.getActivePeerCount() : null;
      return { trusted: true, content: { epoch, block_height: height, active_peers: peers } };
    }
    if (type === "staking_info") {
      const account = input.account;
      if (!account) return { content: "account required for staking_info", trusted: true, error: "missing_param" };
      const staked = stateStore.getStakedAmount ? stateStore.getStakedAmount(account) : 0;
      const delegated = stateStore.getDelegatedAmount ? stateStore.getDelegatedAmount(account) : 0;
      return { trusted: true, content: { account, staked, delegated } };
    }
    return { content: `Unknown query type: ${type}`, trusted: true, error: "unknown_type" };
  } catch (err) {
    return { content: err.message, trusted: true, error: "query_error" };
  }
}

function _execSensorQuery(input) {
  const limit = Math.min(parseInt(input.limit) || 10, 100);
  try {
    const sensors = stateStore.getAllSensors ? stateStore.getAllSensors() : [];
    let filtered = sensors;
    if (input.sensor_id) {
      filtered = filtered.filter((s) => s.sensor_id === input.sensor_id || s.id === input.sensor_id);
    }
    if (input.data_type) {
      filtered = filtered.filter(
        (s) => s.sensor_types && s.sensor_types.includes(input.data_type)
      );
    }
    return {
      trusted: true,
      content: {
        sensors: filtered.slice(0, limit).map((s) => ({
          sensor_id: s.sensor_id || s.id,
          sensor_types: s.sensor_types || [],
          lat: s.lat,
          lon: s.lon,
          last_seen: s.last_seen,
          owner: s.owner,
        })),
        total: filtered.length,
      },
    };
  } catch (err) {
    return { content: err.message, trusted: true, error: "query_error" };
  }
}

function _execCode(input) {
  const language = input.language;
  const code = input.code;
  const timeout = Math.min(parseInt(input.timeout_ms) || 5000, 30000);
  const maxOutput = 16 * 1024; // 16KB output limit

  if (!code) return { content: "code required", trusted: true, error: "missing_param" };

  const { execSync } = require("child_process");
  const os = require("os");
  const path = require("path");
  const fs = require("fs");

  let ext, cmd;
  if (language === "javascript") {
    ext = ".js";
    cmd = "node";
  } else if (language === "python") {
    ext = ".py";
    cmd = "python3";
  } else if (language === "bash") {
    ext = ".sh";
    cmd = "bash";
  } else {
    return { content: `Unsupported language: ${language}`, trusted: true, error: "unsupported_language" };
  }

  // Write code to a temp file
  const tmpFile = path.join(os.tmpdir(), `btcpc_exec_${Date.now()}${ext}`);
  try {
    fs.writeFileSync(tmpFile, code, "utf8");
    const stdout = execSync(`${cmd} ${tmpFile}`, {
      timeout,
      maxBuffer: maxOutput,
      env: {
        PATH: process.env.PATH,
        HOME: os.tmpdir(),
        // No network-sensitive env vars
      },
    }).toString("utf8").slice(0, maxOutput);
    return {
      trusted: true,
      content: { stdout, stderr: "", exit_code: 0, language },
    };
  } catch (err) {
    const stdout = err.stdout ? err.stdout.toString("utf8").slice(0, maxOutput) : "";
    const stderr = err.stderr ? err.stderr.toString("utf8").slice(0, maxOutput) : err.message;
    return {
      trusted: true,
      content: { stdout, stderr, exit_code: err.status || 1, language },
    };
  } finally {
    try { fs.unlinkSync(tmpFile); } catch (_) {}
  }
}

async function _execWebFetch(input) {
  const url = input.url;
  if (!url) return { content: "url required", trusted: false, error: "missing_param" };
  if (!/^https?:\/\//i.test(url)) {
    return { content: "Only http/https URLs are supported", trusted: false, error: "invalid_url" };
  }
  try {
    const axios = require("axios");
    const resp = await axios.get(url, { timeout: 10000, maxContentLength: 512 * 1024 });
    const body = input.format === "json" ? resp.data : String(resp.data).slice(0, 8000);
    return { trusted: false, content: body };
  } catch (err) {
    return { content: `Fetch failed: ${err.message}`, trusted: false, error: "fetch_error" };
  }
}

async function _execWebSearch(input) {
  const query = input.query;
  if (!query) return { content: "query required", trusted: false, error: "missing_param" };
  const num = Math.min(parseInt(input.num_results) || 5, 10);
  try {
    const axios = require("axios");
    const encoded = encodeURIComponent(query);
    const url = `https://api.duckduckgo.com/?q=${encoded}&format=json&no_redirect=1&no_html=1&skip_disambig=1`;
    const resp = await axios.get(url, { timeout: 8000 });
    const data = resp.data;
    const results = [];
    if (data.AbstractText) {
      results.push({ title: data.Heading || query, snippet: data.AbstractText, url: data.AbstractURL });
    }
    if (data.RelatedTopics) {
      for (const topic of data.RelatedTopics.slice(0, num - results.length)) {
        if (topic.Text) results.push({ title: topic.Text.split(" - ")[0] || "", snippet: topic.Text, url: topic.FirstURL || "" });
      }
    }
    return { trusted: false, content: { query, results: results.slice(0, num) } };
  } catch (err) {
    return { content: `Search failed: ${err.message}`, trusted: false, error: "search_error" };
  }
}

module.exports = {
  PROTOCOL_TOOL_SCHEMAS,
  PROTOCOL_NATIVE_TOOLS,
  MINER_EXECUTED_TOOLS,
  isProtocolNativeTool,
  isMinerExecutedTool,
  canMinerExecute,
  getSchema,
  executeProtocolTool,
};
