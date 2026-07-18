"use strict";

const fs = require("fs");
const path = require("path");

function getCrossChainDir() {
  return path.resolve(process.env.HONE_DATA_DIR || path.resolve(__dirname, "../../data"), "anchors/cross-chain");
}

function _readJson(filePath) {
  try {
    if (!fs.existsSync(filePath)) return null;
    return JSON.parse(fs.readFileSync(filePath, "utf8"));
  } catch (_) {
    return null;
  }
}

function _listEpochFiles(dirPath) {
  if (!fs.existsSync(dirPath)) return [];
  return fs.readdirSync(dirPath)
    .filter((name) => /^finality-\d{8}\.json$/.test(name))
    .sort();
}

function _latestFilePath(dirPath) {
  const files = _listEpochFiles(dirPath);
  if (!files.length) return null;
  return path.join(dirPath, files[files.length - 1]);
}

function _normalizeAnnouncement(payload, chainId) {
  if (!payload) return null;
  const chain = chainId || payload.target_chain || null;
  const cutoffEpoch = Number(payload.cutoff_epoch || payload.finality_epoch || 0) || 0;
  const finalizedJobs = Array.isArray(payload.finalized_jobs) ? payload.finalized_jobs.slice() : [];
  return {
    target_chain: chain,
    finality_epoch: Number(payload.finality_epoch || 0) || 0,
    cutoff_epoch: cutoffEpoch,
    state_root: payload.state_root || null,
    finality_hash: payload.finality_hash || null,
    inference_finality_hash: payload.inference_finality_hash || null,
    finalized_job_count: Number(payload.finalized_job_count || finalizedJobs.length || 0) || 0,
    finalized_jobs: finalizedJobs,
    announcement_hash: payload.announcement_hash || null,
    target_address: payload.target_address || null,
    anchored_at: payload.anchored_at || null,
  };
}

function loadLatestAnnouncement(chainId) {
  if (!chainId) return null;

  const crossChainDir = getCrossChainDir();
  const chainDir = path.join(crossChainDir, chainId);
  const chainFile = _latestFilePath(chainDir);
  if (chainFile) {
    return _normalizeAnnouncement(_readJson(chainFile), chainId);
  }

  const summaryFile = _latestFilePath(crossChainDir);
  if (!summaryFile) return null;
  const summary = _readJson(summaryFile);
  if (!summary) return null;
  if (Array.isArray(summary.targets)) {
    const target = summary.targets.find((entry) => entry && entry.chain_id === chainId);
    if (target) return _normalizeAnnouncement(target, chainId);
  }
  return null;
}

function getFinalityCutoff(chainId) {
  const announcement = loadLatestAnnouncement(chainId);
  return announcement ? announcement.cutoff_epoch : null;
}

function isEpochFinalized(chainId, epochNumber) {
  const cutoff = getFinalityCutoff(chainId);
  if (cutoff === null) return false;
  return Number(epochNumber) <= cutoff;
}

function assertEpochFinalized(chainId, epochNumber) {
  const cutoff = getFinalityCutoff(chainId);
  if (cutoff === null) {
    throw new Error("no finality announcement available for " + chainId);
  }
  const epoch = Number(epochNumber) || 0;
  if (epoch > cutoff) {
    throw new Error("epoch " + epoch + " exceeds finality cutoff " + cutoff + " for " + chainId);
  }
  return true;
}

module.exports = {
  getCrossChainDir,
  loadLatestAnnouncement,
  getFinalityCutoff,
  isEpochFinalized,
  assertEpochFinalized,
};
