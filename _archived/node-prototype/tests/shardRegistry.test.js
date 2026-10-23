"use strict";

const {
  registerShard,
  getGroupForModel,
  getAllGroups,
  pruneStaleShardsOlderThan,
  computeShardWorkValue,
  _shards,
  _groups,
} = require("../src/inference/shardRegistry");

beforeEach(() => { _shards.clear(); _groups.clear(); });

test("complete layer coverage → group forms", () => {
  const r1 = registerShard({ node_id: "alpha", model_name: "llama3:8b", layer_start: 0, layer_end: 15, total_layers: 32, param_count: 4e9, engine: "llamacpp-rpc", rpc_endpoint: "ws://a:8080" });
  expect(r1.groupResult.formed).toBe(false);
  expect(getGroupForModel("llama3:8b")).toBeNull();

  const r2 = registerShard({ node_id: "beta", model_name: "llama3:8b", layer_start: 16, layer_end: 31, total_layers: 32, param_count: 4e9, engine: "llamacpp-rpc", rpc_endpoint: "ws://b:8080" });
  expect(r2.groupResult.formed).toBe(true);

  const group = r2.groupResult.group;
  expect(group.model_name).toBe("llama3:8b");
  expect(group.shards.length).toBe(2);
  expect(group.coordinator).toBe("alpha");
  expect(group.total_params).toBe(8e9);
  expect(group.status).toBe("active");
  expect(getGroupForModel("llama3:8b").group_id).toBe(group.group_id);
});

test("gap in layers → no group forms", () => {
  registerShard({ node_id: "x", model_name: "mistral:7b", layer_start: 0, layer_end: 9, total_layers: 32, param_count: 3e9, engine: "exo", rpc_endpoint: "ws://x:9000" });
  const r = registerShard({ node_id: "y", model_name: "mistral:7b", layer_start: 15, layer_end: 31, total_layers: 32, param_count: 4e9, engine: "exo", rpc_endpoint: "ws://y:9000" });
  expect(r.groupResult.formed).toBe(false);
  expect(getGroupForModel("mistral:7b")).toBeNull();
});

test("pruneStaleShardsOlderThan removes old shards", () => {
  registerShard({ node_id: "old", model_name: "gemma:2b", layer_start: 0, layer_end: 17, total_layers: 18, param_count: 1e9, engine: "ollama", rpc_endpoint: null });
  registerShard({ node_id: "fresh", model_name: "gemma:2b", layer_start: 0, layer_end: 17, total_layers: 18, param_count: 1e9, engine: "ollama", rpc_endpoint: null });

  const oldKey = "old::gemma:2b";
  _shards.get(oldKey).last_seen = Date.now() - 200000;

  expect(pruneStaleShardsOlderThan(60000)).toBeGreaterThanOrEqual(1);
  expect(_shards.has(oldKey)).toBe(false);
  expect(_shards.has("fresh::gemma:2b")).toBe(true);
});

test("computeShardWorkValue proportional to param count", () => {
  registerShard({ node_id: "s0", model_name: "deepseek:33b", layer_start: 0, layer_end: 9, total_layers: 30, param_count: 10e9, engine: "llamacpp-rpc", rpc_endpoint: "ws://a:8080" });
  registerShard({ node_id: "s1", model_name: "deepseek:33b", layer_start: 10, layer_end: 19, total_layers: 30, param_count: 12e9, engine: "llamacpp-rpc", rpc_endpoint: "ws://b:8080" });
  const last = registerShard({ node_id: "s2", model_name: "deepseek:33b", layer_start: 20, layer_end: 29, total_layers: 30, param_count: 11e9, engine: "llamacpp-rpc", rpc_endpoint: "ws://c:8080" });

  expect(last.groupResult.formed).toBe(true);
  const gid = last.groupResult.group.group_id;
  const tokens = 500;

  expect(computeShardWorkValue(gid, "s0", tokens)).toBe(tokens * 10e9);
  expect(computeShardWorkValue(gid, "s1", tokens)).toBe(tokens * 12e9);
  expect(computeShardWorkValue(gid, "s2", tokens)).toBe(tokens * 11e9);
  expect(computeShardWorkValue(gid, "unknown", tokens)).toBe(0);
  expect(computeShardWorkValue("bad-group", "s0", tokens)).toBe(0);
});

test("getAllGroups returns active groups", () => {
  registerShard({ node_id: "solo", model_name: "phi3:mini", layer_start: 0, layer_end: 0, total_layers: 1, param_count: 3.8e9, engine: "ollama", rpc_endpoint: null });
  const groups = getAllGroups();
  const phi = groups.find(g => g.model_name === "phi3:mini");
  expect(phi).toBeTruthy();
  expect(phi.status).toBe("active");
});

test("re-registering a shard updates last_seen but preserves registered_at", () => {
  const reg = { node_id: "hb", model_name: "tinyllama:1b", layer_start: 0, layer_end: 21, total_layers: 22, param_count: 1.1e9, engine: "ollama", rpc_endpoint: "ws://x:8080" };
  registerShard(reg);
  const key = "hb::tinyllama:1b";
  const first = { ...(_shards.get(key)) };

  registerShard(reg);
  const second = _shards.get(key);
  expect(second.last_seen).toBeGreaterThanOrEqual(first.last_seen);
  expect(second.registered_at).toBe(first.registered_at);
});
