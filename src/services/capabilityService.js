"use strict";

/**
 * Miner capability detection and job requirement matching.
 * Shin Devlin
 *
 * detectCapabilities() interrogates the local environment and returns a
 * capabilities object that is broadcast on-chain as MINER_CAPABILITY.
 *
 * meetsJobRequirements(job, caps) lets the miner skip jobs it cannot serve
 * before claiming them, avoiding wasted TTL for the buyer.
 *
 * getJobRequirements(job) extracts what a job needs from its fields, so
 * any caller (miner, buyer, explorer) can reason about job routability.
 */

const { execSync } = require("child_process");
const { version } = require("../../package.json");

const OLLAMA_URL = process.env.OLLAMA_URL || "http://localhost:11434";

// ── Capability detection ───────────────────────────────────────────────────────

function _tryExec(cmd) {
  try { execSync(cmd, { stdio: "pipe", timeout: 4000 }); return true; } catch (_) { return false; }
}

/** Fetch local Ollama model list. Returns [] on any error. */
async function _getOllamaModels() {
  try {
    const axios = require("axios");
    const resp = await axios.get(`${OLLAMA_URL}/api/tags`, { timeout: 5000 });
    return (resp.data && Array.isArray(resp.data.models))
      ? resp.data.models.map((m) => m.name)
      : [];
  } catch (_) {
    return [];
  }
}

const VISION_MODEL_PATTERNS = [
  /llava/i, /bakllava/i, /moondream/i, /minicpm-v/i, /qwen.*vl/i,
  /gemma.*vision/i, /phi.*vision/i, /cogvlm/i, /internvl/i, /deepseek.*vl/i,
];

function _isVisionModel(name) {
  return VISION_MODEL_PATTERNS.some((re) => re.test(name));
}

const REASONING_MODEL_PATTERNS = [/qwq/i, /deepseek.*r1/i, /o1/i, /r1/i, /qwen.*think/i, /qwq/i];
const FAST_MODEL_PATTERNS = [/0\.6b/i, /1b/i, /phi3.*mini/i, /phi-3.*mini/i, /gemma.*2b/i, /smollm/i, /tinyllama/i];

function _isReasoningModel(name) { return REASONING_MODEL_PATTERNS.some((re) => re.test(name)); }
function _isFastModel(name) { return FAST_MODEL_PATTERNS.some((re) => re.test(name)); }

function _isWhisperAvailable() {
  return _tryExec("whisper --help") || _tryExec("python3 -m whisper --help");
}

function _isPlaywrightAvailable() {
  try { require("playwright"); return true; } catch (_) { return false; }
}

/**
 * Detect what this miner can serve.
 * Async because it queries Ollama for the local model list.
 */
async function detectCapabilities() {
  const models = await _getOllamaModels();

  // Vision: explicit env var takes precedence, then scan model names
  const visionModelEnv = process.env.BTCPC_VISION_MODEL;
  const visionFromEnv = visionModelEnv && models.includes(visionModelEnv);
  const visionFromScan = models.some(_isVisionModel);
  const vision = !!(visionFromEnv || visionFromScan);
  const visionModel = vision
    ? (visionFromEnv ? visionModelEnv : models.find(_isVisionModel))
    : null;

  // Tiers
  const tiers = ["standard"];
  if (models.some(_isReasoningModel) || process.env.BTCPC_REASONING_MODEL) {
    tiers.push("reasoning");
  }
  if (models.some(_isFastModel) || process.env.BTCPC_FAST_MODEL) {
    tiers.push("fast");
  }

  return {
    models,
    vision,
    vision_model: visionModel || null,
    audio: _isWhisperAvailable(),
    code_exec: process.env.BTCPC_CODE_EXEC_ENABLED === "true",
    browser: process.env.BTCPC_BROWSER_ENABLED !== "false" && _isPlaywrightAvailable(),
    finetune: process.env.BTCPC_FINETUNE_ENABLED === "true",
    tiers,
    version,
  };
}

// ── Job requirement extraction ─────────────────────────────────────────────────

/**
 * Extract what capabilities a job requires from its fields.
 * Returns a requirements object — falsy fields mean "no requirement".
 */
function getJobRequirements(job) {
  const req = {};

  if (job.image_cids && job.image_cids.length > 0) req.vision = true;
  if (job.audio_cid) req.audio = true;

  // Tool requirements
  if (Array.isArray(job.tools)) {
    if (job.tools.some((t) => t === "execute_code" || (t && t.name === "execute_code"))) {
      req.code_exec = true;
    }
  }

  if (job.tier && job.tier !== "standard") req.tier = job.tier;

  return req;
}

/**
 * Returns true if the given capabilities satisfy the job's requirements.
 */
function meetsJobRequirements(job, caps) {
  if (!caps) return false;
  const req = getJobRequirements(job);

  if (req.vision && !caps.vision) return false;
  if (req.audio && !caps.audio) return false;
  if (req.code_exec && !caps.code_exec) return false;
  if (req.tier && !caps.tiers.includes(req.tier)) return false;

  return true;
}

module.exports = { detectCapabilities, getJobRequirements, meetsJobRequirements };
