"use strict";

/**
 * Inference Marketplace — Verasens / BTCPC Network
 * Shin Devlin
 *
 * Connects compute buyers to miners. Buyers post jobs with a BTCPC escrow.
 * Miners claim, run inference via Ollama, submit a result hash. Verifier
 * nodes confirm. Protocol takes 10% on settlement.
 *
 * Job lifecycle:
 *   open → claimed → submitted → settled
 *                 ↘ expired → refunded
 *
 * All money moves through the existing escrow system. Job metadata is
 * tracked via INFERENCE_JOB_* ledger entries and the inferenceJobs Map
 * in stateStore (rebuilt on replay, same as commerce orders).
 */

const crypto = require("crypto");
const stateStore = require("../chain/stateStore");
const ledger = require("./ledger");
const protocolTools = require("./protocolTools");

const PROTOCOL_FEE_ACCOUNT = "btcpc_fees";
const PROTOCOL_FEE_PCT = 0.10;
const DEFAULT_TTL_EPOCHS = 20; // 10 minutes at 30s/epoch
const MIN_JOB_FEE = 0.01; // 0.01 BTCPC minimum

function _jobId() {
  return "job_" + crypto.randomBytes(8).toString("hex");
}

/**
 * Buyer opens a marketplace job. Escrow is locked immediately.
 * Returns the jobId that miners can claim.
 *
 * @param {string} buyer - BTCPC account name
 * @param {string} prompt - The inference prompt
 * @param {number} maxFee - BTCPC to escrow (miner earns 90%, protocol 10%)
 * @param {object} opts - { model, ttlEpochs, systemPrompt, streaming }
 */
async function openJob(buyer, prompt, maxFee, opts) {
  opts = opts || {};
  if (!buyer || !prompt) throw new Error("buyer and prompt required");
  if (!maxFee || maxFee < MIN_JOB_FEE) {
    throw new Error(`maxFee must be at least ${MIN_JOB_FEE} BTCPC`);
  }

  const balance = stateStore.getBalance(buyer, "BTCPC");
  if (balance < maxFee) {
    throw new Error(`Insufficient balance: have ${balance} BTCPC, need ${maxFee}`);
  }

  const jobId = _jobId();
  const epoch = await ledger.getCurrentEpoch();
  const ttlEpochs = opts.ttlEpochs || DEFAULT_TTL_EPOCHS;

  // Lock escrow first
  await ledger.recordEscrowLock(buyer, jobId, maxFee, epoch);

  const tools = Array.isArray(opts.tools) ? opts.tools : [];
  const maxTurns = Math.min(parseInt(opts.maxTurns) || 1, 20);
  const outputSchema = opts.outputSchema || null;
  const tier = ["standard", "reasoning", "fast"].includes(opts.tier) ? opts.tier : "standard";
  const ragCids = Array.isArray(opts.ragCids) ? opts.ragCids.slice(0, 10) : [];
  const imageCids = Array.isArray(opts.imageCids) ? opts.imageCids.slice(0, 5) : [];
  const audioCid = opts.audioCid || null;
  const batchId = opts.batchId || null;
  const sessionId = opts.sessionId || null;

  // If session_id provided, prepend session summary to system prompt
  let systemPrompt = opts.systemPrompt || null;
  if (sessionId) {
    const session = stateStore.getSession ? stateStore.getSession(sessionId) : null;
    if (session && session.summary) {
      const sessionCtx = `[Session context]\n${session.summary}\n\n`;
      systemPrompt = sessionCtx + (systemPrompt || "");
    }
    if (session && session.buyer && session.buyer !== buyer) {
      throw new Error("Session belongs to a different buyer");
    }
  }

  await ledger.recordInferenceJobOpen(buyer, jobId, {
    prompt,
    max_fee: maxFee,
    model: opts.model || null,
    system_prompt: systemPrompt,
    ttl_epochs: ttlEpochs,
    expires_epoch: epoch + ttlEpochs,
    tools,
    max_turns: maxTurns,
    output_schema: outputSchema,
    tier,
    rag_cids: ragCids,
    image_cids: imageCids,
    audio_cid: audioCid,
    batch_id: batchId,
    session_id: sessionId,
  }, epoch);

  // Register job with session if provided
  if (sessionId) {
    try { await ledger.recordSessionAddJob(sessionId, jobId, epoch); } catch (_) {}
  }

  return { job_id: jobId, buyer, max_fee: maxFee, status: "open", epoch, tools, max_turns: maxTurns, tier, image_cids: imageCids, audio_cid: audioCid, batch_id: batchId, session_id: sessionId };
}

/**
 * Miner claims an open job. Job transitions to 'claimed'.
 * Only one miner can claim a job at a time.
 *
 * @param {string} jobId
 * @param {string} miner - BTCPC account name
 */
async function claimJob(jobId, miner) {
  const job = stateStore.getInferenceJob(jobId);
  if (!job) throw new Error("Job not found: " + jobId);
  if (job.status !== "open") throw new Error("Job not open (status: " + job.status + ")");

  const epoch = await ledger.getCurrentEpoch();
  if (job.expires_epoch && epoch > job.expires_epoch) {
    // Auto-expire
    await _expireJob(jobId, job, epoch);
    throw new Error("Job expired");
  }

  await ledger.recordInferenceJobClaim(jobId, miner, epoch);

  // Return turns history so miner can reconstruct conversation context for multi-turn jobs
  const fresh = stateStore.getInferenceJob(jobId);
  return {
    job_id: jobId,
    miner,
    status: "claimed",
    epoch,
    turns: fresh ? (fresh.turns || []) : [],
    current_turn: fresh ? (fresh.current_turn || 0) : 0,
    tools: fresh ? (fresh.tools || []) : [],
    max_turns: fresh ? (fresh.max_turns || 1) : 1,
  };
}

/**
 * Miner submits tool calls it wants executed. Transitions job to 'tool_pending'.
 * Protocol-native tools are auto-executed by this function (no buyer round-trip).
 * External tools (web_fetch, web_search) are forwarded to the buyer.
 *
 * Returns { tool_pending, auto_resolved, buyer_tools } where:
 *   - auto_resolved: array of { tool_use_id, name, content, trusted } for protocol tools
 *   - buyer_tools: array of tool calls the buyer still needs to execute
 */
async function submitToolCalls(jobId, miner, toolCalls) {
  const job = stateStore.getInferenceJob(jobId);
  if (!job) throw new Error("Job not found: " + jobId);
  if (job.status !== "claimed") throw new Error("Job not claimed (status: " + job.status + ")");
  if (job.miner !== miner) throw new Error("Not your job");
  if (!Array.isArray(toolCalls) || toolCalls.length === 0) throw new Error("toolCalls array required");

  const epoch = await ledger.getCurrentEpoch();
  const turn = (job.current_turn || 0) + 1;

  if (turn > (job.max_turns || 1)) {
    throw new Error("max_turns exceeded — submit a final answer instead");
  }

  // Auto-execute any protocol-native or miner-executable tools right now
  const autoResolved = [];
  const buyerTools = [];

  for (const tc of toolCalls) {
    const name = (tc.function && tc.function.name) || tc.name || "";
    const rawInput = (tc.function && tc.function.arguments) || tc.arguments || tc.input || {};
    const input = typeof rawInput === "string" ? JSON.parse(rawInput) : rawInput;
    const id = tc.id || ("tc_" + crypto.randomBytes(4).toString("hex"));

    if (protocolTools.canMinerExecute(name)) {
      const result = await protocolTools.executeProtocolTool(name, input);
      autoResolved.push({
        tool_use_id: id,
        name,
        content: typeof result.content === "string" ? result.content : JSON.stringify(result.content),
        trusted: result.trusted,
        error: result.error || null,
      });
    } else {
      // Check if it's a registered webhook tool — call it now
      const registered = stateStore.getRegisteredTool ? stateStore.getRegisteredTool(name) : null;
      if (registered && registered.webhook_url) {
        try {
          const axios = require("axios");
          const hookResp = await axios.post(registered.webhook_url, {
            tool_use_id: id, name, input,
          }, { timeout: 15000 });
          const hookContent = hookResp.data && hookResp.data.content != null
            ? hookResp.data.content
            : JSON.stringify(hookResp.data);
          autoResolved.push({
            tool_use_id: id,
            name,
            content: typeof hookContent === "string" ? hookContent : JSON.stringify(hookContent),
            trusted: false,
            webhook: true,
            error: null,
          });
        } catch (hookErr) {
          autoResolved.push({
            tool_use_id: id, name,
            content: `Webhook error: ${hookErr.message}`,
            trusted: false, webhook: true, error: "webhook_error",
          });
        }
      } else {
        buyerTools.push({ ...tc, id });
      }
    }
  }

  // Record tool_call entry on chain
  await ledger.recordInferenceJobToolCall(jobId, miner, toolCalls, turn, epoch);

  // If all tools were auto-resolved, immediately feed them back as tool_results
  // so the job returns to 'claimed' for the next inference turn
  if (buyerTools.length === 0 && autoResolved.length > 0) {
    await ledger.recordInferenceJobToolResult(
      jobId, miner, autoResolved, turn, epoch
    );
    return {
      tool_pending: false,
      auto_resolved: autoResolved,
      buyer_tools: [],
      job_id: jobId,
      turn,
    };
  }

  return {
    tool_pending: true,
    auto_resolved: autoResolved,
    buyer_tools: buyerTools,
    job_id: jobId,
    turn,
  };
}

/**
 * Buyer submits tool execution results. Transitions job back to 'claimed'
 * so the miner can run the next inference turn.
 */
async function submitToolResults(jobId, buyer, toolResults) {
  const job = stateStore.getInferenceJob(jobId);
  if (!job) throw new Error("Job not found: " + jobId);
  if (job.status !== "tool_pending") throw new Error("Job not awaiting tool results (status: " + job.status + ")");
  if (job.buyer !== buyer) throw new Error("Not your job");
  if (!Array.isArray(toolResults) || toolResults.length === 0) throw new Error("toolResults array required");

  const epoch = await ledger.getCurrentEpoch();
  const turn = job.current_turn || 0;

  await ledger.recordInferenceJobToolResult(jobId, buyer, toolResults, turn, epoch);

  return {
    job_id: jobId,
    status: "claimed",
    turn,
    next_turn: turn + 1,
    epoch,
  };
}

/**
 * Miner submits inference result. Includes the response text and a hash
 * for verification. Job transitions to 'submitted'.
 *
 * @param {string} jobId
 * @param {string} miner
 * @param {string} result - The actual response text
 * @param {string} proofHash - sha256(prompt + result + miner) for light verification
 */
async function submitJob(jobId, miner, result, proofHash) {
  const job = stateStore.getInferenceJob(jobId);
  if (!job) throw new Error("Job not found: " + jobId);
  // Allow submission from 'claimed' (normal) or 'tool_pending' (miner got final answer after tools)
  if (job.status !== "claimed" && job.status !== "tool_pending") {
    throw new Error("Job not in submittable state (status: " + job.status + ")");
  }
  if (job.miner !== miner) throw new Error("Not your job");
  if (!result) throw new Error("result required");

  const epoch = await ledger.getCurrentEpoch();

  // Compute canonical proof hash if not provided
  if (!proofHash) {
    proofHash = crypto
      .createHash("sha256")
      .update(job.prompt + result + miner)
      .digest("hex");
  }

  await ledger.recordInferenceJobSubmit(jobId, miner, proofHash, epoch);
  return { job_id: jobId, miner, status: "submitted", proof_hash: proofHash, epoch };
}

/**
 * Settle a submitted job. Verifier (or auto-settle after timeout) calls this.
 * Releases 90% escrow to miner, 10% to btcpc_fees, refunds overpayment.
 *
 * @param {string} jobId
 * @param {number} actualCost - How much BTCPC the job actually cost (≤ maxFee)
 * @param {string} settledBy - Account settling (verifier or 'auto')
 */
async function settleJob(jobId, actualCost, settledBy) {
  const job = stateStore.getInferenceJob(jobId);
  if (!job) throw new Error("Job not found: " + jobId);
  if (job.status !== "submitted") throw new Error("Job not submitted (status: " + job.status + ")");

  const epoch = await ledger.getCurrentEpoch();

  // Clamp actual cost to max_fee
  actualCost = Math.min(actualCost || job.max_fee, job.max_fee);

  const protocolFee = parseFloat((actualCost * PROTOCOL_FEE_PCT).toFixed(10));
  const minerPayout = parseFloat((actualCost - protocolFee).toFixed(10));
  const overpayment = parseFloat((job.max_fee - actualCost).toFixed(10));

  // Release miner payout from escrow
  if (minerPayout > 0) {
    await ledger.recordEscrowRelease(job.miner, jobId, minerPayout, epoch, "Inference job settlement");
  }

  // Protocol fee — separate ESCROW_RELEASE to protocol account
  if (protocolFee > 0) {
    await ledger.recordEscrowRelease(PROTOCOL_FEE_ACCOUNT, jobId, protocolFee, epoch, "Inference protocol fee 10%");
  }

  // Refund overpayment to buyer
  if (overpayment > 0.000001) {
    await ledger.recordEscrowRefund(job.buyer, jobId, overpayment, epoch);
  }

  // Record settlement on chain for explorer + reputation
  await ledger.recordInferenceJobSettle(jobId, job.miner, job.buyer, {
    actual_cost: actualCost,
    miner_payout: minerPayout,
    protocol_fee: protocolFee,
    overpayment,
    settled_by: settledBy || "auto",
  }, epoch);

  // Update miner reputation
  try {
    await ledger.recordNodeReputationUpdate(job.miner, "inference", true, epoch);
  } catch (_) {}

  return {
    job_id: jobId,
    status: "settled",
    miner: job.miner,
    buyer: job.buyer,
    miner_payout: minerPayout,
    protocol_fee: protocolFee,
    overpayment,
  };
}

/**
 * Refund an expired or cancelled job back to the buyer.
 */
async function refundJob(jobId) {
  const job = stateStore.getInferenceJob(jobId);
  if (!job) throw new Error("Job not found: " + jobId);
  if (job.status === "settled" || job.status === "refunded") {
    throw new Error("Job already " + job.status);
  }

  const epoch = await ledger.getCurrentEpoch();
  await _expireJob(jobId, job, epoch);
  return { job_id: jobId, status: "refunded", buyer: job.buyer, amount: job.max_fee };
}

async function _expireJob(jobId, job, epoch) {
  await ledger.recordEscrowRefund(job.buyer, jobId, job.max_fee, epoch);
  await ledger.recordInferenceJobRefund(jobId, job.buyer, "expired", epoch);
}

/**
 * Sweep expired jobs — auto-refund any open/claimed jobs past their TTL.
 * Called periodically by the epoch manager.
 */
async function sweepExpiredJobs() {
  const epoch = await ledger.getCurrentEpoch();
  const open = stateStore.getOpenInferenceJobs();
  let swept = 0;

  for (const job of open) {
    if (job.expires_epoch && epoch > job.expires_epoch) {
      try {
        await _expireJob(job.job_id, job, epoch);
        swept++;
      } catch (_) {}
    }
  }

  if (swept > 0) {
    console.log(`[InferenceMarket] Swept ${swept} expired jobs at epoch ${epoch}`);
  }
  return { swept, epoch };
}

/**
 * Get paginated list of open jobs, optionally filtered by model.
 */
function getOpenJobs(opts) {
  opts = opts || {};
  const limit = Math.min(opts.limit || 50, 200);
  const model = opts.model || null;
  const offset = opts.offset || 0;

  let jobs = stateStore.getOpenInferenceJobs();
  if (model) jobs = jobs.filter((j) => j.model === model);
  return {
    jobs: jobs.slice(offset, offset + limit),
    total: jobs.length,
    limit,
    offset,
  };
}

/**
 * Get jobs posted by a specific buyer.
 */
function getBuyerJobs(buyer, opts) {
  opts = opts || {};
  const limit = Math.min(opts.limit || 50, 200);
  return stateStore.getInferenceJobsByBuyer(buyer).slice(0, limit);
}

/**
 * Get jobs claimed by a specific miner.
 */
function getMinerJobs(miner, opts) {
  opts = opts || {};
  const limit = Math.min(opts.limit || 50, 200);
  return stateStore.getInferenceJobsByMiner(miner).slice(0, limit);
}

module.exports = {
  openJob,
  claimJob,
  submitToolCalls,
  submitToolResults,
  submitJob,
  settleJob,
  refundJob,
  sweepExpiredJobs,
  getOpenJobs,
  getBuyerJobs,
  getMinerJobs,
  PROTOCOL_FEE_PCT,
  MIN_JOB_FEE,
};
