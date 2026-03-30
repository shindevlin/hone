"use strict";

/**
 * BTCPC Tokenizer
 * Shin Devlin
 *
 * Loads HuggingFace-format tokenizer.json files for real BPE tokenization.
 * Used by the client SDK to tokenize prompts locally before encryption.
 *
 * Supports: qwen, llama, mistral, gemma (any HuggingFace BPE tokenizer)
 *
 * Usage:
 *   const tokenizer = require('./tokenizer');
 *   await tokenizer.load('qwen3.5');
 *   const tokens = tokenizer.encode('Hello world');
 *   const text = tokenizer.decode(tokens);
 */

const fs = require("fs");
const path = require("path");

const TOKENIZER_DIR = path.resolve(__dirname, "../../data/tokenizers");

// Cached tokenizers
const loaded = {};

/**
 * Load a tokenizer from data/tokenizers/{model}/tokenizer.json
 * Downloads from HuggingFace if not cached locally.
 */
async function load(model) {
  const key = model.replace(/[:/]/g, "-");
  if (loaded[key]) return loaded[key];

  const tokenizerPath = path.join(TOKENIZER_DIR, key, "tokenizer.json");

  if (!fs.existsSync(tokenizerPath)) {
    // Try to download from HuggingFace
    await downloadTokenizer(model, key);
  }

  if (!fs.existsSync(tokenizerPath)) {
    console.warn(`[BTCPC Tokenizer] No tokenizer found for ${model}, using byte-level fallback`);
    loaded[key] = createByteFallback();
    return loaded[key];
  }

  const data = JSON.parse(fs.readFileSync(tokenizerPath, "utf8"));
  const tokenizer = parseBPETokenizer(data);
  loaded[key] = tokenizer;
  console.log(`[BTCPC Tokenizer] Loaded ${model}: ${tokenizer.vocabSize} tokens`);
  return tokenizer;
}

/**
 * Download tokenizer.json from HuggingFace.
 */
async function downloadTokenizer(model, key) {
  const axios = require("axios");
  const dir = path.join(TOKENIZER_DIR, key);

  // Map model names to HuggingFace repos
  const hfRepos = {
    "qwen3.5:27b": "Qwen/Qwen3-27B",
    "qwen3.5:9b": "Qwen/Qwen3-9B",
    "qwen3.5": "Qwen/Qwen3-27B",
    "deepseek-r1:8b": "deepseek-ai/DeepSeek-R1-Distill-Qwen-8B",
    "llama3.1:8b": "meta-llama/Llama-3.1-8B",
    "gemma2:9b": "google/gemma-2-9b",
  };

  const repo = hfRepos[model] || hfRepos[model.split(":")[0]];
  if (!repo) {
    console.warn(`[BTCPC Tokenizer] No HuggingFace repo mapped for ${model}`);
    return;
  }

  const url = `https://huggingface.co/${repo}/resolve/main/tokenizer.json`;
  console.log(`[BTCPC Tokenizer] Downloading ${url}...`);

  try {
    fs.mkdirSync(dir, { recursive: true });
    const res = await axios.get(url, { timeout: 30000, responseType: "arraybuffer" });
    fs.writeFileSync(path.join(dir, "tokenizer.json"), res.data);
    console.log(`[BTCPC Tokenizer] Saved to ${dir}/tokenizer.json`);
  } catch (err) {
    console.warn(`[BTCPC Tokenizer] Download failed: ${err.message}`);
  }
}

/**
 * Parse a HuggingFace tokenizer.json into an encoder/decoder.
 * Supports BPE (byte-level) format used by GPT, Qwen, LLaMA, etc.
 */
function parseBPETokenizer(data) {
  const vocab = {};
  const reverseVocab = {};

  // Extract vocabulary from the tokenizer model
  const model = data.model || {};
  const vocabData = model.vocab || {};

  for (const [token, id] of Object.entries(vocabData)) {
    vocab[token] = id;
    reverseVocab[id] = token;
  }

  // Also check for added_tokens
  if (data.added_tokens) {
    for (const at of data.added_tokens) {
      vocab[at.content] = at.id;
      reverseVocab[at.id] = at.content;
    }
  }

  const vocabSize = Object.keys(vocab).length;

  // BPE merges
  const merges = (model.merges || []).map((m) => {
    const parts = m.split(" ");
    return [parts[0], parts[1]];
  });

  return {
    vocabSize,
    encode: (text) => bpeEncode(text, vocab, merges),
    decode: (tokens) => bpeDecode(tokens, reverseVocab),
  };
}

/**
 * BPE encode: text → token IDs
 */
function bpeEncode(text, vocab, merges) {
  if (!text) return [];

  // Convert text to byte-level representation
  const bytes = Buffer.from(text, "utf8");
  let symbols = [];
  for (const b of bytes) {
    const ch = byteToChar(b);
    symbols.push(ch);
  }

  // Apply BPE merges
  for (const [a, b] of merges) {
    const merged = a + b;
    let i = 0;
    const newSymbols = [];
    while (i < symbols.length) {
      if (i < symbols.length - 1 && symbols[i] === a && symbols[i + 1] === b) {
        newSymbols.push(merged);
        i += 2;
      } else {
        newSymbols.push(symbols[i]);
        i++;
      }
    }
    symbols = newSymbols;
  }

  // Look up token IDs
  const ids = [];
  for (const sym of symbols) {
    if (vocab[sym] !== undefined) {
      ids.push(vocab[sym]);
    } else {
      // Unknown token — encode each byte individually
      const symBytes = Buffer.from(sym, "utf8");
      for (const b of symBytes) {
        const ch = byteToChar(b);
        if (vocab[ch] !== undefined) {
          ids.push(vocab[ch]);
        }
      }
    }
  }

  return ids;
}

/**
 * BPE decode: token IDs → text
 */
function bpeDecode(tokens, reverseVocab) {
  const pieces = [];
  for (const id of tokens) {
    const piece = reverseVocab[id];
    if (piece !== undefined) {
      pieces.push(piece);
    }
  }

  // Convert byte-level chars back to actual bytes
  const bytes = [];
  for (const piece of pieces) {
    for (const ch of piece) {
      const b = charToByte(ch);
      if (b !== null) {
        bytes.push(b);
      }
    }
  }

  return Buffer.from(bytes).toString("utf8");
}

/**
 * Byte-level BPE character mapping (GPT-2 / Qwen style).
 * Maps bytes 0-255 to unicode characters to avoid control chars.
 */
const BYTE_TO_CHAR = {};
const CHAR_TO_BYTE = {};

(function initByteMap() {
  let n = 0;
  for (let b = 0; b < 256; b++) {
    if (
      (b >= 33 && b <= 126) || // printable ASCII
      (b >= 161 && b <= 172) ||
      (b >= 174 && b <= 255)
    ) {
      BYTE_TO_CHAR[b] = String.fromCharCode(b);
      CHAR_TO_BYTE[String.fromCharCode(b)] = b;
    } else {
      BYTE_TO_CHAR[b] = String.fromCharCode(256 + n);
      CHAR_TO_BYTE[String.fromCharCode(256 + n)] = b;
      n++;
    }
  }
})();

function byteToChar(b) {
  return BYTE_TO_CHAR[b] || String.fromCharCode(b);
}

function charToByte(ch) {
  const b = CHAR_TO_BYTE[ch];
  return b !== undefined ? b : null;
}

/**
 * Byte-level fallback tokenizer (no vocabulary needed).
 * Each byte becomes a token. Works with any model but produces longer sequences.
 */
function createByteFallback() {
  return {
    vocabSize: 256,
    encode: (text) => Array.from(Buffer.from(text, "utf8")),
    decode: (tokens) => Buffer.from(tokens).toString("utf8"),
  };
}

module.exports = { load };
