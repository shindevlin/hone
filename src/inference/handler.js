"use strict";

/**
 * BTCPC Inference Handler — runs on miner nodes
 * Shin Devlin
 *
 * Listens for INFERENCE_REQUEST messages on the P2P network,
 * auto-claims jobs, runs inference via Ollama, commits/reveals results.
 *
 * This is the miner's side of the inference pipeline.
 */

const crypto = require("crypto");
const axios = require("axios");
const p2p = require("../p2p/network");
const { createMessage } = require("../p2p/protocol");
const { GENESIS_MINER } = require("../mining/genesisBlock");
const MINER_NAME = process.env.BTCPC_MINER || GENESIS_MINER;
const { getModelWeight } = require("../mining/workGenerator");
const stateStore = require("../chain/stateStore");
const nodeRegistry = require("../chain/nodeRegistry");
const inferenceCrypto = require("./crypto");

const OLLAMA_URL = process.env.OLLAMA_URL || "http://100.122.145.60:11434";
const MAX_CONCURRENT = parseInt(process.env.MAX_CONCURRENT_INFERENCE) || 1;

let activeJobs = 0;
const modelActiveJobs = new Map();
const modelStats = new Map();
let inferenceCount = 0;

// Track request IDs we've already seen to ignore relay echoes
const seenRequests = new Set();
const SEEN_MAX = 1000;
const MODEL_BUSY_THRESHOLD = parseInt(process.env.BTCPC_MODEL_BUSY_THRESHOLD, 10) || 3;

// Per-job state: which jobs have been claimed (by us or anyone), what's in flight on this miner
// claimedJobs: Map<request_id, { miner, claimed_at, nudges_sent }> — every claim we've seen
// jobInFlight: { request_id, started_at, nudges_received } | null — what we're processing
const claimedJobs = new Map();
const claimedJobsMax = 5000;
const jobsClaimedByOthers = new Set(); // legacy alias for back-compat lookups
let jobInFlight = null;
const JOB_MAX_DURATION_MS = parseInt(process.env.BTCPC_JOB_MAX_DURATION_MS, 10) || 10 * 60 * 1000; // 10 min
const CLAIM_DELAY_MS = parseInt(process.env.BTCPC_CLAIM_DELAY_MS, 10) || 800;
const STALE_CLAIM_MS = parseInt(process.env.BTCPC_STALE_CLAIM_MS, 10) || 90 * 1000; // 90s before nudging
const NUDGE_INTERVAL_MS = parseInt(process.env.BTCPC_NUDGE_INTERVAL_MS, 10) || 30 * 1000; // 30s between nudges
const MAX_NUDGES = parseInt(process.env.BTCPC_MAX_NUDGES, 10) || 2; // 2 nudges before reclaim

function rememberClaim(reqId, miner) {
  if (!reqId) return;
  // Preserve first_claimed_at so we can enforce a hard cap regardless of refreshes
  const existing = claimedJobs.get(reqId);
  claimedJobs.set(reqId, {
    miner: miner || 'unknown',
    first_claimed_at: existing?.first_claimed_at || Date.now(),
    claimed_at: Date.now(),
    nudges_sent: 0,
    refresh_count: existing ? (existing.refresh_count || 0) + 1 : 0,
  });
  jobsClaimedByOthers.add(reqId);
  if (claimedJobs.size > claimedJobsMax) {
    const first = claimedJobs.keys().next().value;
    claimedJobs.delete(first);
    jobsClaimedByOthers.delete(first);
  }
}

function forgetClaim(reqId) {
  if (!reqId) return;
  claimedJobs.delete(reqId);
  jobsClaimedByOthers.delete(reqId);
}

function clearJobInFlight(reason) {
  if (jobInFlight) {
    console.log(`[BTCPC Inference] Job ${jobInFlight.request_id?.slice(0, 8)} cleared (${reason}) after ${Date.now() - jobInFlight.started_at}ms`);
    forgetClaim(jobInFlight.request_id);
    jobInFlight = null;
  }
}

// Local watchdog: clear our own jobs that exceed max duration
setInterval(() => {
  if (jobInFlight && Date.now() - jobInFlight.started_at > JOB_MAX_DURATION_MS) {
    console.log(`[BTCPC Inference] WATCHDOG: clearing stuck job ${jobInFlight.request_id?.slice(0, 8)}`);
    clearJobInFlight('watchdog timeout');
  }
}, 30000);

// Network watchdog: nudge stale claims, reclaim if unresponsive.
// Any node can run this — P2P network-wide stale-job recovery.
//
// Two timeouts:
//   STALE_CLAIM_MS — time since last refresh before sending a nudge
//   JOB_MAX_DURATION_MS — total time since first claim, hard cap
//
// The miner can extend the soft timeout by responding to nudges with refresh
// claims, but cannot extend past the hard cap.
setInterval(() => {
  const now = Date.now();
  for (const [reqId, claim] of claimedJobs) {
    // Skip our own in-flight job (we have a local watchdog for that)
    if (jobInFlight && jobInFlight.request_id === reqId) continue;

    const ageSinceRefresh = now - claim.claimed_at;
    const totalAge = now - claim.first_claimed_at;

    // Hard cap: regardless of refreshes, force release after JOB_MAX_DURATION_MS
    if (totalAge >= JOB_MAX_DURATION_MS) {
      const reclaim = createMessage("INFERENCE_RECLAIM", {
        request_id: reqId,
        original_miner: claim.miner,
        reason: 'exceeded hard cap (' + Math.round(JOB_MAX_DURATION_MS / 60000) + ' min)',
        age_ms: totalAge,
      }, p2p.NODE_ID);
      p2p.broadcast(reclaim);
      console.log(`[BTCPC Inference] RECLAIM ${reqId.slice(0, 8)} — ${claim.miner} exceeded hard cap (${Math.round(totalAge/1000)}s, ${claim.refresh_count} refreshes)`);
      forgetClaim(reqId);
      continue;
    }

    // Soft timeout: nudge after STALE_CLAIM_MS since last refresh
    if (ageSinceRefresh < STALE_CLAIM_MS) continue;

    if (claim.nudges_sent >= MAX_NUDGES) {
      const reclaim = createMessage("INFERENCE_RECLAIM", {
        request_id: reqId,
        original_miner: claim.miner,
        reason: 'unresponsive after ' + MAX_NUDGES + ' nudges',
        age_ms: totalAge,
      }, p2p.NODE_ID);
      p2p.broadcast(reclaim);
      console.log(`[BTCPC Inference] RECLAIM ${reqId.slice(0, 8)} — ${claim.miner} silent for ${Math.round(ageSinceRefresh/1000)}s`);
      forgetClaim(reqId);
      continue;
    }

    const lastNudgeAge = ageSinceRefresh - (claim.nudges_sent * NUDGE_INTERVAL_MS);
    if (lastNudgeAge < NUDGE_INTERVAL_MS) continue;

    const nudge = createMessage("INFERENCE_NUDGE", {
      request_id: reqId,
      target_miner: claim.miner,
      age_ms: totalAge,
      since_refresh_ms: ageSinceRefresh,
      nudge_number: claim.nudges_sent + 1,
    }, p2p.NODE_ID);
    p2p.broadcast(nudge);
    claim.nudges_sent++;
    console.log(`[BTCPC Inference] NUDGE ${reqId.slice(0, 8)} → ${claim.miner} (#${claim.nudges_sent}, silent: ${Math.round(ageSinceRefresh/1000)}s)`);
  }
}, 15000);

function getModelStats(model) {
  if (!modelStats.has(model)) {
    modelStats.set(model, {
      requests: 0,
      successes: 0,
      failures: 0,
      avgResponseMs: 0,
      avgTokensPerSec: 0
    });
  }
  return modelStats.get(model);
}

function parseModelParams(model) {
  const match = String(model || "").match(/(\d+(?:\.\d+)?)\s*b/i);
  return match ? Number(match[1]) : 0;
}

function promptComplexity(prompt) {
  const text = String(prompt || "");
  if (text.length > 4000 || /analy[sz]e|architecture|proof|derive|implement|refactor|security/i.test(text)) return "high";
  if (text.length > 1200 || /code|debug|explain|compare|summari[sz]e/i.test(text)) return "medium";
  return "low";
}

function modelComplexityFloor(model) {
  const params = parseModelParams(model);
  if (params >= 20) return "high";
  if (params >= 7) return "medium";
  return "low";
}

function complexityRank(value) {
  return { low: 0, medium: 1, high: 2 }[value] || 0;
}

function scoreModel(model, desiredComplexity) {
  const stats = getModelStats(model);
  const successRate = stats.requests ? stats.successes / stats.requests : 1;
  const speed = stats.avgTokensPerSec || 0;
  const busyPenalty = (modelActiveJobs.get(model) || 0) * 100;
  const complexityPenalty = Math.max(0, complexityRank(modelComplexityFloor(model)) - complexityRank(desiredComplexity)) * 2;
  return (parseModelParams(model) * 1000) + (successRate * 100) + speed - busyPenalty - complexityPenalty;
}

function chooseModel(requestedModel, prompt, availableModels) {
  const available = (availableModels || []).filter(Boolean);
  if (!available.length) return requestedModel || "qwen3.5:27b";

  if (requestedModel && requestedModel !== "auto") {
    const exact = available.includes(requestedModel) ? requestedModel : null;
    const withLatest = !requestedModel.includes(":") && available.includes(requestedModel + ":latest") ? requestedModel + ":latest" : null;
    return exact || withLatest || requestedModel;
  }

  const desired = promptComplexity(prompt);
  const eligible = available.filter(model => complexityRank(modelComplexityFloor(model)) >= complexityRank(desired));
  const candidates = eligible.length ? eligible : available;
  const sorted = candidates.slice().sort((a, b) => scoreModel(b, desired) - scoreModel(a, desired));
  const open = sorted.find(model => (modelActiveJobs.get(model) || 0) <= MODEL_BUSY_THRESHOLD);
  return open || sorted[0];
}

function recordModelResult(model, elapsedMs, tokens, success) {
  const stats = getModelStats(model);
  stats.requests++;
  if (success) stats.successes++;
  else stats.failures++;

  if (success) {
    const tokensPerSec = elapsedMs > 0 ? tokens / (elapsedMs / 1000) : 0;
    stats.avgResponseMs = stats.avgResponseMs
      ? (stats.avgResponseMs * 0.85) + (elapsedMs * 0.15)
      : elapsedMs;
    stats.avgTokensPerSec = stats.avgTokensPerSec
      ? (stats.avgTokensPerSec * 0.85) + (tokensPerSec * 0.15)
      : tokensPerSec;
  }

  inferenceCount++;
  if (inferenceCount % 100 === 0) {
    const summary = Array.from(modelStats.entries()).map(([name, s]) => {
      const successRate = s.requests ? ((s.successes / s.requests) * 100).toFixed(1) : "0.0";
      return `${name}: ${s.requests} req, ${successRate}% ok, ${Math.round(s.avgResponseMs)}ms avg, ${s.avgTokensPerSec.toFixed(2)} tok/s`;
    }).join(" | ");
    console.log(`[BTCPC Inference] Model stats after ${inferenceCount} inferences: ${summary}`);
  }
}

/**
 * Start listening for inference requests on the P2P network.
 * Call this after P2P is connected.
 */
function startInferenceHandler() {
  p2p.onMessage(async (msg, peer) => {
    if (!msg || !msg.type) return;

    switch (msg.type) {
      case "INFERENCE_REQUEST":
        await handleInferenceRequest(msg);
        break;
      case "INFERENCE_CLAIM":
        handleInferenceClaim(msg);
        break;
      case "INFERENCE_NUDGE":
        handleInferenceNudge(msg);
        break;
      case "INFERENCE_RECLAIM":
        handleInferenceReclaim(msg);
        break;
      case "INFERENCE_ASSIGN":
        await handleAssignment(msg);
        break;
      case "INFERENCE_PAYLOAD":
        await handlePayload(msg);
        break;
      case "INFERENCE_RESULT":
      case "INFERENCE_REVEAL":
      case "INFERENCE_COMMIT":
        // A job has been completed — if it was OUR in-flight job, clear the slot
        handleInferenceCompletion(msg);
        break;
      case "MODEL_DEMAND":
        handleModelDemand(msg);
        break;
    }
  });

  console.log("[BTCPC Inference] Handler active, listening for requests");
}

/**
 * Record that some miner (possibly us) claimed a job. Other miners back off.
 * Tracks claim time so the network watchdog can nudge stale claims.
 */
function handleInferenceClaim(msg) {
  const data = msg.data || {};
  const reqId = data.request_id;
  const claimedBy = data.node_name;
  if (!reqId) return;

  rememberClaim(reqId, claimedBy);

  if (claimedBy && claimedBy !== MINER_NAME) {
    console.log(`[BTCPC Inference] Job ${reqId.slice(0, 8)} claimed by ${claimedBy} — backing off`);
  }
}

/**
 * Another node nudged us about a job we claimed. If it's ours, send a heartbeat
 * (renew the claim by re-broadcasting). If it's not ours, ignore.
 */
function handleInferenceNudge(msg) {
  const data = msg.data || {};
  const reqId = data.request_id;
  const target = data.target_miner;
  if (!reqId) return;

  if (target === MINER_NAME && jobInFlight && jobInFlight.request_id === reqId) {
    // We're still working on it — refresh our claim by re-broadcasting
    const elapsed = Date.now() - jobInFlight.started_at;
    console.log(`[BTCPC Inference] Got nudge for ${reqId.slice(0, 8)} — still working (${Math.round(elapsed/1000)}s)`);
    const refresh = createMessage("INFERENCE_CLAIM", {
      request_id: reqId,
      node_name: MINER_NAME,
      node_id: p2p.NODE_ID,
      refresh: true,
      elapsed_ms: elapsed,
      model: jobInFlight.model,
    }, p2p.NODE_ID);
    p2p.broadcast(refresh);
  }
}

/**
 * A claim has been released by the network — the original miner was unresponsive.
 * Remove from claimed set so other miners can claim it. If we were stuck on it,
 * give it up.
 */
function handleInferenceReclaim(msg) {
  const data = msg.data || {};
  const reqId = data.request_id;
  if (!reqId) return;

  console.log(`[BTCPC Inference] RECLAIM received for ${reqId.slice(0, 8)} — ${data.original_miner} unresponsive`);
  forgetClaim(reqId);
  // Also drop from seenRequests so we can re-process the request if it comes around again
  seenRequests.delete(reqId);

  if (jobInFlight && jobInFlight.request_id === reqId) {
    clearJobInFlight('reclaimed by network');
  }
}

/**
 * When a job completes (we hear the result), if it was our in-flight job,
 * clear the slot so we can claim the next one.
 */
function handleInferenceCompletion(msg) {
  const data = msg.data || {};
  const reqId = data.request_id;
  if (!reqId) return;
  // Always drop from claimed map — job is done, no longer occupying any miner
  forgetClaim(reqId);
  if (jobInFlight && jobInFlight.request_id === reqId) {
    clearJobInFlight('completion broadcast');
  }
}

/**
 * Handle incoming inference request.
 *
 * Flow:
 * 1. Dedupe relay echoes
 * 2. If already in-flight on this miner → back off (one job at a time)
 * 3. If already claimed by another miner → back off
 * 4. Verify we have the model + Ollama is reachable
 * 5. Wait CLAIM_DELAY_MS (so other miners get a chance to claim too)
 * 6. Re-check after delay — bail if anyone else claimed
 * 7. Mark in-flight, broadcast INFERENCE_CLAIM
 *
 * The miner stays locked to this job until INFERENCE_RESULT clears it
 * (or watchdog timeout).
 */
async function handleInferenceRequest(msg) {
  const data = msg.data || msg;
  const reqId = data.request_id;

  // Deduplicate relay echoes
  if (!reqId || seenRequests.has(reqId)) return;
  seenRequests.add(reqId);
  if (seenRequests.size > SEEN_MAX) {
    const first = seenRequests.values().next().value;
    seenRequests.delete(first);
  }

  // Already claimed by someone (us or another miner)?
  if (jobsClaimedByOthers.has(reqId)) return;

  // We're already processing a job
  if (jobInFlight) {
    console.log(`[BTCPC Inference] Skipping ${reqId.slice(0, 8)} — busy with ${jobInFlight.request_id.slice(0, 8)}`);
    return;
  }

  // Check if we have the exact requested model
  const requestedModel = data.model || "qwen3.5:27b";
  let model = requestedModel;
  try {
    const modelsResp = await axios.get(`${OLLAMA_URL}/api/tags`, { timeout: 5000 });
    const available = (modelsResp.data.models || []).map(m => m.name);
    model = chooseModel(requestedModel, data.prompt || data.messages?.map(m => m.content).join("\n") || "", available);
    if (!available.includes(model)) {
      const modelBase = model.includes(':') ? model : model + ':latest';
      if (!available.includes(modelBase)) {
        return; // silently skip — don't have the model
      }
    }
  } catch (_) {
    return; // Ollama unreachable, skip silently
  }

  // Wait a short delay before claiming so other miners on the network have
  // a chance to claim too (random jitter to spread load fairly)
  const jitter = Math.random() * CLAIM_DELAY_MS;
  await new Promise((r) => setTimeout(r, jitter));

  // Re-check after delay — has anyone else claimed it?
  if (jobsClaimedByOthers.has(reqId)) {
    return;
  }

  // Re-check that we're still not busy
  if (jobInFlight) {
    return;
  }

  // Get our node info from nodeRegistry (no Mongo)
  const nodeEntry = nodeRegistry.getNode(MINER_NAME);

  // Mark as in-flight BEFORE broadcasting so race conditions don't double-claim
  jobInFlight = {
    request_id: reqId,
    started_at: Date.now(),
    model,
  };

  const claim = createMessage("INFERENCE_CLAIM", {
    request_id: reqId,
    node_id: p2p.NODE_ID,
    sik_hash: "none",
    price: Math.min(data.max_fee || 10, 5),
    model: model,
    node_name: MINER_NAME,
  }, p2p.NODE_ID);

  p2p.broadcast(claim);
  console.log(`[BTCPC Inference] Claimed ${reqId.slice(0, 8)} (model: ${model}, waited: ${Math.round(jitter)}ms)`);
}

/**
 * Handle assignment — if we're assigned, prepare to receive payload.
 */
async function handleAssignment(msg) {
  const data = msg.data || msg;
  const myNodeId = p2p.NODE_ID;

  const assigned = (data.assignments || []).find(
    a => a.node_id === myNodeId || a.node_name === MINER_NAME
  );

  if (!assigned) return; // not assigned to us

  console.log(`[BTCPC Inference] Assigned to request ${data.request_id?.slice(0, 8)}`);
  activeJobs++;
}

/**
 * Handle encrypted payload — decrypt, run inference, commit result.
 * For the initial version, we accept plaintext prompts directly
 * (encryption layer added when SIK is compiled on the node).
 */
async function handlePayload(msg) {
  const data = msg.data || msg;
  const requestId = data.request_id;

  // Deduplicate — only process each payload once
  const payloadKey = 'payload_' + requestId;
  if (!requestId || seenRequests.has(payloadKey)) return;
  seenRequests.add(payloadKey);

  // Check if this payload is for us
  const targetNode = data.node_id || data.target_node;
  if (targetNode && targetNode !== p2p.NODE_ID && targetNode !== MINER_NAME) return;
  let prompt = data.prompt; // plaintext for now; may be overwritten by decryption below
  const model = data.model || "qwen3.5:27b";
  const maxTokens = data.max_tokens || 1024;
  const temperature = data.temperature || 0.7;
  const toolTraceHash = data.tool_trace_hash || null;
  const toolsUsed = data.tools_used || null;

  // ── Encrypted inference: ECDH decrypt prompt, will re-encrypt result ──
  let sharedSecret = null;
  if (data.encrypted === true && data.prompt_encrypted && data.user_memo_pubkey) {
    // Only handle encrypted jobs that target this miner account (or are broadcast to all)
    if (data.miner_account && data.miner_account !== MINER_NAME) return;

    try {
      const mnemonic = process.env.BTCPC_MNEMONIC;
      if (!mnemonic) {
        console.error("[BTCPC Inference] Encrypted job received but BTCPC_MNEMONIC not set — cannot decrypt");
        return;
      }
      const keyManager = require("../wallet/keyManager");
      const keys = await keyManager.mnemonicToKeys(mnemonic);
      const minerMemoPriv = keys.memo.privateKey;

      sharedSecret = inferenceCrypto.computeSharedSecret(minerMemoPriv, data.user_memo_pubkey);
      prompt = inferenceCrypto.decrypt(data.prompt_encrypted, sharedSecret);
      console.log(`[BTCPC Inference] Decrypted encrypted prompt for ${requestId?.slice(0, 8)} (${prompt.length} chars)`);
    } catch (decryptErr) {
      console.error(`[BTCPC Inference] Failed to decrypt prompt for ${requestId?.slice(0, 8)}: ${decryptErr.message}`);
      activeJobs = Math.max(0, activeJobs - 1);
      if (jobInFlight && jobInFlight.request_id === requestId) {
        clearJobInFlight('decrypt failed');
      }
      return;
    }
  }

  if (!prompt) {
    console.log(`[BTCPC Inference] No prompt in payload for ${requestId?.slice(0, 8)}`);
    activeJobs = Math.max(0, activeJobs - 1);
    if (jobInFlight && jobInFlight.request_id === requestId) {
      clearJobInFlight('no prompt');
    }
    return;
  }

  console.log(`[BTCPC Inference] Processing request ${requestId?.slice(0, 8)} (${prompt.length} chars, model: ${model})`);

  try {
    // Run inference via Ollama chat endpoint (works with both chat and completion models)
    const startTime = Date.now();
    modelActiveJobs.set(model, (modelActiveJobs.get(model) || 0) + 1);

    // Parse prompt back into messages if it contains System: prefix, otherwise use as user message
    const messages = [];
    const systemMatch = prompt.match(/^System:\s*([\s\S]*?)(?=\n\n|$)/);
    if (systemMatch) {
      messages.push({ role: 'system', content: systemMatch[1].trim() });
      const userContent = prompt.slice(systemMatch[0].length).trim();
      if (userContent) messages.push({ role: 'user', content: userContent });
    } else {
      messages.push({ role: 'user', content: prompt });
    }

    const response = await axios.post(`${OLLAMA_URL}/api/chat`, {
      model,
      messages,
      stream: false,
      think: false, // disable thinking/reasoning mode — we want content directly
      options: { temperature, num_predict: maxTokens },
    }, { timeout: 600000 }); // 10 min — large models on busy GPU need time

    // Some models (qwen3.5) put output in 'thinking' field when reasoning mode is on
    const msg = response.data.message || {};
    let resultText = msg.content || msg.thinking || response.data.response || "";
    // Strip thinking leaks — qwen3 sometimes exposes reasoning even with think:false
    resultText = resultText.replace(/^(Okay, the user|Hmm,|Interesting|First, I need to|Let me unpack|Let me think)[^\n]*\n+/i, '').trim();
    const tokensGenerated = response.data.eval_count || Math.ceil(resultText.length / 4);
    const elapsed = Date.now() - startTime;

    const resultHash = crypto.createHash("sha256").update(resultText).digest("hex");
    const promptHash = crypto.createHash("sha256").update(prompt).digest("hex");

    // Store work proof in stateStore (no Mongo)
    const weightFactor = getModelWeight(model);
    const workValue = tokensGenerated * weightFactor;
    const currentEpochForProof = p2p.getCurrentEpoch ? p2p.getCurrentEpoch() : 0;
    if (stateStore.addComputeProof) {
      stateStore.addComputeProof(currentEpochForProof, {
        node_id: MINER_NAME,
        prompt_hash: promptHash,
        result_hash: resultHash,
        model,
        tokens_generated: tokensGenerated,
        model_weight_factor: weightFactor,
        work_value: workValue,
        job_id: requestId,
        tool_trace_hash: toolTraceHash,
        tools_used: toolsUsed,
        completed_at: new Date().toISOString()
      });
    }

    // Commit result hash
    const commit = createMessage("INFERENCE_COMMIT", {
      request_id: requestId,
      node_id: p2p.NODE_ID,
      node_name: MINER_NAME,
      result_hash: resultHash,
      tokens_generated: tokensGenerated,
    }, p2p.NODE_ID);
    p2p.broadcast(commit);

    // currentEpoch already captured above as currentEpochForProof
    const currentEpoch = currentEpochForProof;

    // Immediately reveal (single-node mode; multi-node waits for all commits)
    const reveal = createMessage("INFERENCE_REVEAL", {
      request_id: requestId,
      node_id: p2p.NODE_ID,
      node_name: MINER_NAME,
      result_hash: resultHash,
      result_text: resultText, // plaintext for now; encrypted in production
      tokens_generated: tokensGenerated,
      model,
      model_weight: weightFactor,
      work_value: workValue,
      epoch_number: currentEpoch,
      elapsed_ms: elapsed,
      work_proof: { prompt_hash: promptHash, result_hash: resultHash },
      tool_trace_hash: toolTraceHash,
      tools_used: toolsUsed,
    }, p2p.NODE_ID);
    p2p.broadcast(reveal);

    // For encrypted jobs: encrypt the result before broadcasting.
    // Plaintext result_text is withheld from the P2P message.
    let resultEncrypted = null;
    if (sharedSecret) {
      try {
        resultEncrypted = inferenceCrypto.encrypt(resultText, sharedSecret);
        console.log(`[BTCPC Inference] Result encrypted for ${requestId?.slice(0, 8)}`);
      } catch (encErr) {
        console.error(`[BTCPC Inference] Failed to encrypt result for ${requestId?.slice(0, 8)}: ${encErr.message}`);
      }
    }

    // Also broadcast as INFERENCE_RESULT for the requester
    const result = createMessage("INFERENCE_RESULT", {
      request_id: requestId,
      // Only include plaintext for non-encrypted jobs
      result_text: sharedSecret ? undefined : resultText,
      result_encrypted: resultEncrypted || undefined,
      encrypted: sharedSecret ? true : undefined,
      result_hash: resultHash,
      tokens_generated: tokensGenerated,
      model,
      model_weight: weightFactor,
      work_value: workValue,
      epoch_number: currentEpoch,
      elapsed_ms: elapsed,
      node_name: MINER_NAME,
    }, p2p.NODE_ID);
    p2p.broadcast(result);

    // Broadcast VERIFY_REQUEST — verifiers see full response but NOT the prompt
    try {
      const latestEpoch = stateStore.getLatestEpoch ? stateStore.getLatestEpoch() : null;
      const epochForVerify = latestEpoch ? (latestEpoch.epoch_number || 0) : 0;
      const verifyReq = createMessage("VERIFY_REQUEST", {
        job_id: requestId,
        result: resultText,
        model,
        token_count: tokensGenerated,
        timing_ms: elapsed,
        miner: MINER_NAME,
        epoch: epochForVerify,
        block_hash: latestEpoch ? (latestEpoch.consensus_hash || "0".repeat(64)) : "0".repeat(64)
      }, p2p.NODE_ID);
      p2p.broadcast(verifyReq);
    } catch (verifyErr) {
      console.error(`[BTCPC Inference] Failed to broadcast VERIFY_REQUEST: ${verifyErr.message}`);
    }

    console.log(`[BTCPC Inference] Completed ${requestId?.slice(0, 8)}: ${tokensGenerated} tokens, ${elapsed}ms`);
    recordModelResult(model, elapsed, tokensGenerated, true);
  } catch (err) {
    console.error(`[BTCPC Inference] Failed ${requestId?.slice(0, 8)}:`, err.message);
    recordModelResult(model, 0, 0, false);

    // Broadcast failure so requester knows
    const fail = createMessage("INFERENCE_RESULT", {
      request_id: requestId,
      error: err.message,
      node_name: MINER_NAME,
    }, p2p.NODE_ID);
    p2p.broadcast(fail);
  } finally {
    activeJobs = Math.max(0, activeJobs - 1);
    modelActiveJobs.set(model, Math.max(0, (modelActiveJobs.get(model) || 0) - 1));
    // Clear in-flight slot if this was our job
    if (jobInFlight && jobInFlight.request_id === requestId) {
      clearJobInFlight('local processing complete');
    }
  }
}

/**
 * Handle MODEL_DEMAND broadcast — log so miners know what to pull.
 */
function handleModelDemand(msg) {
  const data = msg.data || msg;
  console.log(`[BTCPC Inference] \u{1F4E2} MODEL DEMAND: "${data.model}" — ${data.demand} request(s) waiting. Pull this model to earn from unmet demand.`);
}

module.exports = {
  startInferenceHandler,
  _modelRouting: {
    chooseModel,
    getModelStats,
    modelStats,
    modelActiveJobs,
    parseModelParams,
    promptComplexity,
    recordModelResult
  }
};
