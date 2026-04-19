"use strict";

/**
 * Tests for src/inference/shardRegistry.js
 */

const assert = require("assert");
const {
  registerShard,
  _tryFormGroup,
  getGroupForModel,
  getAllGroups,
  pruneStaleShardsOlderThan,
  computeShardWorkValue,
  _shards,
  _groups,
} = require("../src/inference/shardRegistry");

// Helper: clear in-memory state between tests
function clearState() {
  _shards.clear();
  _groups.clear();
}

// ---------------------------------------------------------------------------
// Test 1: registerShard with complete coverage → group forms
// ---------------------------------------------------------------------------
{
  clearState();

  // Model with 32 total layers, split across 2 shards
  const result1 = registerShard({
    node_id: "node-alpha",
    model_name: "llama3:8b",
    layer_start: 0,
    layer_end: 15,
    total_layers: 32,
    param_count: 4e9,
    engine: "llamacpp-rpc",
    rpc_endpoint: "ws://10.0.0.1:8080",
  });

  assert.ok(result1.shard, "shard entry created");
  assert.strictEqual(result1.groupResult.formed, false, "no group yet — only half the layers covered");
  assert.strictEqual(getGroupForModel("llama3:8b"), null, "no active group yet");

  const result2 = registerShard({
    node_id: "node-beta",
    model_name: "llama3:8b",
    layer_start: 16,
    layer_end: 31,
    total_layers: 32,
    param_count: 4e9,
    engine: "llamacpp-rpc",
    rpc_endpoint: "ws://10.0.0.2:8080",
  });

  assert.strictEqual(result2.groupResult.formed, true, "group should form when all layers covered");
  const group = result2.groupResult.group;
  assert.ok(group, "group object returned");
  assert.strictEqual(group.model_name, "llama3:8b");
  assert.strictEqual(group.shards.length, 2, "group has 2 shards");
  assert.strictEqual(group.coordinator, "node-alpha", "coordinator is the first shard (lowest layers)");
  assert.strictEqual(group.total_params, 8e9, "total params = sum of shard params");
  assert.strictEqual(group.status, "active");

  const active = getGroupForModel("llama3:8b");
  assert.ok(active, "getGroupForModel returns the active group");
  assert.strictEqual(active.group_id, group.group_id);

  console.log("PASS: complete coverage → group forms");
}

// ---------------------------------------------------------------------------
// Test 2: registerShard with gap → no group
// ---------------------------------------------------------------------------
{
  clearState();

  // Shard covers 0..9 (10 layers)
  registerShard({
    node_id: "node-x",
    model_name: "mistral:7b",
    layer_start: 0,
    layer_end: 9,
    total_layers: 32,
    param_count: 3e9,
    engine: "exo",
    rpc_endpoint: "ws://10.0.0.3:9000",
  });

  // Shard covers 15..31 (gap at 10..14)
  const result = registerShard({
    node_id: "node-y",
    model_name: "mistral:7b",
    layer_start: 15,
    layer_end: 31,
    total_layers: 32,
    param_count: 4e9,
    engine: "exo",
    rpc_endpoint: "ws://10.0.0.4:9000",
  });

  assert.strictEqual(result.groupResult.formed, false, "gap detected — no group formed");
  assert.strictEqual(getGroupForModel("mistral:7b"), null, "no active group when gap exists");

  console.log("PASS: gap in layers → no group");
}

// ---------------------------------------------------------------------------
// Test 3: pruneStaleShardsOlderThan removes old entries
// ---------------------------------------------------------------------------
{
  clearState();

  // Register two shards
  registerShard({
    node_id: "node-old",
    model_name: "gemma:2b",
    layer_start: 0,
    layer_end: 17,
    total_layers: 18,
    param_count: 1e9,
    engine: "ollama",
    rpc_endpoint: null,
  });

  registerShard({
    node_id: "node-new",
    model_name: "gemma:2b",
    layer_start: 0,
    layer_end: 17,
    total_layers: 18,
    param_count: 1e9,
    engine: "ollama",
    rpc_endpoint: null,
  });

  // Manually backdate node-old's last_seen
  const oldKey = "node-old::gemma:2b";
  const oldShard = _shards.get(oldKey);
  assert.ok(oldShard, "old shard exists before pruning");
  oldShard.last_seen = Date.now() - 200000; // 200 seconds ago

  const pruned = pruneStaleShardsOlderThan(60000); // 60 second threshold
  assert.ok(pruned >= 1, "at least one shard pruned");
  assert.strictEqual(_shards.has(oldKey), false, "stale shard removed");
  assert.ok(_shards.has("node-new::gemma:2b"), "fresh shard retained");

  console.log("PASS: pruneStaleShardsOlderThan removes stale shards");
}

// ---------------------------------------------------------------------------
// Test 4: computeShardWorkValue proportional to param count
// ---------------------------------------------------------------------------
{
  clearState();

  // 3-shard model: 32 layers, different param counts per shard
  registerShard({
    node_id: "shard-0",
    model_name: "deepseek:33b",
    layer_start: 0,
    layer_end: 9,
    total_layers: 30,
    param_count: 10e9,
    engine: "llamacpp-rpc",
    rpc_endpoint: "ws://a:8080",
  });
  registerShard({
    node_id: "shard-1",
    model_name: "deepseek:33b",
    layer_start: 10,
    layer_end: 19,
    total_layers: 30,
    param_count: 12e9,
    engine: "llamacpp-rpc",
    rpc_endpoint: "ws://b:8080",
  });
  const lastResult = registerShard({
    node_id: "shard-2",
    model_name: "deepseek:33b",
    layer_start: 20,
    layer_end: 29,
    total_layers: 30,
    param_count: 11e9,
    engine: "llamacpp-rpc",
    rpc_endpoint: "ws://c:8080",
  });

  assert.strictEqual(lastResult.groupResult.formed, true, "3-shard group formed");
  const group = lastResult.groupResult.group;
  const groupId = group.group_id;

  const tokens = 500;

  // shard-0: tokens × 10e9
  const work0 = computeShardWorkValue(groupId, "shard-0", tokens);
  assert.strictEqual(work0, tokens * 10e9, "shard-0 work value correct");

  // shard-1: tokens × 12e9
  const work1 = computeShardWorkValue(groupId, "shard-1", tokens);
  assert.strictEqual(work1, tokens * 12e9, "shard-1 work value correct");

  // shard-2: tokens × 11e9
  const work2 = computeShardWorkValue(groupId, "shard-2", tokens);
  assert.strictEqual(work2, tokens * 11e9, "shard-2 work value correct");

  // Proportional: bigger param shard gets more reward
  assert.ok(work1 > work0, "higher-param shard earns more");
  assert.ok(work1 > work2, "highest-param shard earns most");

  // Node not in group
  const workZ = computeShardWorkValue(groupId, "shard-unknown", tokens);
  assert.strictEqual(workZ, 0, "unknown node gets 0 work value");

  // Non-existent group
  const workNone = computeShardWorkValue("bad-group-id", "shard-0", tokens);
  assert.strictEqual(workNone, 0, "non-existent group returns 0");

  console.log("PASS: computeShardWorkValue proportional to param count");
}

// ---------------------------------------------------------------------------
// Test 5: getAllGroups returns only active groups
// ---------------------------------------------------------------------------
{
  clearState();

  // Register a complete 1-shard model (1 layer total)
  registerShard({
    node_id: "solo-node",
    model_name: "phi3:mini",
    layer_start: 0,
    layer_end: 0,
    total_layers: 1,
    param_count: 3.8e9,
    engine: "ollama",
    rpc_endpoint: null,
  });

  const groups = getAllGroups();
  assert.ok(groups.length >= 1, "at least one active group");
  const phi = groups.find((g) => g.model_name === "phi3:mini");
  assert.ok(phi, "phi3:mini group in getAllGroups");
  assert.strictEqual(phi.status, "active");

  console.log("PASS: getAllGroups returns active groups");
}

// ---------------------------------------------------------------------------
// Test 6: re-registering an existing shard updates last_seen
// ---------------------------------------------------------------------------
{
  clearState();

  registerShard({
    node_id: "heartbeat-node",
    model_name: "tinyllama:1b",
    layer_start: 0,
    layer_end: 21,
    total_layers: 22,
    param_count: 1.1e9,
    engine: "ollama",
    rpc_endpoint: "ws://x:8080",
  });

  const key = "heartbeat-node::tinyllama:1b";
  const first = _shards.get(key);
  const firstSeen = first.last_seen;

  // Small delay not needed — test just checks idempotency
  registerShard({
    node_id: "heartbeat-node",
    model_name: "tinyllama:1b",
    layer_start: 0,
    layer_end: 21,
    total_layers: 22,
    param_count: 1.1e9,
    engine: "ollama",
    rpc_endpoint: "ws://x:8080",
  });

  const second = _shards.get(key);
  assert.ok(second.last_seen >= firstSeen, "last_seen updated on re-registration");
  assert.strictEqual(second.registered_at, first.registered_at, "registered_at preserved on re-registration");

  console.log("PASS: re-registration updates last_seen");
}

console.log("\nAll shardRegistry tests passed.");
