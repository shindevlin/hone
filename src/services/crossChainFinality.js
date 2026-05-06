"use strict";

const fs = require("fs");
const path = require("path");
const crypto = require("crypto");
const bridgeRegistry = require("./bridgeRegistry");
const ledger = require("./ledger");
const stateStore = require("../chain/stateStore");

const lastAnnouncedEpochByTarget = new Map();

function getCrossChainDir() {
  return path.resolve(process.env.BTCPC_DATA_DIR || path.resolve(__dirname, "../../data"), "anchors/cross-chain");
}

function _round(n) {
  return parseFloat(Number(n).toFixed(10));
}

function _ensureDir(dirPath) {
  if (!fs.existsSync(dirPath)) {
    fs.mkdirSync(dirPath, { recursive: true });
  }
}

function _writeAtomic(filePath, content) {
  const tmpPath = filePath + ".tmp";
  fs.writeFileSync(tmpPath, content);
  fs.renameSync(tmpPath, filePath);
}

function _sha256(data) {
  return crypto.createHash("sha256").update(JSON.stringify(data), "utf8").digest("hex");
}

function _collectFinalizedJobs(epochNumber, lastEpoch) {
  const jobs = stateStore.getRecentInferenceFinality ? stateStore.getRecentInferenceFinality(1000) : [];
  return jobs
    .filter((job) => Number(job.finality_epoch || 0) > (lastEpoch || 0) && Number(job.finality_epoch || 0) <= epochNumber)
    .sort((a, b) => {
      if ((a.finality_epoch || 0) !== (b.finality_epoch || 0)) return (a.finality_epoch || 0) - (b.finality_epoch || 0);
      return String(a.job_id || "").localeCompare(String(b.job_id || ""));
    })
    .map((job) => ({
      job_id: job.job_id,
      finality_epoch: job.finality_epoch || 0,
      finality_hash: job.finality_hash || null,
      finality_outcome: job.finality_outcome || null,
      review_stage: job.review_stage || null,
      challenge_status: job.challenge_status || null,
      review_winner: job.review_winner || null,
      review_dissenters: Array.isArray(job.review_dissenters) ? job.review_dissenters.slice() : [],
      review_vote_count: job.review_vote_count || 0,
    }));
}

function buildAnnouncement(epochNumber, snapshot, anchorResult) {
  const epoch = Number(epochNumber) || 0;
  const stateRoot = anchorResult && anchorResult.state_root
    ? anchorResult.state_root
    : crypto.createHash("sha256").update(JSON.stringify(snapshot), "utf8").digest("hex");
  const targets = bridgeRegistry.getAllChains().map((chain) => ({
    chain_id: chain.chain_id,
    name: chain.name,
    bridge_reserve_address: chain.bridge_reserve_address,
  }));

  const finalizedJobs = _collectFinalizedJobs(epoch, -1);
  const inferenceFinalityHash = _sha256(finalizedJobs);
  const announcementHash = _sha256({
    epoch,
    state_root: stateRoot,
    inference_finality_hash: inferenceFinalityHash,
    finalized_job_count: finalizedJobs.length,
    targets: targets.map((t) => t.chain_id),
  });

  return {
    finality_epoch: epoch,
    state_root: stateRoot,
    finality_hash: anchorResult && anchorResult.finality_hash ? anchorResult.finality_hash : stateRoot,
    inference_finality_hash: inferenceFinalityHash,
    finalized_job_count: finalizedJobs.length,
    finalized_jobs: finalizedJobs,
    targets,
    announcement_hash: announcementHash,
    anchored_at: Date.now(),
  };
}

async function announce(epochNumber, snapshot, anchorResult) {
  const epoch = Number(epochNumber) || 0;
  const targets = bridgeRegistry.getAllChains();
  if (!targets.length) {
    return { skipped: true, reason: "no bridge chains registered", epoch };
  }

  const crossChainDir = getCrossChainDir();
  _ensureDir(crossChainDir);
  const announcement = buildAnnouncement(epoch, snapshot, anchorResult);
  const results = [];

  for (const target of targets) {
    const lastEpoch = lastAnnouncedEpochByTarget.get(target.chain_id) || 0;
    const jobs = _collectFinalizedJobs(epoch, lastEpoch);
    if (jobs.length === 0 && lastEpoch >= epoch) {
      continue;
    }

    const targetAnnouncement = {
      target_chain: target.chain_id,
      target_name: target.name,
      target_address: target.bridge_reserve_address,
      finality_epoch: announcement.finality_epoch,
      cutoff_epoch: announcement.finality_epoch,
      state_root: announcement.state_root,
      finality_hash: announcement.finality_hash,
      inference_finality_hash: _sha256(jobs),
      finalized_job_count: jobs.length,
      finalized_jobs: jobs,
      announcement_hash: _sha256({
        target_chain: target.chain_id,
        finality_epoch: announcement.finality_epoch,
        state_root: announcement.state_root,
        inference_finality_hash: _sha256(jobs),
        finalized_job_count: jobs.length,
      }),
      anchored_at: announcement.anchored_at,
    };

    const chainDir = path.join(crossChainDir, target.chain_id);
    _ensureDir(chainDir);
    const fileName = "finality-" + String(epoch).padStart(8, "0") + ".json";
    _writeAtomic(path.join(chainDir, fileName), JSON.stringify(targetAnnouncement, null, 2));

    await ledger.recordCrossChainFinalityAnnouncement(target.chain_id, {
      state_root: targetAnnouncement.state_root,
      finality_hash: targetAnnouncement.finality_hash,
      announcement_hash: targetAnnouncement.announcement_hash,
      inference_finality_hash: targetAnnouncement.inference_finality_hash,
      finalized_job_count: targetAnnouncement.finalized_job_count,
      cutoff_epoch: targetAnnouncement.cutoff_epoch,
      finalized_jobs: targetAnnouncement.finalized_jobs,
      target_address: targetAnnouncement.target_address,
      anchored_at: targetAnnouncement.anchored_at,
    }, epoch);

    lastAnnouncedEpochByTarget.set(target.chain_id, epoch);
    results.push(targetAnnouncement);
  }

  const summaryPath = path.join(crossChainDir, "finality-" + String(epoch).padStart(8, "0") + ".json");
  _writeAtomic(summaryPath, JSON.stringify(announcement, null, 2));

  return {
    skipped: false,
    epoch,
    announcement_hash: announcement.announcement_hash,
    finalized_job_count: announcement.finalized_job_count,
    targets: results,
  };
}

function resetForTests() {
  lastAnnouncedEpochByTarget.clear();
}

module.exports = {
  buildAnnouncement,
  announce,
  resetForTests,
  getCrossChainDir,
};
