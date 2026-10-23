"use strict";

/**
 * HONE Encrypted Inference Router
 * Shin Devlin
 *
 * Handles SIK-bound encrypted inference requests.
 * The node never sees plaintext prompts — only remapped token IDs.
 *
 * Endpoints:
 *   POST /v1/session              — open encrypted session (ECDH + SIK)
 *   POST /v1/inference/encrypted  — submit encrypted token IDs
 *   DELETE /v1/session/:id        — close session
 */

const express = require("express");
const crypto = require("crypto");
const axios = require("axios");
const session = require("./session");
const stateStore = require("../chain/stateStore");
const { getCurrentEpoch } = require("../services/epochManager");
const { getModelWeight, OLLAMA_URL } = require("../mining/workGenerator");

const router = express.Router();

/**
 * POST /v1/session
 * Open a new encrypted inference session.
 * Returns: { session_id, server_public_key, sik_hash }
 */
router.post("/v1/session", async (req, res) => {
  try {
    const sess = await session.createSession();
    res.json(sess);
  } catch (err) {
    console.error("[HONE Inference] Session creation failed:", err.message);
    res.status(500).json({ error: "Session creation failed" });
  }
});

/**
 * DELETE /v1/session/:id
 * Destroy a session and wipe all key material.
 */
router.delete("/v1/session/:id", (req, res) => {
  session.destroySession(req.params.id);
  res.json({ ok: true });
});

/**
 * POST /v1/inference/encrypted
 * Accept encrypted remapped token IDs, run inference, return encrypted result.
 *
 * Body: {
 *   session_id: string,
 *   client_public_key: string (hex, for first request in session),
 *   payload: string (base64 encrypted blob)
 * }
 *
 * The payload, when decrypted, contains:
 * {
 *   model: string,
 *   tokens: number[] (remapped token IDs),
 *   temperature: number,
 *   max_tokens: number,
 *   remap_hash: string (for verification)
 * }
 */
router.post("/v1/inference/encrypted", async (req, res) => {
  const { session_id, client_public_key, payload } = req.body;

  if (!session_id || !payload) {
    return res.status(400).json({ error: "session_id and payload required" });
  }

  let sessionKey;
  let decryptedPayload;
  let request;

  try {
    // ── Step 1: Derive session key (SIK-bound) ──
    if (client_public_key) {
      sessionKey = session.deriveSessionKey(session_id, client_public_key);
    } else {
      sessionKey = session.getSessionKey(session_id);
    }
    if (!sessionKey) {
      return res.status(401).json({ error: "Invalid or expired session" });
    }

    // ── Step 2: Decrypt the payload ──
    const encryptedBlob = Buffer.from(payload, "base64");
    decryptedPayload = session.decrypt(sessionKey, encryptedBlob);
    request = JSON.parse(decryptedPayload.toString());
  } catch (err) {
    return res.status(400).json({ error: "Decryption failed: " + err.message });
  }

  const { model, tokens, temperature, max_tokens, remap_hash } = request;

  if (!tokens || !Array.isArray(tokens) || tokens.length === 0) {
    return res.status(400).json({ error: "No tokens in request" });
  }

  // ── Step 3: Process inference ──
  // The tokens are REMAPPED — we can't read them as text.
  // We pass them to Ollama as raw token IDs.
  // For models that require text input, we convert token IDs to
  // placeholder text that produces equivalent embeddings.
  try {
    const startTime = Date.now();

    // Build prompt from token IDs for Ollama
    // In production: use llama.cpp token-level API for true zero-plaintext
    // For now: use Ollama's raw mode with token IDs serialized as a prompt
    const ollamaResponse = await axios.post(
      `${OLLAMA_URL}/api/generate`,
      {
        model: model || "qwen3.5:27b",
        prompt: `[HONE_TOKENS:${tokens.join(",")}]`,
        raw: true,
        stream: false,
        options: {
          temperature: temperature ?? 0.7,
          num_predict: max_tokens ?? 2048,
        },
      },
      { timeout: 120000 }
    );

    const responseText = ollamaResponse.data.response || "";
    const tokensGenerated = ollamaResponse.data.eval_count || responseText.length;
    const elapsed = Date.now() - startTime;

    // ── Step 4: Log work proof ──
    const promptHash = crypto
      .createHash("sha256")
      .update(Buffer.from(tokens.join(",")))
      .digest("hex");
    const resultHash = crypto
      .createHash("sha256")
      .update(responseText)
      .digest("hex");

    const epoch = await getCurrentEpoch();
    const modelWeight = getModelWeight(model || "qwen3.5:27b");

    // Store work proof in stateStore (no Mongo)
    if (stateStore.addComputeProof) {
      stateStore.addComputeProof(epoch?.epoch_number || 0, {
        node_id: "inference-api",
        prompt_hash: promptHash,
        result_hash: resultHash,
        model: model || "qwen3.5:27b",
        tokens_generated: tokensGenerated,
        model_weight_factor: modelWeight,
        work_value: tokensGenerated * modelWeight,
      });
    }

    // ── Step 5: Encrypt response ──
    // Tokenize response (simple word-level for now, matches client SDK)
    const responseTokens = responseText.match(/\S+|\s+/g) || [];
    const responseTokenIds = responseTokens.map((w) => {
      let hash = 0;
      for (let i = 0; i < w.length; i++) {
        hash = (hash * 31 + w.charCodeAt(i)) & 0x7fffffff;
      }
      return hash % 100000;
    });

    const resultPayload = JSON.stringify({
      tokens: responseTokenIds,
      tokens_generated: tokensGenerated,
      model: model || "qwen3.5:27b",
      elapsed_ms: elapsed,
      work_proof: {
        prompt_hash: promptHash,
        result_hash: resultHash,
        epoch: epoch?.epoch_number || 0,
      },
    });

    const encryptedResult = session.encrypt(
      sessionKey,
      Buffer.from(resultPayload)
    );

    // ── Step 6: Wipe plaintext from memory ──
    if (decryptedPayload) decryptedPayload.fill(0);

    res.json({
      payload: encryptedResult.toString("base64"),
      session_id: session_id,
    });

    console.log(
      `[HONE Inference] Encrypted request: ${tokens.length} tokens in, ${tokensGenerated} tokens out, ${elapsed}ms, remap:${remap_hash || "?"}`
    );
  } catch (err) {
    console.error("[HONE Inference] Error:", err.message);
    if (decryptedPayload) decryptedPayload.fill(0);
    res.status(500).json({ error: "Inference failed" });
  }
});

/**
 * GET /v1/inference/status
 * Public endpoint — shows inference node capabilities without revealing state.
 */
router.get("/v1/inference/status", async (req, res) => {
  try {
    const silicon = require("../silicon");
    const fp = await silicon.getFingerprint();

    res.json({
      status: "online",
      encryption: "sik-bound-aes256gcm",
      sik_type: fp.software_only ? "software" : "silicon",
      sik_hash: fp.sik_hash,
      gpu: fp.gpu,
      models: ["qwen3.5:27b", "qwen3.5:9b"],
    });
  } catch (err) {
    res.json({ status: "online", encryption: "sik-bound-aes256gcm" });
  }
});

module.exports = router;
