"use strict";

/**
 * BTCPC Shard Registry
 * Shin Devlin
 *
 * Mode B inference sharding: each shard node holds a contiguous range of
 * model layers and exposes an RPC WebSocket endpoint for passing activations.
 * This registry tracks which shards are registered and assembles ShardGroups
 * when all layers of a model are covered.
 *
 * The actual tensor serialization / shard-to-shard RPC calls are stubbed
 * with TODO comments — only the registry and grouping logic is fully
 * implemented here.
 *
 * All state is in-memory (no Mongo).
 */

const crypto = require("crypto");

// ── In-memory stores ──

// shards: Map<nodeId+model → ShardRegistration>
const shards = new Map();

// groups: Map<groupId → ShardGroup>
const groups = new Map();

// ── Helpers ──

function _shardKey(nodeId, modelName) {
  return `${nodeId}::${modelName}`;
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/**
 * Register or update a shard.
 * After storing, tries to form a group for this model.
 *
 * @param {object} reg - ShardRegistration:
 *   {
 *     node_id:      string,
 *     model_name:   string,
 *     layer_start:  number,
 *     layer_end:    number,
 *     total_layers: number,
 *     param_count:  number,      // params in this shard
 *     engine:       'llamacpp-rpc' | 'exo' | 'ollama',
 *     rpc_endpoint: string,      // ws://host:port
 *   }
 * @returns {{ shard: ShardRegistration, groupResult: { formed, group } }}
 */
function registerShard(reg) {
  if (!reg || !reg.node_id || !reg.model_name) {
    throw new Error("registerShard: node_id and model_name are required");
  }

  const key = _shardKey(reg.node_id, reg.model_name);
  const now = Date.now();

  const entry = {
    node_id: reg.node_id,
    model_name: reg.model_name,
    layer_start: reg.layer_start || 0,
    layer_end: reg.layer_end || 0,
    total_layers: reg.total_layers || 1,
    param_count: reg.param_count || 0,
    engine: reg.engine || "ollama",
    rpc_endpoint: reg.rpc_endpoint || null,
    registered_at: shards.has(key) ? shards.get(key).registered_at : now,
    last_seen: now,
  };

  shards.set(key, entry);

  const groupResult = _tryFormGroup(reg.model_name);
  return { shard: entry, groupResult };
}

/**
 * Attempt to form a ShardGroup for a model if all layers 0..total_layers-1
 * are covered by registered shards (with no gaps).
 *
 * @param {string} modelName
 * @returns {{ formed: boolean, group: ShardGroup|null }}
 */
function _tryFormGroup(modelName) {
  // Collect all shards for this model
  const modelShards = [];
  for (const [, shard] of shards) {
    if (shard.model_name === modelName) {
      modelShards.push(shard);
    }
  }

  if (modelShards.length === 0) return { formed: false, group: null };

  // All shards must agree on total_layers
  const totalLayers = modelShards[0].total_layers;
  const allAgree = modelShards.every((s) => s.total_layers === totalLayers);
  if (!allAgree) return { formed: false, group: null };

  // Sort by layer_start
  modelShards.sort((a, b) => a.layer_start - b.layer_start);

  // Verify coverage: layers 0..totalLayers-1 must be covered contiguously
  let cursor = 0;
  for (const shard of modelShards) {
    if (shard.layer_start !== cursor) {
      // Gap detected
      return { formed: false, group: null };
    }
    cursor = shard.layer_end + 1;
  }

  // cursor must exactly reach totalLayers
  if (cursor !== totalLayers) {
    return { formed: false, group: null };
  }

  // All layers covered — form or refresh the group
  const totalParams = modelShards.reduce((sum, s) => sum + (s.param_count || 0), 0);
  const groupId = crypto
    .createHash("sha256")
    .update(modelName + modelShards.map((s) => s.node_id).join("|"))
    .digest("hex")
    .slice(0, 16);

  const group = {
    group_id: groupId,
    model_name: modelName,
    total_params: totalParams,
    shards: modelShards,
    coordinator: modelShards[0].node_id, // lowest-layer shard coordinates
    formed_at: Date.now(),
    status: "active",
  };

  groups.set(groupId, group);
  return { formed: true, group };
}

/**
 * Get the active ShardGroup for a model, or null.
 * @param {string} modelName
 * @returns {ShardGroup|null}
 */
function getGroupForModel(modelName) {
  for (const [, group] of groups) {
    if (group.model_name === modelName && group.status === "active") {
      return group;
    }
  }
  return null;
}

/**
 * Get all active ShardGroups.
 * @returns {ShardGroup[]}
 */
function getAllGroups() {
  return Array.from(groups.values()).filter((g) => g.status === "active");
}

/**
 * Remove shards that haven't sent a heartbeat in `ms` milliseconds.
 * Re-evaluates affected models' group status after pruning.
 * @param {number} ms
 * @returns {number} count of pruned shards
 */
function pruneStaleShardsOlderThan(ms) {
  const cutoff = Date.now() - ms;
  let pruned = 0;
  const affectedModels = new Set();

  for (const [key, shard] of shards) {
    if (shard.last_seen < cutoff) {
      affectedModels.add(shard.model_name);
      shards.delete(key);
      pruned++;
    }
  }

  // Invalidate groups whose model lost shards
  for (const model of affectedModels) {
    for (const [gid, group] of groups) {
      if (group.model_name === model) {
        group.status = "degraded";
      }
    }
    // Try to re-form if remaining shards still cover all layers
    _tryFormGroup(model);
  }

  return pruned;
}

/**
 * Compute the proportional work value for a single shard node.
 *
 * Formula: tokens × shardParamCount
 * (This simplifies to tokens × totalParams × (shardParams / totalParams),
 * i.e. the shard's proportional share of the full model's output.)
 *
 * @param {string} groupId
 * @param {string} nodeId
 * @param {number} tokensGenerated
 * @returns {number} work value (0 if node not in group)
 */
function computeShardWorkValue(groupId, nodeId, tokensGenerated) {
  const group = groups.get(groupId);
  if (!group) return 0;

  const shard = group.shards.find((s) => s.node_id === nodeId);
  if (!shard) return 0;

  // Proportional: tokens × shardParamCount
  return tokensGenerated * shard.param_count;
}

// ---------------------------------------------------------------------------
// Shard-to-shard activation passing (stub)
// ---------------------------------------------------------------------------

/**
 * Serialize activations for passing between shard nodes.
 * TODO: Replace JSON round-trip with a proper float16/bfloat16 tensor
 *       encoding once we integrate llama.cpp RPC or exo transport.
 *
 * @param {any} activations   — activation tensor (array of floats)
 * @returns {Buffer}
 */
function serializeActivations(activations) {
  // TODO: real tensor serialization (float16 / bfloat16)
  return Buffer.from(JSON.stringify(activations));
}

/**
 * Deserialize activations received from the previous shard node.
 * TODO: Mirror serializeActivations once real encoding is in place.
 *
 * @param {Buffer} buf
 * @returns {any}
 */
function deserializeActivations(buf) {
  // TODO: mirror real encoding
  return JSON.parse(buf.toString());
}

/**
 * Build a SHARD_ACTIVATE P2P message payload.
 * Sent from one shard node to the next in the pipeline.
 *
 * @param {string} groupId
 * @param {string} requestId
 * @param {number} layerIndex  — output layer of the sending shard
 * @param {any}    activations — raw activation data
 * @returns {object}
 */
function buildActivatePayload(groupId, requestId, layerIndex, activations) {
  return {
    group_id: groupId,
    request_id: requestId,
    layer_index: layerIndex,
    // TODO: replace with real tensor encoding
    activations_b64: serializeActivations(activations).toString("base64"),
  };
}

module.exports = {
  registerShard,
  _tryFormGroup,
  getGroupForModel,
  getAllGroups,
  pruneStaleShardsOlderThan,
  computeShardWorkValue,
  serializeActivations,
  deserializeActivations,
  buildActivatePayload,
  // Exposed for testing
  _shards: shards,
  _groups: groups,
};
