"use strict";

/**
 * HONE-FS Blob Payouts — v2.11.1
 * Shin Devlin
 *
 * Computes pro-rata payouts for storage hosts based on bytes served,
 * routes the protocol fee share to hone_recycle (No Burn, All Recycle),
 * and hands the results to ledger.recordTransfer to actually move HONE
 * from the uploader's escrow to the hosts.
 *
 * Called at epoch boundaries (or on-demand via a settlement route) to
 * distribute the accumulated payment_hone pool for a given CID across
 * its hosts according to bytes_served_by_host.
 *
 * Split (from docs/TOKENOMICS.md §5):
 *   90% → serving hosts (pro-rata by bytes_served_by_host)
 *    9% → hone_recycle
 *    1% → hone_reputation_pool
 *
 * If no bytes have been served, the payment stays in escrow — hosts
 * can only claim what they've actually delivered.
 */

var HOST_SHARE = 0.90;
var RECYCLE_SHARE = 0.09;
var REPUTATION_SHARE = 0.01;

/**
 * Compute payout amounts for a blob given its current chain state.
 * Pure function — no side effects, no chain writes. Callers decide
 * whether and when to actually record the transfers.
 *
 * Returns {
 *   cid,
 *   total_pool: number,          // unchanged from commit.payment_hone
 *   host_pool: number,           // 90% of total
 *   recycle_pool: number,        // 9% of total
 *   reputation_pool: number,     // 1% of total
 *   total_bytes_served: number,  // sum of bytes_served_by_host
 *   host_payouts: [              // pro-rata slice of host_pool
 *     { host, bytes_served, amount_hone }
 *   ]
 * }
 *
 * If total_bytes_served is 0, all host_payouts are 0 and the host_pool
 * is NOT redistributed to recycle — it remains in the blob's payment
 * pool and carries forward to the next settlement window. Honest hosts
 * can only earn what they've actually served.
 */
function computePayouts(blobCommit) {
  if (!blobCommit) {
    return null;
  }
  var totalPool = Number(blobCommit.payment_hone) || 0;
  var hostPool = _round(totalPool * HOST_SHARE);
  var recyclePool = _round(totalPool * RECYCLE_SHARE);
  var reputationPool = _round(totalPool * REPUTATION_SHARE);

  var servedByHost = blobCommit.bytes_served_by_host || {};
  var totalBytes = 0;
  var hostKeys = Object.keys(servedByHost);
  for (var i = 0; i < hostKeys.length; i++) {
    totalBytes += Number(servedByHost[hostKeys[i]]) || 0;
  }

  var hostPayouts = hostKeys.map(function (host) {
    var bytes = Number(servedByHost[host]) || 0;
    var amount = totalBytes > 0 ? _round(hostPool * (bytes / totalBytes)) : 0;
    return { host: host, bytes_served: bytes, amount_hone: amount };
  });

  return {
    cid: blobCommit.cid,
    total_pool: totalPool,
    host_pool: hostPool,
    recycle_pool: recyclePool,
    reputation_pool: reputationPool,
    total_bytes_served: totalBytes,
    host_payouts: hostPayouts,
  };
}

/**
 * Settle a blob's payout by actually recording the HONE transfers on
 * chain. Debits the uploader (from an escrow lock or direct balance —
 * caller decides where the pool lives), credits each host pro-rata,
 * routes recycle + reputation shares.
 *
 * Dependencies injected for test-ability. Returns the same structure
 * as computePayouts, enriched with a `transfers` array of recorded
 * ledger entries.
 *
 * NOTE: v2.11.1 does NOT modify blobCommit.payment_hone itself on
 * settlement — that's a downstream responsibility. Callers should
 * track "settled so far" separately or commit an empty BLOB_STORE_COMMIT
 * with negative delta to zero it out. A cleaner approach lands in
 * v2.11.2 alongside the escrow lock integration.
 */
async function settlePayouts(blobCommit, options) {
  options = options || {};
  var ledger = options.ledger || require("./ledger");
  var uploader = options.uploader || (blobCommit && blobCommit.uploader);
  var epoch = options.epoch;

  var calc = computePayouts(blobCommit);
  if (!calc || calc.total_pool <= 0 || calc.total_bytes_served <= 0) {
    return Object.assign({ transfers: [] }, calc || {});
  }
  if (!uploader) {
    throw new Error("uploader required for settlement");
  }
  if (!Number.isFinite(epoch)) {
    epoch = await ledger.getCurrentEpoch();
  }

  var transfers = [];

  // Host payouts
  for (var i = 0; i < calc.host_payouts.length; i++) {
    var p = calc.host_payouts[i];
    if (p.amount_hone > 0) {
      await ledger.recordTransfer(
        uploader,
        p.host,
        p.amount_hone,
        "HONE",
        null,
        epoch,
        "Blob payout (bandwidth): " + calc.cid
      );
      transfers.push({ to: p.host, amount: p.amount_hone, kind: "host_pro_rata" });
    }
  }

  // Recycle share
  if (calc.recycle_pool > 0) {
    await ledger.recordTransfer(
      uploader,
      "hone_recycle",
      calc.recycle_pool,
      "HONE",
      null,
      epoch,
      "Blob payout (recycle): " + calc.cid
    );
    transfers.push({ to: "hone_recycle", amount: calc.recycle_pool, kind: "recycle" });
  }

  // Reputation pool share
  if (calc.reputation_pool > 0) {
    await ledger.recordTransfer(
      uploader,
      "hone_reputation_pool",
      calc.reputation_pool,
      "HONE",
      null,
      epoch,
      "Blob payout (reputation): " + calc.cid
    );
    transfers.push({
      to: "hone_reputation_pool",
      amount: calc.reputation_pool,
      kind: "reputation",
    });
  }

  return Object.assign({ transfers: transfers }, calc);
}

function _round(n) {
  return parseFloat(Number(n).toFixed(10));
}

module.exports = {
  HOST_SHARE: HOST_SHARE,
  RECYCLE_SHARE: RECYCLE_SHARE,
  REPUTATION_SHARE: REPUTATION_SHARE,
  computePayouts: computePayouts,
  settlePayouts: settlePayouts,
};
