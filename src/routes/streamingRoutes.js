"use strict";

/**
 * Streaming inference routes — real-time token output via SSE.
 * Shin Devlin
 *
 * POST /api/inference/stream — direct streaming inference (auth required).
 *   Bypasses the marketplace. Buyer pays per-token from their balance.
 *   Streams tokens back as SSE events in OpenAI-compatible format.
 *
 * GET /api/jobs/:id/stream — replay a settled job's result as SSE.
 *   Useful for clients that want to stream display of already-complete jobs.
 */

const express = require("express");
const router = express.Router();
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");
const axios = require("axios");
const { authenticateToken } = require("../middlewares/auth");
const { sanitizeString, sanitizeAmount } = require("../middlewares/validate");
const stateStore = require("../chain/stateStore");
const ledger = require("../services/ledger");

const OLLAMA_URL = process.env.OLLAMA_URL || "http://localhost:11434";
const MIN_STREAM_FEE = 0.001;
const COST_PER_TOKEN = 0.000001;
const MAX_PROMPT_LENGTH = 8000;
const BLOB_DIR = process.env.BTCPC_BLOB_DIR || path.resolve(__dirname, "../../data/blobs");

const VISION_MODEL_PATTERNS = [/llava/i, /bakllava/i, /moondream/i, /minicpm-v/i, /qwen.*vl/i];
function _isVisionModel(name) { return VISION_MODEL_PATTERNS.some((re) => re.test(name)); }

function _pickStreamModel(tier, hasImages, requestedModel) {
  if (hasImages) return requestedModel || process.env.BTCPC_VISION_MODEL || "llava:7b";
  if (tier === "reasoning") return requestedModel || process.env.BTCPC_REASONING_MODEL || "qwq:32b";
  if (tier === "fast") return requestedModel || process.env.BTCPC_FAST_MODEL || "qwen3:0.6b";
  return requestedModel || process.env.BTCPC_MODEL || "qwen3:4b";
}

function _storeInlineBlob(b64, maxBytes) {
  try {
    const buf = Buffer.from(b64, "base64");
    if (buf.length > maxBytes) return null;
    if (!fs.existsSync(BLOB_DIR)) fs.mkdirSync(BLOB_DIR, { recursive: true });
    const cid = crypto.createHash("sha256").update(buf).digest("hex");
    fs.writeFileSync(path.join(BLOB_DIR, cid), buf);
    return cid;
  } catch (_) { return null; }
}

// ── POST /api/inference/stream ────────────────────────────────────────────────
// Direct streaming inference — bypasses marketplace, instant response.
// Charges per-token from buyer's wallet after completion.
router.post("/stream", authenticateToken, async (req, res) => {
  const buyer = req.user.username;
  const prompt = sanitizeString(req.body.prompt, MAX_PROMPT_LENGTH);
  const model = req.body.model ? sanitizeString(req.body.model, 100) : null;
  const systemPrompt = req.body.system_prompt
    ? sanitizeString(req.body.system_prompt, 2000) : null;
  const outputSchema = req.body.output_schema || null;
  const tier = req.body.tier || "standard";
  const maxFee = sanitizeAmount(req.body.max_fee) || 0.1;

  if (!prompt) return res.status(400).json({ error: "prompt required" });

  const balance = stateStore.getBalance(buyer, "BTCPC");
  if (balance < MIN_STREAM_FEE) {
    return res.status(402).json({ error: `Insufficient balance: need at least ${MIN_STREAM_FEE} BTCPC` });
  }

  // Images: accept pre-uploaded CIDs or inline base64 (max 4MB each, max 5)
  let imageCids = Array.isArray(req.body.image_cids) ? req.body.image_cids.slice(0, 5) : [];
  if (Array.isArray(req.body.images)) {
    for (const b64 of req.body.images.slice(0, 5)) {
      const cid = _storeInlineBlob(b64, 4 * 1024 * 1024);
      if (cid) imageCids.push(cid);
    }
  }
  const hasImages = imageCids.length > 0;

  // Audio: transcribe inline before streaming if provided
  let audioTranscript = null;
  const audioCid = req.body.audio_cid ? sanitizeString(req.body.audio_cid, 64) : null;
  const inlineAudio = req.body.audio;
  let resolvedAudioCid = audioCid;
  if (!resolvedAudioCid && inlineAudio) {
    resolvedAudioCid = _storeInlineBlob(inlineAudio, 10 * 1024 * 1024);
  }
  if (resolvedAudioCid) {
    try {
      const audioTranscriber = require("../services/audioTranscriber");
      audioTranscript = await audioTranscriber.transcribe(resolvedAudioCid);
      try { fs.unlinkSync(path.join(BLOB_DIR, resolvedAudioCid)); } catch (_) {}
    } catch (audioErr) {
      console.warn(`[Stream] Audio transcription failed: ${audioErr.message}`);
    }
  }

  const resolvedModel = _pickStreamModel(tier, hasImages, model);

  // Set up SSE
  res.setHeader("Content-Type", "text/event-stream");
  res.setHeader("Cache-Control", "no-cache");
  res.setHeader("Connection", "keep-alive");
  res.setHeader("X-Accel-Buffering", "no");
  res.flushHeaders();

  const send = (event, data) => {
    res.write(`event: ${event}\ndata: ${JSON.stringify(data)}\n\n`);
  };

  // Build messages — inject audio transcript, attach images
  const messages = [];
  if (systemPrompt) messages.push({ role: "system", content: systemPrompt });
  const userContent = audioTranscript
    ? `[Audio transcript]\n${audioTranscript}\n\n${prompt}`
    : prompt;
  const userMsg = { role: "user", content: userContent };

  if (hasImages) {
    const images = [];
    for (const cid of imageCids) {
      if (!/^[a-f0-9]{64}$/.test(cid)) continue;
      const blobPath = path.join(BLOB_DIR, cid);
      try { images.push(fs.readFileSync(blobPath).toString("base64")); } catch (_) {}
    }
    if (images.length > 0) userMsg.images = images;
  }
  messages.push(userMsg);

  const body = { model: resolvedModel, messages, stream: true };
  if (outputSchema) body.format = { type: "json", schema: outputSchema };

  let totalTokens = 0;
  let fullContent = "";

  try {
    const resp = await axios.post(`${OLLAMA_URL}/api/chat`, body, {
      responseType: "stream",
      timeout: 120000,
    });

    await new Promise((resolve, reject) => {
      resp.data.on("data", (chunk) => {
        const lines = chunk.toString().split("\n").filter(Boolean);
        for (const line of lines) {
          try {
            const parsed = JSON.parse(line);
            if (parsed.message && parsed.message.content) {
              const token = parsed.message.content;
              fullContent += token;
              totalTokens++;
              // OpenAI-compatible delta format
              send("delta", {
                choices: [{ delta: { content: token }, index: 0 }],
              });
            }
            if (parsed.done) {
              totalTokens = parsed.eval_count || totalTokens;
            }
          } catch (_) {}
        }
      });
      resp.data.on("end", resolve);
      resp.data.on("error", reject);
    });

    // Charge after streaming completes
    const costPerToken = parseFloat(process.env.BTCPC_STREAM_COST_PER_TOKEN) || COST_PER_TOKEN;
    const actualCost = Math.min(maxFee, parseFloat((totalTokens * costPerToken).toFixed(10)));

    try {
      if (actualCost > 0) {
        const epoch = await ledger.getCurrentEpoch();
        // Record as inference charge
        await ledger.recordInferenceCharge(buyer, actualCost, epoch, {
          tokens: totalTokens,
          model: resolvedModel,
          tier,
          stream: true,
        });
      }
    } catch (chargeErr) {
      console.warn(`[Stream] Charge failed for ${buyer}: ${chargeErr.message}`);
    }

    send("done", {
      tokens: totalTokens,
      cost_btcpc: actualCost,
      model: resolvedModel,
      images: imageCids.length > 0 ? imageCids.length : undefined,
      audio_transcribed: audioTranscript ? true : undefined,
    });
    res.end();
  } catch (err) {
    send("error", { error: err.message });
    res.end();
  }
});

// ── GET /api/jobs/:id/stream ──────────────────────────────────────────────────
// Replay a settled job's result as a token stream (simulated SSE).
// Useful for UIs that always use SSE for display.
router.get("/jobs/:id/stream", (req, res) => {
  const job = stateStore.getInferenceJob(req.params.id);
  if (!job) return res.status(404).json({ error: "Job not found" });
  if (job.status !== "settled") {
    return res.status(409).json({ error: `Job not settled yet (status: ${job.status})` });
  }

  res.setHeader("Content-Type", "text/event-stream");
  res.setHeader("Cache-Control", "no-cache");
  res.setHeader("Connection", "keep-alive");
  res.setHeader("X-Accel-Buffering", "no");
  res.flushHeaders();

  const send = (event, data) => {
    res.write(`event: ${event}\ndata: ${JSON.stringify(data)}\n\n`);
  };

  // Simulate streaming by emitting chunks of the stored result
  const result = job.result || "";
  const CHUNK_MS = 20;
  const CHUNK_CHARS = 8;
  let pos = 0;

  const interval = setInterval(() => {
    if (pos >= result.length) {
      send("done", { tokens: result.length, job_id: job.job_id, replayed: true });
      res.end();
      clearInterval(interval);
      return;
    }
    const chunk = result.slice(pos, pos + CHUNK_CHARS);
    send("delta", { choices: [{ delta: { content: chunk }, index: 0 }] });
    pos += CHUNK_CHARS;
  }, CHUNK_MS);

  req.on("close", () => clearInterval(interval));
});

module.exports = router;
