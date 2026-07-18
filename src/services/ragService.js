"use strict";

/**
 * Retrieval-Augmented Generation (RAG) service.
 * Shin Devlin
 *
 * Fetches HONE-FS blobs by CID, chunks the text content, generates
 * embeddings via Ollama's /api/embed endpoint, and returns the top-k
 * most relevant chunks for a given query. No external vector DB required —
 * embeddings are computed per-job at inference time.
 *
 * Flow: job.rag_cids → fetch blobs → chunk → embed → cosine rank → inject context
 */

const axios = require("axios");
const path = require("path");
const fs = require("fs");

const OLLAMA_URL = process.env.OLLAMA_URL || "http://localhost:11434";
const EMBED_MODEL = process.env.HONE_EMBED_MODEL || "nomic-embed-text";
const CHUNK_SIZE = 512;      // characters per chunk
const CHUNK_OVERLAP = 64;    // character overlap between chunks
const DEFAULT_TOP_K = 5;
const MAX_CONTEXT_CHARS = 6000;
const BLOB_DATA_DIR = process.env.HONE_BLOB_DIR || path.resolve(__dirname, "../../data/blobs");

// ── Text chunking ─────────────────────────────────────────────────────────────

function chunkText(text, size, overlap) {
  size = size || CHUNK_SIZE;
  overlap = overlap || CHUNK_OVERLAP;
  const chunks = [];
  let start = 0;
  while (start < text.length) {
    chunks.push(text.slice(start, start + size));
    start += size - overlap;
    if (start + overlap >= text.length) break;
  }
  if (start < text.length) chunks.push(text.slice(start));
  return chunks;
}

// ── Cosine similarity ─────────────────────────────────────────────────────────

function cosineSim(a, b) {
  let dot = 0, magA = 0, magB = 0;
  for (let i = 0; i < a.length; i++) {
    dot += a[i] * b[i];
    magA += a[i] * a[i];
    magB += b[i] * b[i];
  }
  const denom = Math.sqrt(magA) * Math.sqrt(magB);
  return denom === 0 ? 0 : dot / denom;
}

// ── Ollama embedding ──────────────────────────────────────────────────────────

async function embedText(texts, model) {
  model = model || EMBED_MODEL;
  // Ollama /api/embed accepts input as array or string
  const resp = await axios.post(`${OLLAMA_URL}/api/embed`, {
    model,
    input: texts,
  }, { timeout: 30000 });
  // Returns { embeddings: [[...], [...]] } or { embedding: [...] }
  if (resp.data && resp.data.embeddings) return resp.data.embeddings;
  if (resp.data && resp.data.embedding) return [resp.data.embedding];
  throw new Error("Unexpected embedding response from Ollama");
}

// ── Blob loading ──────────────────────────────────────────────────────────────

async function loadBlobContent(cid) {
  // CID is a 64-char hex sha256. Blob stored at data/blobs/<cid>
  if (!/^[a-f0-9]{64}$/.test(cid)) throw new Error(`Invalid CID: ${cid}`);
  const blobPath = path.join(BLOB_DATA_DIR, cid);
  if (!fs.existsSync(blobPath)) {
    throw new Error(`Blob not found locally: ${cid}`);
  }
  return fs.readFileSync(blobPath, "utf8");
}

// ── Main RAG function ─────────────────────────────────────────────────────────

/**
 * Retrieve relevant context chunks from a set of HONE-FS blobs.
 *
 * @param {string} query - The user's query / prompt
 * @param {string[]} cids - Array of blob CIDs to search over
 * @param {object} opts - { topK, embedModel, chunkSize, chunkOverlap, maxContextChars }
 * @returns {string} Formatted context string to inject into system prompt
 */
async function retrieveContext(query, cids, opts) {
  opts = opts || {};
  if (!cids || cids.length === 0) return "";

  const topK = opts.topK || DEFAULT_TOP_K;
  const model = opts.embedModel || EMBED_MODEL;
  const chunkSize = opts.chunkSize || CHUNK_SIZE;
  const overlap = opts.chunkOverlap || CHUNK_OVERLAP;
  const maxChars = opts.maxContextChars || MAX_CONTEXT_CHARS;

  // 1. Load and chunk all blobs
  const allChunks = [];
  for (const cid of cids) {
    try {
      const text = await loadBlobContent(cid);
      const chunks = chunkText(text, chunkSize, overlap);
      for (const chunk of chunks) {
        allChunks.push({ cid, text: chunk });
      }
    } catch (err) {
      console.warn(`[RAG] Failed to load blob ${cid}: ${err.message}`);
    }
  }

  if (allChunks.length === 0) return "";

  // 2. Embed query + all chunks in one batch request
  const allTexts = [query, ...allChunks.map((c) => c.text)];
  let embeddings;
  try {
    embeddings = await embedText(allTexts, model);
  } catch (err) {
    console.warn(`[RAG] Embedding failed: ${err.message} — falling back to keyword injection`);
    // Fallback: just prepend the first maxChars chars of all blobs
    const fallback = allChunks.map((c) => c.text).join("\n\n").slice(0, maxChars);
    return `[Retrieved context]\n${fallback}`;
  }

  const queryEmbedding = embeddings[0];
  const chunkEmbeddings = embeddings.slice(1);

  // 3. Rank by cosine similarity
  const scored = allChunks.map((chunk, i) => ({
    ...chunk,
    score: cosineSim(queryEmbedding, chunkEmbeddings[i]),
  }));
  scored.sort((a, b) => b.score - a.score);

  // 4. Take top-k, deduplicate, build context string
  const seen = new Set();
  const topChunks = [];
  let totalChars = 0;

  for (const chunk of scored.slice(0, topK * 3)) {
    if (totalChars >= maxChars) break;
    const key = chunk.text.slice(0, 50);
    if (seen.has(key)) continue;
    seen.add(key);
    topChunks.push(chunk);
    totalChars += chunk.text.length;
    if (topChunks.length >= topK) break;
  }

  if (topChunks.length === 0) return "";

  const contextLines = topChunks.map((c, i) =>
    `[Source ${i + 1} — ${c.cid.slice(0, 8)}... (relevance: ${(c.score * 100).toFixed(0)}%)]\n${c.text}`
  );

  return `[Retrieved context — ${topChunks.length} passages from ${cids.length} document(s)]\n\n` +
    contextLines.join("\n\n---\n\n");
}

module.exports = {
  retrieveContext,
  chunkText,
  cosineSim,
  embedText,
};
