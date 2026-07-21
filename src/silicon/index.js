"use strict";

/**
 * HONE Silicon Identity Key (SIK) — Node.js interface
 * Shin Devlin
 *
 * Wraps the CUDA fingerprint probe binary. Provides:
 *   - getFingerprint()  — derive SIK from physical GPU
 *   - verify(hash)      — check if this GPU matches a registered hash
 *   - getSikHash()      — get the on-chain hash (SIK never leaves the machine)
 */

const { execFile } = require("child_process");
const path = require("path");
const crypto = require("crypto");

const BINARY_PATH = path.resolve(__dirname, "hone-sik");

/**
 * Run the SIK probe and return { sik, sik_hash, gpu, vram_mb, timing_stable, fp_stable }
 * The SIK itself should NEVER be transmitted — only sik_hash goes on-chain.
 */
function getFingerprint() {
  return new Promise((resolve, reject) => {
    execFile(BINARY_PATH, ["--json"], { timeout: 30000 }, (err, stdout, stderr) => {
      if (err) {
        // If binary not compiled, fall back to software fingerprint
        if (err.code === "ENOENT") {
          console.warn("[HONE SIK] Binary not found. Run: cd src/silicon && nvcc -O2 -o hone-sik fingerprint.cu");
          return resolve(softwareFingerprint());
        }
        return reject(new Error("SIK probe failed: " + (err.message || stderr)));
      }
      try {
        const result = JSON.parse(stdout.trim());
        resolve(result);
      } catch (e) {
        reject(new Error("SIK probe returned invalid JSON: " + stdout));
      }
    });
  });
}

/**
 * Verify that this GPU matches a registered SIK hash.
 */
function verify(expectedHash) {
  return new Promise((resolve, reject) => {
    execFile(BINARY_PATH, ["--verify", expectedHash], { timeout: 30000 }, (err, stdout) => {
      if (err && err.code === "ENOENT") {
        return resolve({ match: false, reason: "binary_not_found" });
      }
      try {
        const result = JSON.parse(stdout.trim());
        resolve(result);
      } catch (e) {
        resolve({ match: false, reason: "parse_error" });
      }
    });
  });
}

/**
 * Get just the SIK hash (for node registration).
 */
async function getSikHash() {
  const fp = await getFingerprint();
  return fp.sik_hash;
}

/**
 * Derive a session encryption key from SIK + request context.
 * Used to decrypt inference prompts. Key exists only in memory, briefly.
 */
async function deriveSessionKey(requestId) {
  const fp = await getFingerprint();
  const key = crypto.createHash("sha256")
    .update(fp.sik)
    .update(requestId)
    .update(Date.now().toString())
    .digest();
  // Zero the SIK from the fingerprint object
  fp.sik = "0".repeat(64);
  return key;
}

/**
 * Software-only fingerprint fallback for machines without CUDA.
 * Uses CPU serial, hostname, and OS entropy. NOT silicon-bound.
 * Marked as software_only so the network can distinguish.
 */
function softwareFingerprint() {
  const os = require("os");
  const data = [
    os.hostname(),
    os.cpus()[0]?.model || "",
    os.totalmem().toString(),
    os.platform(),
    os.arch(),
  ].join("|");

  const sik = crypto.createHash("sha256").update("hone-software-sik:" + data).digest("hex");
  const sik_hash = crypto.createHash("sha256").update(Buffer.from(sik, "hex")).digest("hex");

  return {
    sik,
    sik_hash,
    gpu: "none (software fallback)",
    vram_mb: 0,
    timing_stable: 0,
    fp_stable: 0,
    software_only: true,
  };
}

module.exports = {
  getFingerprint,
  verify,
  getSikHash,
  deriveSessionKey,
};
