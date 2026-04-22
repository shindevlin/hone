"use strict";

/**
 * Inference memory — per-account BTCPC-FS memory with Obsidian-style graph.
 * Shin Devlin
 *
 * After a job settles the miner calls extractAndSaveMemory() to:
 *   1. Ask the local Ollama model to summarize the exchange + extract tags
 *   2. Write the summary blob to BTCPC-FS (blobStore)
 *   3. Find similar existing memories by tag overlap (graph linking)
 *   4. Record INFERENCE_MEMORY_SAVE on chain
 *
 * Before a job runs, loadMemoriesForJob() fetches relevant blobs by CID and
 * returns a formatted context string injected into the system prompt.
 */

const fs = require("fs");
const path = require("path");
const crypto = require("crypto");
const axios = require("axios");
const stateStore = require("../chain/stateStore");
const ledger = require("./ledger");

const OLLAMA_URL = process.env.OLLAMA_URL || "http://localhost:11434";
const BLOB_DIR = process.env.BTCPC_BLOB_DIR || path.resolve(__dirname, "../../data/blobs");
const MEMORY_MODEL = process.env.BTCPC_MEMORY_MODEL || process.env.BTCPC_FAST_MODEL || "qwen3:0.6b";
const MAX_MEMORY_BYTES = 32 * 1024; // 32 KB per memory blob
const MAX_LINKED = 5;               // max backlinks per new memory
const MAX_TAG_OVERLAP_CANDIDATES = 20;

/** Write content string to BLOB_DIR, return sha256 hex CID. */
function _writeBlobSync(content) {
  const buf = Buffer.from(content, "utf8");
  if (buf.length > MAX_MEMORY_BYTES) return null;
  if (!fs.existsSync(BLOB_DIR)) fs.mkdirSync(BLOB_DIR, { recursive: true });
  const cid = crypto.createHash("sha256").update(buf).digest("hex");
  fs.writeFileSync(path.join(BLOB_DIR, cid), buf);
  return { cid, size: buf.length };
}

/** Read a blob by CID. Returns string or null. */
function _readBlobSync(cid) {
  if (!/^[a-f0-9]{64}$/.test(cid)) return null;
  try {
    return fs.readFileSync(path.join(BLOB_DIR, cid), "utf8");
  } catch (_) { return null; }
}

/** Ask Ollama for a JSON summary + tags array. Returns { summary, tags } or null. */
async function _extractSummaryAndTags(prompt, response, model) {
  const instruction = `You are a memory-extraction assistant. Given a conversation exchange, output a JSON object with exactly two fields:
- "summary": a 1-3 sentence summary of what was discussed or accomplished
- "tags": an array of 3-8 lowercase keyword tags (topics, entities, project names, technologies)

Conversation:
USER: ${prompt.slice(0, 1000)}
ASSISTANT: ${response.slice(0, 2000)}

Respond ONLY with valid JSON, no prose.`;

  try {
    const resp = await axios.post(`${OLLAMA_URL}/api/chat`, {
      model,
      messages: [{ role: "user", content: instruction }],
      stream: false,
      format: "json",
      options: { temperature: 0.1 },
    }, { timeout: 30000 });
    const content = resp.data?.message?.content || "";
    const parsed = JSON.parse(content);
    if (typeof parsed.summary !== "string" || !Array.isArray(parsed.tags)) return null;
    return {
      summary: parsed.summary.trim(),
      tags: parsed.tags.map(String).map(t => t.toLowerCase().trim()).filter(Boolean).slice(0, 8),
    };
  } catch (_) { return null; }
}

/**
 * Find up to MAX_LINKED existing memories for this account that share the most tags.
 * Returns array of memory_ids sorted by overlap score descending.
 */
function _findRelatedMemories(account, tags) {
  if (!tags || tags.length === 0) return [];
  const memories = stateStore.getUserMemories(account);
  const tagSet = new Set(tags);
  const scored = memories
    .slice(0, MAX_TAG_OVERLAP_CANDIDATES)
    .map(function(m) {
      const overlap = m.tags.filter(function(t) { return tagSet.has(t); }).length;
      return { memory_id: m.memory_id, overlap };
    })
    .filter(function(s) { return s.overlap > 0; })
    .sort(function(a, b) { return b.overlap - a.overlap; })
    .slice(0, MAX_LINKED);
  return scored.map(function(s) { return s.memory_id; });
}

/**
 * Post-settle hook called by the miner.
 * Summarizes the job exchange, writes a blob, links to related memories, records on chain.
 *
 * @param {string} account     - buyer's account name
 * @param {string} jobId       - inference job ID (for logging)
 * @param {string} prompt      - the original user prompt
 * @param {string} fullContent - the full model response
 * @param {string} projectId   - optional project grouping
 * @returns {string|null}      - memory_id if saved, null on failure
 */
async function extractAndSaveMemory(account, jobId, prompt, fullContent, projectId) {
  try {
    const extracted = await _extractSummaryAndTags(prompt, fullContent, MEMORY_MODEL);
    if (!extracted) return null;

    const blobContent = JSON.stringify({
      memory_version: 1,
      account,
      job_id: jobId,
      summary: extracted.summary,
      tags: extracted.tags,
      project_id: projectId || null,
      prompt_snippet: prompt.slice(0, 500),
    });

    const blobResult = _writeBlobSync(blobContent);
    if (!blobResult) return null;

    const linkedIds = _findRelatedMemories(account, extracted.tags);
    const memoryId = crypto.randomBytes(16).toString("hex");

    const epoch = await ledger.getCurrentEpoch();
    await ledger.recordMemorySave(
      account,
      memoryId,
      blobResult.cid,
      extracted.tags,
      projectId || null,
      linkedIds,
      blobResult.size,
      epoch
    );

    console.log(`[Memory] Saved memory ${memoryId} for ${account} (tags: ${extracted.tags.join(", ")}, linked: ${linkedIds.length})`);
    return memoryId;
  } catch (err) {
    console.warn(`[Memory] extractAndSaveMemory failed for ${account}: ${err.message}`);
    return null;
  }
}

/**
 * Load memory blobs by ID list (or all for an account if no list given).
 * Returns a formatted context string for injection into the system prompt.
 *
 * @param {string}   account    - buyer account
 * @param {string[]} memoryIds  - specific IDs; if empty, loads recent memories
 * @param {string}   projectId  - if set, filters to this project
 * @param {number}   limit      - max memories to load (default 5)
 */
function loadMemoriesForJob(account, memoryIds, projectId, limit) {
  limit = limit || 5;
  let candidates;

  if (Array.isArray(memoryIds) && memoryIds.length > 0) {
    candidates = memoryIds
      .map(function(id) { return stateStore.getMemory(account, id); })
      .filter(Boolean);
  } else {
    candidates = stateStore.getUserMemories(account);
    if (projectId) {
      candidates = candidates.filter(function(m) { return m.project_id === projectId; });
    }
    candidates = candidates.slice(0, limit);
  }

  if (candidates.length === 0) return null;

  const sections = [];
  for (const mem of candidates) {
    const blob = _readBlobSync(mem.cid);
    if (!blob) continue;
    try {
      const parsed = JSON.parse(blob);
      sections.push(`[Memory: ${parsed.tags ? parsed.tags.join(", ") : mem.memory_id}]\n${parsed.summary}`);
    } catch (_) {
      sections.push(`[Memory: ${mem.memory_id}]\n${blob.slice(0, 300)}`);
    }
  }

  if (sections.length === 0) return null;
  return `--- Relevant memories from previous sessions ---\n${sections.join("\n\n")}\n--- End memories ---`;
}

/**
 * Find memories semantically similar to a query string by tag overlap with probe tags.
 * Used by the /api/memory/search endpoint.
 */
async function searchMemories(account, query, limit) {
  limit = limit || 10;
  // Extract probe tags from query by calling the summarizer
  const extracted = await _extractSummaryAndTags(query, "", MEMORY_MODEL).catch(() => null);
  const probeTags = extracted ? extracted.tags : query.toLowerCase().split(/\s+/).slice(0, 5);
  const probeSet = new Set(probeTags);

  const memories = stateStore.getUserMemories(account);
  return memories
    .map(function(m) {
      const overlap = m.tags.filter(function(t) { return probeSet.has(t); }).length;
      return { ...m, _score: overlap };
    })
    .filter(function(m) { return m._score > 0; })
    .sort(function(a, b) { return b._score - a._score; })
    .slice(0, limit)
    .map(function(m) { const { _score, ...rest } = m; return rest; });
}

module.exports = { extractAndSaveMemory, loadMemoriesForJob, searchMemories };
