"use strict";

/**
 * Inference Marketplace — Verasens / BTCPC Network
 * Shin Devlin
 *
 * Connects compute buyers to miners. Buyers post jobs with a BTCPC escrow.
 * Miners claim, run inference via Ollama, submit a result hash. Reviewers
 * verify work, challenge windows gate disputes, and finality closes the job.
 * Protocol takes 10% of the inference payout.
 *
 * Job lifecycle:
 *   open → claimed → submitted → reviewed → finalized
 *                 ↘ expired → refunded
 *                ↘ challenged → appeal-reviewed → finalized
 *
 * All money moves through the existing escrow system. Job metadata is
 * tracked via INFERENCE_JOB_* ledger entries and the inferenceJobs Map
 * in stateStore (rebuilt on replay, same as commerce orders).
 */

const crypto = require("crypto");
const stateStore = require("../chain/stateStore");
const nodeRegistry = require("../chain/nodeRegistry");
const ledger = require("./ledger");
const protocolTools = require("./protocolTools");

const PROTOCOL_FEE_ACCOUNT = "btcpc_fees";
const PROTOCOL_FEE_PCT = 0.10;
const DEFAULT_TTL_EPOCHS = 20; // 10 minutes at 30s/epoch
const DEFAULT_REVIEW_FEE_PCT = 0.05;
const DEFAULT_CHALLENGE_FEE_PCT = 0.02;
const DEFAULT_CHALLENGE_WINDOW_EPOCHS = parseInt(process.env.BTCPC_CHALLENGE_WINDOW_EPOCHS, 10) || 2880;
const MIN_JOB_FEE = 0.01; // 0.01 BTCPC minimum

function _jobId() {
  return "job_" + crypto.randomBytes(8).toString("hex");
}

function _challengeEscrowId(jobId) {
  return `${jobId}:challenge`;
}

function _jobFinalityHash(job) {
  return crypto
    .createHash("sha256")
    .update(JSON.stringify({
      job_id: job.job_id,
      buyer: job.buyer,
      miner: job.miner,
      review_mode: job.review_mode,
      review_verdict: job.review_verdict,
      review_stage: job.review_stage,
      challenge_status: job.challenge_status,
      challenge_reason: job.challenge_reason,
      finality_epoch: job.finality_epoch,
      actual_cost: job.actual_cost,
      max_fee: job.max_fee,
      review_fee: job.review_fee,
      challenge_fee: job.challenge_fee,
    }))
    .digest("hex");
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

  const reviewMode = ["computer", "human"].includes(opts.reviewMode || opts.review_mode)
    ? (opts.reviewMode || opts.review_mode)
    : "computer";
  const reviewFee = opts.reviewFee != null
    ? parseFloat(opts.reviewFee)
    : parseFloat((maxFee * DEFAULT_REVIEW_FEE_PCT).toFixed(10));
  const challengeFee = opts.challengeFee != null
    ? parseFloat(opts.challengeFee)
    : parseFloat((maxFee * DEFAULT_CHALLENGE_FEE_PCT).toFixed(10));
  const challengeWindowEpochs = Math.max(
    parseInt(opts.challengeWindowEpochs || opts.challenge_window_epochs || DEFAULT_CHALLENGE_WINDOW_EPOCHS, 10) || DEFAULT_CHALLENGE_WINDOW_EPOCHS,
    1
  );
  const totalEscrow = parseFloat((maxFee + reviewFee).toFixed(10));

  const balance = stateStore.getBalance(buyer, "BTCPC");
  if (balance < totalEscrow) {
    throw new Error(`Insufficient balance: have ${balance} BTCPC, need ${totalEscrow}`);
  }

  const jobId = _jobId();
  const epoch = await ledger.getCurrentEpoch();
  const ttlEpochs = opts.ttlEpochs || DEFAULT_TTL_EPOCHS;

  // Lock escrow first
  await ledger.recordEscrowLock(buyer, jobId, totalEscrow, epoch);

  const tools = Array.isArray(opts.tools) ? opts.tools : [];
  const maxTurns = Math.min(parseInt(opts.maxTurns) || 1, 20);
  const outputSchema = opts.outputSchema || null;
  const tier = ["standard", "reasoning", "fast"].includes(opts.tier) ? opts.tier : "standard";
  const ragCids = Array.isArray(opts.ragCids) ? opts.ragCids.slice(0, 10) : [];
  const imageCids = Array.isArray(opts.imageCids) ? opts.imageCids.slice(0, 5) : [];
  const audioCid = opts.audioCid || null;
  const batchId = opts.batchId || null;
  const sessionId = opts.sessionId || null;
  const autoMemory = !!opts.autoMemory;
  const memoryProject = opts.memoryProject || null;

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
    escrow_amount: totalEscrow,
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
    auto_memory: autoMemory,
    memory_project: memoryProject,
    review_mode: reviewMode,
    review_fee: reviewFee,
    challenge_fee: challengeFee,
    challenge_window_epochs: challengeWindowEpochs,
    challenge_deadline_epoch: 0,
    review_stage: "initial",
  }, epoch);

  // Register job with session if provided
  if (sessionId) {
    try { await ledger.recordSessionAddJob(sessionId, jobId, epoch); } catch (_) {}
  }

  return {
    job_id: jobId,
    buyer,
    max_fee: maxFee,
    escrow_amount: totalEscrow,
    review_fee: reviewFee,
    challenge_fee: challengeFee,
    challenge_window_epochs: challengeWindowEpochs,
    status: "open",
    epoch,
    tools,
    max_turns: maxTurns,
    tier,
    review_mode: reviewMode,
    image_cids: imageCids,
    audio_cid: audioCid,
    batch_id: batchId,
    session_id: sessionId,
    auto_memory: autoMemory,
    memory_project: memoryProject,
  };
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
async function submitJob(jobId, miner, result, proofHash, actualCost) {
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

  const submittedCost = Math.min(
    actualCost != null ? actualCost : job.max_fee,
    job.max_fee
  );
  await ledger.recordInferenceJobSubmit(jobId, miner, proofHash, submittedCost, epoch);
  return { job_id: jobId, miner, status: "submitted", proof_hash: proofHash, actual_cost: submittedCost, epoch };
}

/**
 * Record an initial verifier review or an appeal review.
 * Initial review opens the challenge window. Appeal review resolves
 * challenged work and can be finalized immediately.
 */
async function reviewJob(jobId, reviewer, verdict, reviewData) {
  const job = stateStore.getInferenceJob(jobId);
  if (!job) throw new Error("Job not found: " + jobId);
  if (!reviewer) throw new Error("reviewer required");
  if (reviewer === job.buyer || reviewer === job.miner) {
    throw new Error("Reviewer cannot be the buyer or miner");
  }
  if (!["submitted", "review_rejected", "challenged", "awaiting_challenge", "review_voting"].includes(job.status)) {
    throw new Error("Job not in reviewable state (status: " + job.status + ")");
  }

  const epoch = await ledger.getCurrentEpoch();
  const reviewMode = reviewData && reviewData.review_mode ? reviewData.review_mode : job.review_mode || "computer";
  const reviewStage = job.status === "challenged" ? "appeal" : (reviewData && reviewData.review_stage) || "initial";
  const normalizedVerdict = (verdict === "rejected" || verdict === false) ? "rejected" : "accepted";
  const challengeWindowEpochs = Math.max(
    parseInt((reviewData && reviewData.challenge_window_epochs) || job.challenge_window_epochs || DEFAULT_CHALLENGE_WINDOW_EPOCHS, 10) || DEFAULT_CHALLENGE_WINDOW_EPOCHS,
    1
  );
  const reviewStatus = reviewStage === "appeal"
    ? "challenged"
    : (normalizedVerdict === "accepted" ? "awaiting_challenge" : "review_rejected");
  const reviewDeadline = reviewStage === "appeal"
    ? job.challenge_deadline_epoch || (epoch + challengeWindowEpochs)
    : (epoch + challengeWindowEpochs);
  const appealChallengeStatus = reviewStage === "appeal"
    ? (normalizedVerdict === "accepted" ? "denied" : "upheld")
    : null;

  await ledger.recordInferenceJobReview(jobId, reviewer, {
    verdict: normalizedVerdict,
    review_mode: reviewMode,
    review_fee: job.review_fee,
    challenge_window_epochs: challengeWindowEpochs,
    challenge_deadline_epoch: reviewDeadline,
    review_stage: reviewStage,
    challenge_status: appealChallengeStatus,
    status: reviewStatus,
  }, epoch);

  const updated = stateStore.getInferenceJob(jobId);
  if (!updated) throw new Error("Job not found after review update");

  if (reviewStage === "appeal") {
    const normalizedVerdictForVote = verdict === "fraud" ? "fraud" : "non_fraud";
    await ledger.recordInferenceJobReviewVote(jobId, reviewer, {
      verdict: normalizedVerdictForVote,
      review_mode: reviewMode,
      review_stage: "appeal",
      status: "review_voting",
    }, epoch);

    const updatedJob = stateStore.getInferenceJob(jobId);
    const assignedReviewers = (job.assigned_reviewers || []);
    const appealVotes = ((updatedJob && updatedJob.review_votes) || []).filter(function(v) {
      return v.review_stage === "appeal";
    });

    if (appealVotes.length < assignedReviewers.length) {
      return {
        job_id: jobId,
        status: "review_voting",
        reviewer,
        verdict: normalizedVerdictForVote,
        review_stage: "appeal",
        votes_received: appealVotes.length,
        votes_needed: assignedReviewers.length,
        epoch,
      };
    }

    // All votes in — compute majority
    const fraudCount = appealVotes.filter(function(v) { return v.verdict === "fraud"; }).length;
    const nonFraudCount = appealVotes.filter(function(v) { return v.verdict === "non_fraud"; }).length;
    const majorityVerdict = fraudCount >= nonFraudCount ? "fraud" : "non_fraud";
    const challengeStatus = majorityVerdict === "fraud" ? "upheld" : "denied";

    const dissenters = appealVotes
      .filter(function(v) { return v.verdict !== majorityVerdict; })
      .map(function(v) { return v.reviewer; });

    for (var di = 0; di < dissenters.length; di++) {
      const dissenter = dissenters[di];
      const stakePool = stateStore.getStakePool(dissenter);
      const currentStake = stakePool ? (stakePool.total_staked || 0) : 0;
      const slashAmount = parseFloat((currentStake * 0.02).toFixed(10));
      if (slashAmount > 0) {
        await ledger.recordSlash(dissenter, slashAmount, epoch, "review_dissent", {
          account: dissenter,
          role: "verifier",
          offenseType: "REVIEW_DISSENT",
          tier: 0,
        });
        const updatedPool = stateStore.getStakePool(dissenter);
        nodeRegistry.updateStake(dissenter, updatedPool ? updatedPool.total_staked : 0);
      }
      await ledger.recordNodeReputationUpdate(dissenter, "review", false, epoch);
    }

    await ledger.recordInferenceJobReviewOutcome(jobId, {
      review_verdict: majorityVerdict,
      challenge_status: challengeStatus,
      review_winner: majorityVerdict === "fraud" ? "challenger" : "miner",
      review_dissenters: dissenters,
      review_vote_count: appealVotes.length,
    }, epoch);

    const finalResult = await finalizeJob(jobId, reviewer);
    return Object.assign({}, finalResult, { review_outcome: majorityVerdict });
  }

  return {
    job_id: jobId,
    status: reviewStatus,
    reviewer,
    verdict: normalizedVerdict,
    review_stage: reviewStage,
    challenge_deadline_epoch: reviewDeadline,
    challenge_window_epochs: challengeWindowEpochs,
    epoch,
  };
}

/**
 * Refund an expired or cancelled job back to the buyer.
 */
async function refundJob(jobId) {
  const job = stateStore.getInferenceJob(jobId);
  if (!job) throw new Error("Job not found: " + jobId);
  if (job.status === "finalized" || job.status === "settled" || job.status === "refunded") {
    throw new Error("Job already " + job.status);
  }

  const epoch = await ledger.getCurrentEpoch();
  await _expireJob(jobId, job, epoch);
  return { job_id: jobId, status: "refunded", buyer: job.buyer, amount: job.max_fee };
}

async function _expireJob(jobId, job, epoch) {
  await ledger.recordEscrowRefund(job.buyer, jobId, job.escrow_amount || job.max_fee, epoch);
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
    if (job.status === "open" && job.expires_epoch && epoch > job.expires_epoch) {
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

async function challengeJob(jobId, challenger, reason) {
  const job = stateStore.getInferenceJob(jobId);
  if (!job) throw new Error("Job not found: " + jobId);
  if (!challenger) throw new Error("challenger required");
  if (!["awaiting_challenge", "review_rejected"].includes(job.status)) {
    throw new Error("Job not challengeable (status: " + job.status + ")");
  }

  const epoch = await ledger.getCurrentEpoch();
  const deadline = job.challenge_deadline_epoch || 0;
  if (deadline && epoch > deadline) {
    throw new Error("Challenge window closed");
  }
  if (job.buyer !== challenger) {
    throw new Error("Only the buyer may challenge this job");
  }

  const challengeFee = parseFloat(job.challenge_fee || 0);
  const challengeEscrowId = _challengeEscrowId(jobId);
  if (challengeFee > 0) {
    await ledger.recordEscrowLock(challenger, challengeEscrowId, challengeFee, epoch);
  }

  // Select appeal reviewers: verifiers excluding buyer and miner, sorted by stake desc
  const allNodes = nodeRegistry.getRegisteredNodes();
  const candidateReviewers = allNodes
    .filter(n => n.type === "verifier" && n.username !== job.buyer && n.username !== job.miner)
    .map(n => n.username);
  const assignedReviewers = candidateReviewers.slice(0, 3);

  await ledger.recordInferenceJobChallenge(jobId, challenger, {
    reason: reason || "quality_dispute",
    challenge_fee: challengeFee,
    challenge_bond: challengeFee,
    challenge_escrow_id: challengeEscrowId,
    challenge_deadline_epoch: deadline,
    assigned_reviewers: assignedReviewers,
  }, epoch);

  return {
    job_id: jobId,
    status: "challenged",
    challenger,
    challenge_fee: challengeFee,
    challenge_deadline_epoch: deadline,
    assigned_reviewers: assignedReviewers,
    epoch,
  };
}

async function finalizeJob(jobId, finalizer) {
  const job = stateStore.getInferenceJob(jobId);
  if (!job) throw new Error("Job not found: " + jobId);
  if (job.status === "finalized" || job.status === "refunded") {
    throw new Error("Job already " + job.status);
  }
  if (job.status === "submitted") {
    throw new Error("Job not reviewed yet");
  }

  const epoch = await ledger.getCurrentEpoch();
  const challengeEscrowId = _challengeEscrowId(jobId);
  const actualCost = Math.min(
    job.actual_cost != null ? job.actual_cost : job.max_fee,
    job.max_fee
  );
  const protocolFee = parseFloat((actualCost * PROTOCOL_FEE_PCT).toFixed(10));
  const minerPayout = parseFloat((actualCost - protocolFee).toFixed(10));
  const reviewPayout = parseFloat((job.review_fee || 0).toFixed(10));
  const inferenceRefund = parseFloat((job.max_fee - actualCost).toFixed(10));
  const reviewStage = job.review_stage || "initial";
  const finalityHash = _jobFinalityHash(job);

  if (job.status === "challenged" && !["upheld", "denied"].includes(job.challenge_status || "")) {
    throw new Error("Challenge has not been resolved yet");
  }
  if ((job.status === "awaiting_challenge" || job.status === "review_rejected") && job.challenge_deadline_epoch && epoch <= job.challenge_deadline_epoch) {
    throw new Error("Challenge window still open");
  }

  if (job.challenge_status === "upheld") {
    if (reviewPayout > 0) {
      await ledger.recordEscrowRelease(job.reviewer || PROTOCOL_FEE_ACCOUNT, jobId, reviewPayout, epoch, "Inference review fee");
    }
    if (job.max_fee > 0) {
      await ledger.recordEscrowRefund(job.buyer, jobId, job.max_fee, epoch);
    }
    if (challengeEscrowId && job.challenge_fee > 0) {
      await ledger.recordEscrowRelease(PROTOCOL_FEE_ACCOUNT, challengeEscrowId, job.challenge_fee, epoch, "Inference challenge fee — appeal process");
    }
  } else if (job.status === "review_rejected" && !job.challenge_status) {
    if (reviewPayout > 0) {
      await ledger.recordEscrowRelease(job.reviewer || PROTOCOL_FEE_ACCOUNT, jobId, reviewPayout, epoch, "Inference review fee");
    }
    if (job.max_fee > 0) {
      await ledger.recordEscrowRefund(job.buyer, jobId, job.max_fee, epoch);
    }
  } else if (job.challenge_status === "denied" || job.status === "awaiting_challenge" || job.status === "review_rejected") {
    if (minerPayout > 0) {
      await ledger.recordEscrowRelease(job.miner, jobId, minerPayout, epoch, "Inference job payout");
    }
    if (protocolFee > 0) {
      await ledger.recordEscrowRelease(PROTOCOL_FEE_ACCOUNT, jobId, protocolFee, epoch, "Inference protocol fee 10%");
    }
    if (reviewPayout > 0) {
      await ledger.recordEscrowRelease(job.reviewer || PROTOCOL_FEE_ACCOUNT, jobId, reviewPayout, epoch, "Inference review fee");
    }
    if (inferenceRefund > 0.000001) {
      await ledger.recordEscrowRefund(job.buyer, jobId, inferenceRefund, epoch);
    }
    if (job.challenge_status === "denied" && challengeEscrowId && job.challenge_fee > 0) {
      await ledger.recordEscrowRelease(PROTOCOL_FEE_ACCOUNT, challengeEscrowId, job.challenge_fee, epoch, "Inference challenge fee forfeited");
    }
  }

  await ledger.recordInferenceJobFinality(jobId, {
    finality_epoch: epoch,
    finality_hash: finalityHash,
    finality_outcome: job.challenge_status || (job.review_verdict || "accepted"),
    review_stage: reviewStage,
    challenge_status: job.challenge_status || null,
    status: "finalized",
    challenge_closed: true,
  }, epoch);

  try {
    if (job.miner) {
      await ledger.recordNodeReputationUpdate(job.miner, "inference", job.challenge_status !== "upheld", epoch);
    }
  } catch (_) {}

  return {
    job_id: jobId,
    status: "finalized",
    miner: job.miner,
    buyer: job.buyer,
    finality_epoch: epoch,
    finality_hash: finalityHash,
    challenge_status: job.challenge_status || "non_fraud",
    review_stage: reviewStage,
  };
}

async function sweepReadyForFinality() {
  const epoch = await ledger.getCurrentEpoch();
  const jobs = stateStore.getInferenceJobsAwaitingFinality();
  let finalized = 0;

  for (const job of jobs) {
    try {
      if (job.status === "submitted") continue;
      if (job.status === "awaiting_challenge" || job.status === "review_rejected") {
        if (job.challenge_deadline_epoch && epoch <= job.challenge_deadline_epoch) continue;
        await finalizeJob(job.job_id, "sweep");
        finalized++;
        continue;
      }
      if (job.status === "challenged" && ["upheld", "denied"].includes(job.challenge_status || "")) {
        await finalizeJob(job.job_id, "sweep");
        finalized++;
      }
    } catch (_) {}
  }

  if (finalized > 0) {
    console.log(`[InferenceMarket] Finalized ${finalized} reviewed jobs at epoch ${epoch}`);
  }
  return { finalized, epoch };
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
  reviewJob,
  challengeJob,
  finalizeJob,
  refundJob,
  sweepExpiredJobs,
  sweepReadyForFinality,
  getOpenJobs,
  getBuyerJobs,
  getMinerJobs,
  PROTOCOL_FEE_PCT,
  MIN_JOB_FEE,
};
