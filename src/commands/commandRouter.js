"use strict";

/**
 * BTCPC Command Router
 * Shin Devlin
 *
 * Unified slash-command interface for all BTCPC engines.
 * Used by: Telegram bot, CLI, future chat interface, in-node REPL.
 *
 * Format: /btcpc <engine> <action> [args...]
 * Short:  /btcpc <action> [args...]   (for top-level shortcuts)
 *
 * Engine map:
 *   mine    — inference / mining engine
 *   verify  — verifier engine
 *   clock   — clock consensus engine
 *   store   — BTCPC-FS storage engine
 *   sensor  — sensor data engine
 *   tool    — MCP / CLI tool registry
 *   infer   — raw inference (tool-augmented)
 *   wallet  — balance, send, stake
 *   chain   — status, blocks, epoch, supply, peers, fork
 *   node    — node registration, stake, heartbeat
 *
 * Each handler receives (args: string[], context: CommandContext) and returns
 * { text: string, data?: object } or throws with a user-readable message.
 *
 * context: {
 *   account: string,        — caller's BTCPC account name
 *   epoch: number,          — current epoch
 *   apiBase: string,        — local node API base URL
 *   sendLedger?: fn,        — optional: submit a ledger entry (for bot/CLI contexts)
 * }
 */

const axios = require("axios").default;

// ── Helpers ───────────────────────────────────────────────────────────────────

function api(context, path, opts) {
  const base = context.apiBase || "http://localhost:4242";
  return axios({ url: base + path, timeout: 10000, ...opts })
    .then(r => r.data)
    .catch(e => { throw new Error((e.response && e.response.data && e.response.data.error) || e.message); });
}

function fmt(n, decimals) {
  return Number(n).toFixed(decimals === undefined ? 4 : decimals);
}

function argErr(usage) {
  throw new Error("Usage: " + usage);
}

// ── Engine handlers ───────────────────────────────────────────────────────────

const engines = {};

// ── /btcpc mine ───────────────────────────────────────────────────────────────
engines.mine = {
  _help: `
mine status                     — show miner status and active model
mine model <name>               — switch mining model (e.g. qwen2.5:14b)
mine start [--model <name>]     — start the mining daemon
mine stop                       — stop the mining daemon
mine proof <id>                 — check status of a submitted proof
mine proofs [count]             — list recent proofs (default 5)
mine earnings [account]         — MINING_REWARD entries for account`,

  status: async (args, ctx) => {
    const data = await api(ctx, "/status");
    const mining = data.mining || {};
    return {
      text: [
        `**Mining status**`,
        `Model: ${mining.model || "none"}`,
        `Active: ${mining.active ? "yes" : "no"}`,
        `Epoch: ${data.chain_height || "?"}`,
        `Pending proofs: ${mining.pending_proofs || 0}`,
      ].join("\n"),
      data,
    };
  },

  model: async (args, ctx) => {
    const name = args[0];
    if (!name) argErr("/btcpc mine model <name>");
    const data = await api(ctx, "/api/mine/model", { method: "POST", data: { model: name } });
    return { text: `Mining model set to **${name}**`, data };
  },

  start: async (args, ctx) => {
    const model = args.includes("--model") ? args[args.indexOf("--model") + 1] : null;
    const data = await api(ctx, "/api/mine/start", { method: "POST", data: { model } });
    return { text: `Mining started${model ? " with model " + model : ""}`, data };
  },

  stop: async (args, ctx) => {
    const data = await api(ctx, "/api/mine/stop", { method: "POST" });
    return { text: "Mining stopped", data };
  },

  proof: async (args, ctx) => {
    const id = args[0];
    if (!id) argErr("/btcpc mine proof <proof_id>");
    const data = await api(ctx, "/api/proof/" + id);
    const p = data.proof || data;
    return {
      text: [
        `**Proof ${id.slice(0, 12)}...**`,
        `Node: ${p.node_id || "?"}`,
        `Status: ${p.status || "?"}`,
        `Work value: ${fmt(p.work_value || 0, 2)}`,
        `Submitted epoch: ${p.submitted_epoch || "?"}`,
        `Deadline epoch: ${p.deadline_epoch || "?"}`,
      ].join("\n"),
      data,
    };
  },

  proofs: async (args, ctx) => {
    const count = parseInt(args[0]) || 5;
    const data = await api(ctx, "/api/proofs?limit=" + count);
    const proofs = data.proofs || data || [];
    if (!proofs.length) return { text: "No recent proofs", data };
    return {
      text: "**Recent proofs**\n" + proofs.map(p =>
        `  ${(p.id || "").slice(0, 10)}... | ${p.status} | work=${fmt(p.work_value || 0, 1)} | ep ${p.submitted_epoch || "?"}`
      ).join("\n"),
      data,
    };
  },

  earnings: async (args, ctx) => {
    const account = args[0] || ctx.account;
    if (!account) argErr("/btcpc mine earnings <account>");
    const data = await api(ctx, "/account/" + account + "/history?type=MINING_REWARD");
    const entries = data.entries || data || [];
    const total = entries.reduce((s, e) => s + (e.amount || 0), 0);
    return {
      text: `**Mining earnings for ${account}**\nTotal: ${fmt(total)} BTCPC (${entries.length} rewards)`,
      data,
    };
  },
};

// ── /btcpc verify ─────────────────────────────────────────────────────────────
engines.verify = {
  _help: `
verify proof <proof_id>         — manually trigger verification of a proof
verify challenge <id> <reason>  — challenge a proof (reasons: invalid_result, model_mismatch, replay, low_quality)
verify proposals [epoch]        — show reward proposals for an epoch
verify status                   — verifier engine status`,

  proof: async (args, ctx) => {
    const id = args[0];
    if (!id) argErr("/btcpc verify proof <proof_id>");
    const data = await api(ctx, "/api/verify/" + id, { method: "POST" });
    return { text: `Verification triggered for proof ${id.slice(0, 12)}...`, data };
  },

  challenge: async (args, ctx) => {
    const [id, reason] = args;
    if (!id || !reason) argErr("/btcpc verify challenge <proof_id> <reason>");
    const data = await api(ctx, "/api/verify/challenge", {
      method: "POST",
      data: { proof_id: id, challenger: ctx.account, reason },
    });
    return { text: `Challenge submitted against ${id.slice(0, 12)}... (reason: ${reason})`, data };
  },

  proposals: async (args, ctx) => {
    const epoch = parseInt(args[0]) || ctx.epoch;
    const data = await api(ctx, "/api/verify/proposals?epoch=" + epoch);
    const proposals = data.proposals || [];
    if (!proposals.length) return { text: `No proposals for epoch ${epoch}`, data };
    return {
      text: "**Reward proposals epoch " + epoch + "**\n" + proposals.map(p =>
        `  ${p.verifier} | weight=${fmt(p.proposal_weight || 0, 2)} | hash=${(p.reward_hash || "").slice(0, 12)}...`
      ).join("\n"),
      data,
    };
  },

  status: async (args, ctx) => {
    const data = await api(ctx, "/api/verify/status");
    return {
      text: [
        `**Verifier engine**`,
        `Active: ${data.active ? "yes" : "no"}`,
        `Pool size: ${data.pool_size || 0} proofs`,
        `Score: ${data.score || 0}/200`,
      ].join("\n"),
      data,
    };
  },
};

// ── /btcpc clock ──────────────────────────────────────────────────────────────
engines.clock = {
  _help: `
clock status                    — clock node status, observer mode, peer count
clock seal [epoch]              — force-produce a clock seal for an epoch
clock peers                     — list clock-capable peers seen recently`,

  status: async (args, ctx) => {
    const data = await api(ctx, "/api/clock/status");
    return {
      text: [
        `**Clock node**`,
        `Observer mode: ${data.observer_mode ? "YES (isolated)" : "no"}`,
        `External peers: ${data.external_peers || 0}`,
        `Current epoch: ${data.current_epoch || "?"}`,
        `Clock score: ${data.score || 0}/200`,
        `Seals produced: ${data.seals_produced || 0}`,
      ].join("\n"),
      data,
    };
  },

  seal: async (args, ctx) => {
    const epoch = parseInt(args[0]) || ctx.epoch;
    const data = await api(ctx, "/api/clock/seal", { method: "POST", data: { epoch } });
    return { text: `Clock seal produced for epoch ${epoch}`, data };
  },

  peers: async (args, ctx) => {
    const data = await api(ctx, "/api/clock/peers");
    const peers = data.peers || [];
    if (!peers.length) return { text: "No clock peers seen recently", data };
    return {
      text: "**Clock peers**\n" + peers.map(p => `  ${p.account} | score=${p.score || 0} | last_epoch=${p.last_epoch || "?"}`).join("\n"),
      data,
    };
  },
};

// ── /btcpc store ──────────────────────────────────────────────────────────────
engines.store = {
  _help: `
store put <file>                — store a local file on BTCPC-FS, returns CID
store get <cid>                 — fetch blob by CID to stdout
store list [account]            — list stored blobs for account
store pin <cid>                 — pin a CID (keep it from garbage collection)
store info <cid>                — metadata for a stored blob`,

  put: async (args, ctx) => {
    const file = args[0];
    if (!file) argErr("/btcpc store put <file>");
    const data = await api(ctx, "/api/store/put", { method: "POST", data: { file_path: file, account: ctx.account } });
    return { text: `Stored **${file}**\nCID: \`${data.cid}\`\nSize: ${data.size_bytes || "?"} bytes`, data };
  },

  get: async (args, ctx) => {
    const cid = args[0];
    if (!cid) argErr("/btcpc store get <cid>");
    const data = await api(ctx, "/api/store/get/" + cid);
    return { text: `Blob CID \`${cid}\`\nSize: ${data.size_bytes || "?"}`, data };
  },

  list: async (args, ctx) => {
    const account = args[0] || ctx.account;
    const data = await api(ctx, "/api/store/list?account=" + account);
    const blobs = data.blobs || [];
    if (!blobs.length) return { text: `No stored blobs for ${account}`, data };
    return {
      text: `**Stored blobs for ${account}**\n` + blobs.slice(0, 10).map(b =>
        `  ${b.cid.slice(0, 16)}... | ${b.size_bytes || "?"}B | ep ${b.epoch || "?"}`
      ).join("\n"),
      data,
    };
  },

  pin: async (args, ctx) => {
    const cid = args[0];
    if (!cid) argErr("/btcpc store pin <cid>");
    const data = await api(ctx, "/api/store/pin", { method: "POST", data: { cid, account: ctx.account } });
    return { text: `Pinned \`${cid}\``, data };
  },

  info: async (args, ctx) => {
    const cid = args[0];
    if (!cid) argErr("/btcpc store info <cid>");
    const data = await api(ctx, "/api/store/info/" + cid);
    return {
      text: [
        `**Blob ${cid.slice(0, 16)}...**`,
        `Size: ${data.size_bytes || "?"}B`,
        `Owner: ${data.owner || "?"}`,
        `Epoch: ${data.epoch || "?"}`,
        `Pinned: ${data.pinned ? "yes" : "no"}`,
      ].join("\n"),
      data,
    };
  },
};

// ── /btcpc sensor ─────────────────────────────────────────────────────────────
engines.sensor = {
  _help: `
sensor list [account]           — list registered sensors
sensor register <id> <type>     — register a sensor (id: account/device-name, type: gnss|temp|humidity|...)
sensor submit <id> <value>      — submit a reading for the current epoch
sensor purchase <hash> <btcpc>  — buy a sensor reading (by data_hash)
sensor earnings [account]       — SENSOR_REWARD entries for account
sensor readings [sensor_id]     — available readings for purchase`,

  list: async (args, ctx) => {
    const account = args[0] || ctx.account;
    const data = await api(ctx, "/api/sensors?account=" + account);
    const sensors = data.sensors || [];
    if (!sensors.length) return { text: `No sensors for ${account}`, data };
    return {
      text: `**Sensors for ${account}**\n` + sensors.map(s =>
        `  ${s.sensor_id} | type=${s.sensor_type || "?"} | readings=${s.total_readings || 0}`
      ).join("\n"),
      data,
    };
  },

  register: async (args, ctx) => {
    const [id, type] = args;
    if (!id || !type) argErr("/btcpc sensor register <id> <type>");
    const data = await api(ctx, "/api/sensors/register", { method: "POST", data: { sensor_id: id, sensor_type: type, account: ctx.account } });
    return { text: `Sensor **${id}** registered (type: ${type})`, data };
  },

  submit: async (args, ctx) => {
    const [id, value] = args;
    if (!id || value === undefined) argErr("/btcpc sensor submit <sensor_id> <value>");
    const data = await api(ctx, "/api/sensors/submit", { method: "POST", data: { sensor_id: id, value: parseFloat(value), account: ctx.account } });
    return { text: `Reading submitted for **${id}**: ${value}\nData hash: \`${(data.data_hash || "").slice(0, 16)}...\``, data };
  },

  purchase: async (args, ctx) => {
    const [hash, amount] = args;
    if (!hash || !amount) argErr("/btcpc sensor purchase <data_hash> <btcpc_amount>");
    const data = await api(ctx, "/api/sensors/purchase", {
      method: "POST",
      data: { data_hash: hash, buyer: ctx.account, payment_btcpc: parseFloat(amount) },
    });
    return { text: `Purchase submitted\nHash: \`${hash.slice(0, 16)}...\`\nAmount: ${amount} BTCPC`, data };
  },

  earnings: async (args, ctx) => {
    const account = args[0] || ctx.account;
    if (!account) argErr("/btcpc sensor earnings <account>");
    const data = await api(ctx, "/account/" + account + "/history?type=SENSOR_REWARD,SENSOR_EPOCH_REWARD");
    const entries = data.entries || [];
    const total = entries.reduce((s, e) => s + (e.amount || 0), 0);
    return { text: `**Sensor earnings for ${account}**\nTotal: ${fmt(total)} BTCPC (${entries.length} rewards)`, data };
  },

  readings: async (args, ctx) => {
    const prefix = args[0] || "";
    const data = await api(ctx, "/api/sensors/readings" + (prefix ? "?sensor_id_prefix=" + prefix : ""));
    const readings = data.readings || [];
    if (!readings.length) return { text: "No readings available", data };
    return {
      text: "**Available sensor readings**\n" + readings.slice(0, 10).map(r =>
        `  ${r.sensor_id} | hash=${r.data_hash.slice(0, 12)}... | ep ${r.reading_epoch}`
      ).join("\n"),
      data,
    };
  },
};

// ── /btcpc tool ───────────────────────────────────────────────────────────────
engines.tool = {
  _help: `
tool list                       — list all available tools (builtins + registered)
tool register <name> <cmd>      — register a CLI tool (cmd can use {{arg}} placeholders)
tool mcp start                  — start local MCP server
tool mcp stop                   — stop local MCP server
tool mcp status                 — MCP server health
tool test <name> [json_input]   — test a tool locally with JSON input
tool nodes                      — list nodes with registered tool capabilities`,

  list: async (args, ctx) => {
    const data = await api(ctx, "/api/tools/list");
    const tools = data.tools || [];
    if (!tools.length) return { text: "No tools registered", data };
    return {
      text: "**Available tools**\n" + tools.map(t =>
        `  ${t.name} | kind=${t.kind} | deterministic=${t.deterministic ? "yes" : "no"} | ${t.multiplier ? "×" + t.multiplier : ""}`
      ).join("\n"),
      data,
    };
  },

  register: async (args, ctx) => {
    const [name, ...rest] = args;
    const command = rest.join(" ");
    if (!name || !command) argErr("/btcpc tool register <name> <shell_command>");
    const data = await api(ctx, "/api/tools/register", {
      method: "POST",
      data: { name, command, kind: "cli", account: ctx.account },
    });
    return { text: `Tool **${name}** registered\nCommand: \`${command}\``, data };
  },

  mcp: async (args, ctx) => {
    const action = args[0] || "status";
    const data = await api(ctx, "/api/tools/mcp/" + action, { method: action === "status" ? "GET" : "POST" });
    if (action === "status") {
      return { text: `MCP server: ${data.ok ? "running" : "stopped"} | port=${data.port || "?"} | tools=${data.tools || 0}`, data };
    }
    return { text: `MCP server ${action}ed`, data };
  },

  test: async (args, ctx) => {
    const name = args[0];
    let input = {};
    if (args[1]) { try { input = JSON.parse(args.slice(1).join(" ")); } catch (_) { input = { value: args[1] }; } }
    if (!name) argErr("/btcpc tool test <name> [json_input]");
    const data = await api(ctx, "/api/tools/call", { method: "POST", data: { tool: name, input } });
    return {
      text: `**Tool ${name} result**\n\`\`\`\n${JSON.stringify(data.output, null, 2).slice(0, 1024)}\n\`\`\``,
      data,
    };
  },

  nodes: async (args, ctx) => {
    const data = await api(ctx, "/api/tools/nodes");
    const nodes = data.nodes || [];
    if (!nodes.length) return { text: "No nodes with registered tools", data };
    return {
      text: "**Nodes with tools**\n" + nodes.map(n =>
        `  ${n.node_id} | tools=${n.tools.map(t => t.name).join(",")}`
      ).join("\n"),
      data,
    };
  },
};

// ── /btcpc infer ──────────────────────────────────────────────────────────────
engines.infer = {
  _help: `
infer <prompt>                  — run inference with default tools
infer <prompt> --model <name>   — specify model
infer <prompt> --tools <t1,t2>  — specify tools (comma-separated)
infer <prompt> --node <id>      — route to specific node`,

  _default: async (args, ctx) => {
    let prompt = [];
    let model = null;
    let tools = null;
    let node = null;

    for (let i = 0; i < args.length; i++) {
      if (args[i] === "--model" && args[i + 1]) { model = args[++i]; }
      else if (args[i] === "--tools" && args[i + 1]) { tools = args[++i].split(","); }
      else if (args[i] === "--node" && args[i + 1]) { node = args[++i]; }
      else { prompt.push(args[i]); }
    }

    const promptStr = prompt.join(" ");
    if (!promptStr) argErr("/btcpc infer <prompt> [--model name] [--tools t1,t2]");

    const data = await api(ctx, "/api/inference", {
      method: "POST",
      data: { prompt: promptStr, model, required_tools: tools, preferred_node: node, requester: ctx.account },
    });
    return {
      text: [
        `**Inference result**`,
        data.output || data.result || "(no output)",
        data.tool_trace_hash ? `Tool trace: \`${data.tool_trace_hash.slice(0, 16)}...\`` : "",
        `Model: ${data.model || "?"} | Work value: ${fmt(data.work_value || 0, 2)}`,
      ].filter(Boolean).join("\n"),
      data,
    };
  },
};

// ── /btcpc wallet ─────────────────────────────────────────────────────────────
engines.wallet = {
  _help: `
wallet balance [account]        — show BTCPC balance
wallet send <to> <amount>       — send BTCPC
wallet stake <amount>           — stake BTCPC as a node operator
wallet unstake <amount>         — unstake BTCPC
wallet history [account]        — recent transactions`,

  balance: async (args, ctx) => {
    const account = args[0] || ctx.account;
    if (!account) argErr("/btcpc wallet balance <account>");
    const data = await api(ctx, "/account/" + account);
    return { text: `**${account}** balance: ${fmt(data.balance || 0)} BTCPC`, data };
  },

  send: async (args, ctx) => {
    const [to, amount] = args;
    if (!to || !amount) argErr("/btcpc wallet send <to> <amount>");
    const data = await api(ctx, "/api/send", { method: "POST", data: { from: ctx.account, to, amount: parseFloat(amount) } });
    return { text: `Sent ${amount} BTCPC to **${to}**\nTx: \`${(data.tx_id || "").slice(0, 16)}...\``, data };
  },

  stake: async (args, ctx) => {
    const amount = parseFloat(args[0]);
    if (!amount) argErr("/btcpc wallet stake <amount>");
    const data = await api(ctx, "/api/stake", { method: "POST", data: { account: ctx.account, amount } });
    return { text: `Staked ${amount} BTCPC`, data };
  },

  unstake: async (args, ctx) => {
    const amount = parseFloat(args[0]);
    if (!amount) argErr("/btcpc wallet unstake <amount>");
    const data = await api(ctx, "/api/unstake", { method: "POST", data: { account: ctx.account, amount } });
    return { text: `Unstaked ${amount} BTCPC`, data };
  },

  history: async (args, ctx) => {
    const account = args[0] || ctx.account;
    const data = await api(ctx, "/account/" + account + "/history?limit=10");
    const entries = data.entries || [];
    if (!entries.length) return { text: `No history for ${account}`, data };
    return {
      text: `**Recent transactions for ${account}**\n` + entries.slice(0, 10).map(e =>
        `  ${e.type} | ${e.amount > 0 ? "+" : ""}${fmt(e.amount)} | ep ${e.epoch || "?"}`
      ).join("\n"),
      data,
    };
  },
};

// ── /btcpc chain ──────────────────────────────────────────────────────────────
engines.chain = {
  _help: `
chain status                    — chain height, epoch, active miners, supply
chain blocks [count]            — recent blocks
chain epoch [number]            — epoch detail
chain supply                    — token supply breakdown
chain peers                     — connected peers
chain fork status               — fork resolver status`,

  status: async (args, ctx) => {
    const data = await api(ctx, "/status");
    return {
      text: [
        `**BTCPC chain**`,
        `Height: ${data.chain_height || "?"}  |  Epoch: ${data.epoch || "?"}`,
        `Peers: ${data.peers || 0}  |  Miners: ${data.active_miners || 0}`,
        `Block reward: ${fmt(data.block_reward || 0)} BTCPC`,
      ].join("\n"),
      data,
    };
  },

  blocks: async (args, ctx) => {
    const count = parseInt(args[0]) || 5;
    const data = await api(ctx, "/blocks?limit=" + count);
    const blocks = data.blocks || [];
    if (!blocks.length) return { text: "No blocks", data };
    return {
      text: "**Recent blocks**\n" + blocks.map(b =>
        `  #${b.number} | miner=${b.miner || "?"} | reward=${fmt(b.block_reward || 0, 2)} | entries=${b.entry_count || 0}`
      ).join("\n"),
      data,
    };
  },

  epoch: async (args, ctx) => {
    const num = parseInt(args[0]) || ctx.epoch;
    const data = await api(ctx, "/epoch/" + num);
    return {
      text: [
        `**Epoch ${num}**`,
        `Block reward: ${fmt(data.block_reward || 0)} BTCPC`,
        `Total work: ${fmt(data.total_work || 0, 2)}`,
        `Status: ${data.status || "?"}`,
      ].join("\n"),
      data,
    };
  },

  supply: async (args, ctx) => {
    const data = await api(ctx, "/supply");
    return {
      text: [
        `**BTCPC supply**`,
        `Total: ${fmt(data.total_supply || 0, 0)} BTCPC`,
        `Circulating: ${fmt(data.circulating || 0, 0)} BTCPC`,
        `Recycled: ${fmt(data.recycled || 0, 4)} BTCPC`,
        `Staked: ${fmt(data.staked || 0, 4)} BTCPC`,
      ].join("\n"),
      data,
    };
  },

  peers: async (args, ctx) => {
    const data = await api(ctx, "/network");
    const peers = data.peers || [];
    if (!peers.length) return { text: "No connected peers", data };
    return {
      text: "**Peers**\n" + peers.slice(0, 10).map(p =>
        `  ${p.peerId ? p.peerId.slice(0, 12) + "..." : "?"} | ${p.address || "?"}`
      ).join("\n"),
      data,
    };
  },

  fork: async (args, ctx) => {
    const data = await api(ctx, "/api/fork/status");
    return {
      text: [
        `**Fork resolver**`,
        `Healing: ${data.heal_in_progress ? "YES" : "no"}`,
        `Last fork epoch: ${data.last_fork_epoch || "none"}`,
        `Rollbacks: ${data.rollback_count || 0}`,
      ].join("\n"),
      data,
    };
  },
};

// ── /btcpc node ───────────────────────────────────────────────────────────────
engines.node = {
  _help: `
node register                   — register this node on-chain
node status [account]           — node registration and stake info
node heartbeat                  — send a manual storage heartbeat
node capabilities               — show declared capabilities`,

  register: async (args, ctx) => {
    const data = await api(ctx, "/api/node/register", { method: "POST", data: { account: ctx.account } });
    return { text: `Node **${ctx.account}** registered on-chain`, data };
  },

  status: async (args, ctx) => {
    const account = args[0] || ctx.account;
    const data = await api(ctx, "/api/node/" + account);
    return {
      text: [
        `**Node ${account}**`,
        `Stake: ${fmt(data.stake || 0)} BTCPC`,
        `Role: ${data.role || "?"}`,
        `Active: ${data.active ? "yes" : "no"}`,
      ].join("\n"),
      data,
    };
  },

  heartbeat: async (args, ctx) => {
    const data = await api(ctx, "/api/node/heartbeat", { method: "POST", data: { account: ctx.account } });
    return { text: `Heartbeat sent for **${ctx.account}**`, data };
  },

  capabilities: async (args, ctx) => {
    const account = args[0] || ctx.account;
    const data = await api(ctx, "/api/tools/nodes?account=" + account);
    const node = (data.nodes || [])[0];
    if (!node) return { text: `No tool capabilities registered for ${account}`, data };
    return {
      text: `**Capabilities for ${account}**\n` + node.tools.map(t =>
        `  ${t.name} | kind=${t.kind} | ×${t.multiplier || 1}`
      ).join("\n"),
      data,
    };
  },
};

// ── Top-level shortcuts ───────────────────────────────────────────────────────
// /btcpc balance → /btcpc wallet balance
// /btcpc send → /btcpc wallet send
// /btcpc blocks → /btcpc chain blocks
// /btcpc status → /btcpc chain status
// /btcpc model → /btcpc mine model

const SHORTCUTS = {
  balance: (args, ctx) => engines.wallet.balance(args, ctx),
  send: (args, ctx) => engines.wallet.send(args, ctx),
  blocks: (args, ctx) => engines.chain.blocks(args, ctx),
  status: (args, ctx) => engines.chain.status(args, ctx),
  model: (args, ctx) => engines.mine.model(args, ctx),
  supply: (args, ctx) => engines.chain.supply(args, ctx),
  peers: (args, ctx) => engines.chain.peers(args, ctx),
  epoch: (args, ctx) => engines.chain.epoch(args, ctx),
};

// ── Router ────────────────────────────────────────────────────────────────────

/**
 * Route a /btcpc command.
 *
 * @param {string} input   — full command string, e.g. "/btcpc mine model qwen2.5:14b"
 * @param {object} context — { account, epoch, apiBase, sendLedger? }
 * @returns {Promise<{ text: string, data?: object }>}
 */
async function route(input, context) {
  const ctx = context || {};

  // Strip leading /btcpc or btcpc
  const raw = input.replace(/^\/?btcpc\s*/i, "").trim();
  if (!raw) return { text: _globalHelp() };

  const parts = raw.split(/\s+/);
  const engineName = parts[0].toLowerCase();
  const action = parts[1] ? parts[1].toLowerCase() : null;
  const args = parts.slice(2);

  // Top-level shortcut?
  if (SHORTCUTS[engineName]) {
    return await SHORTCUTS[engineName](parts.slice(1), ctx);
  }

  const engine = engines[engineName];
  if (!engine) {
    return { text: `Unknown engine: **${engineName}**\n\n` + _globalHelp() };
  }

  // Help
  if (!action || action === "help") {
    return { text: (engine._help || "No help available").trim() };
  }

  // infer has no sub-actions — all args are the prompt
  if (engineName === "infer") {
    return await engines.infer._default(parts.slice(1), ctx);
  }

  const handler = engine[action];
  if (!handler || action.startsWith("_")) {
    return { text: `Unknown action: **${action}**\n\n${(engine._help || "").trim()}` };
  }

  return await handler(args, ctx);
}

function _globalHelp() {
  return [
    "**BTCPC command engines**",
    "  /btcpc mine     — inference / mining",
    "  /btcpc verify   — proof verification",
    "  /btcpc clock    — clock consensus",
    "  /btcpc store    — BTCPC-FS storage",
    "  /btcpc sensor   — sensor data market",
    "  /btcpc tool     — MCP / CLI tools",
    "  /btcpc infer    — tool-augmented inference",
    "  /btcpc wallet   — balance, send, stake",
    "  /btcpc chain    — chain status, blocks, supply",
    "  /btcpc node     — node registration",
    "",
    "Run /btcpc <engine> help for details.",
    "",
    "Shortcuts: /btcpc status · balance · send · blocks · supply · peers · epoch · model",
  ].join("\n");
}

module.exports = { route, engines, SHORTCUTS };
