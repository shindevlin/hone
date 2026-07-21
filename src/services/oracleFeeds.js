"use strict";

/**
 * HONE Oracle Feeds — v2.13-delta
 * Shin Devlin
 *
 * Generic off-chain data consensus primitive. A "feed" is a named data
 * source (e.g., "btc-usd", "weather-nyc", "sports-nba-lakers") that
 * registered verifiers report values for on a schedule. Every
 * finalization epoch takes the MEDIAN of all received reports as the
 * canonical value. Reports that deviate more than max_deviation_bps
 * from the median get a reputation dip — no slashing.
 *
 * Same no-slash discipline as storage hosting (see
 * feedback_storage_no_slash.md): verifiers who miss an epoch just
 * don't get paid for that epoch. Verifiers who lie consistently lose
 * reputation until they stop being called.
 *
 * Design decisions:
 *   - Median, not mean. Median is robust to outliers; mean is not.
 *   - Min verifiers gate: a feed won't finalize an epoch with fewer
 *     than min_verifiers reports. Callers can bump min_verifiers to
 *     increase security or drop it for experimental feeds.
 *   - Open vs restricted verifier lists. Empty `verifiers` = open
 *     (anyone can report, filtered by stake later at the HTTP layer).
 *     Non-empty = strict allowlist.
 *   - Deviation is measured in basis points (1 bp = 0.01%) against
 *     the median, so it's meaningful for both low-magnitude and
 *     high-magnitude values.
 *   - This is the standalone primitive. Chain-integrated dispatcher
 *     (ORACLE_REPORT ledger entries) lands in the follow-up — same
 *     pattern as serviceRegistry → serviceRoutes.
 *
 * Feed ID format: "<owner>/<name>"
 *   - owner must be a valid HONE account name
 *   - name matches /^[a-z0-9][a-z0-9-]{0,62}$/
 *   - same rules as service slugs and commerce product slugs
 */

// feed_id → feed record
var feeds = new Map();

// "<feed_id>|<epoch>" → array of reports for that epoch
var reportsByEpoch = new Map();

// "<feed_id>|<epoch>" → finalized consensus record
var finalizedEpochs = new Map();

// verifier → { total_reports, total_outliers, last_epoch, feeds_seen: Set }
var verifierStats = new Map();

var ID_PATTERN = /^[a-z0-9][a-z0-9-]{0,62}$/;

function parseFeedId(feedId) {
  if (typeof feedId !== "string") {
    return { ok: false, error: "feed_id must be a string" };
  }
  var parts = feedId.split("/");
  if (parts.length !== 2) {
    return { ok: false, error: "feed_id must be <owner>/<name>" };
  }
  if (!ID_PATTERN.test(parts[0])) return { ok: false, error: "invalid owner name" };
  if (!ID_PATTERN.test(parts[1])) return { ok: false, error: "invalid feed name" };
  return { ok: true, owner: parts[0], name: parts[1] };
}

function _round10(n) {
  return parseFloat(Number(n).toFixed(10));
}

function _reportsKey(feedId, epoch) {
  return feedId + "|" + epoch;
}

function _median(values) {
  if (!values || values.length === 0) return null;
  var sorted = values.slice().sort(function (a, b) { return a - b; });
  var n = sorted.length;
  if (n % 2 === 1) return sorted[(n - 1) / 2];
  return (sorted[n / 2 - 1] + sorted[n / 2]) / 2;
}

function _bumpVerifierStats(verifier, outlier, epoch, feedId) {
  var stats = verifierStats.get(verifier) || {
    total_reports: 0,
    total_outliers: 0,
    last_epoch: 0,
    feeds_seen: new Set(),
  };
  stats.total_reports += 1;
  if (outlier) stats.total_outliers += 1;
  stats.last_epoch = epoch;
  stats.feeds_seen.add(feedId);
  verifierStats.set(verifier, stats);
  return stats;
}

/**
 * Register a new feed. Updates to an existing feed can change the
 * verifier list, interval, and deviation bounds but NOT the owner.
 *
 * @param {string} owner — account publishing the feed
 * @param {string} feedId — "<owner>/<name>"
 * @param {object} spec
 * @param {string} spec.description — human-readable description
 * @param {string} spec.unit — e.g., "USD", "celsius"
 * @param {number} spec.decimals — decimal precision (0-18)
 * @param {number} spec.update_interval_epochs — how often the feed is expected to update
 * @param {number} spec.min_verifiers — minimum reports required to finalize an epoch
 * @param {number} spec.max_deviation_bps — max deviation from median before outlier flag (basis points)
 * @param {string[]} [spec.verifiers] — allowlist (empty = open)
 * @param {object} [options]
 * @param {number} [options.epoch] — current epoch (for created_epoch/last_updated_epoch)
 */
function registerFeed(owner, feedId, spec, options) {
  options = options || {};
  if (!owner) throw new Error("owner required");

  var parsed = parseFeedId(feedId);
  if (!parsed.ok) throw new Error("invalid feed_id: " + parsed.error);
  if (parsed.owner !== owner) {
    throw new Error("feed_id owner prefix must match the publishing account");
  }
  if (!spec || typeof spec !== "object") throw new Error("spec required");

  if (typeof spec.description !== "string" || spec.description.length === 0) {
    throw new Error("spec.description required");
  }
  if (typeof spec.unit !== "string" || spec.unit.length === 0) {
    throw new Error("spec.unit required");
  }
  var decimals = parseInt(spec.decimals, 10);
  if (!Number.isFinite(decimals) || decimals < 0 || decimals > 18) {
    throw new Error("spec.decimals must be 0-18");
  }
  var interval = parseInt(spec.update_interval_epochs, 10);
  if (!Number.isFinite(interval) || interval < 1 || interval > 1000000) {
    throw new Error("spec.update_interval_epochs must be 1-1000000");
  }
  var minVerifiers = parseInt(spec.min_verifiers, 10);
  if (!Number.isFinite(minVerifiers) || minVerifiers < 1 || minVerifiers > 1000) {
    throw new Error("spec.min_verifiers must be 1-1000");
  }
  var maxDev = parseInt(spec.max_deviation_bps, 10);
  if (!Number.isFinite(maxDev) || maxDev < 0 || maxDev > 100000) {
    throw new Error("spec.max_deviation_bps must be 0-100000 (basis points)");
  }

  var verifierList = [];
  if (Array.isArray(spec.verifiers)) {
    for (var i = 0; i < spec.verifiers.length; i++) {
      var v = spec.verifiers[i];
      if (typeof v === "string" && v.length > 0 && verifierList.indexOf(v) < 0) {
        verifierList.push(v);
      }
    }
  }

  var existing = feeds.get(feedId);
  var record;
  if (existing) {
    if (existing.owner !== owner) {
      throw new Error("only the original owner can update this feed");
    }
    record = existing;
    record.description = spec.description;
    record.unit = spec.unit;
    record.decimals = decimals;
    record.update_interval_epochs = interval;
    record.min_verifiers = minVerifiers;
    record.max_deviation_bps = maxDev;
    record.verifiers = verifierList;
    record.last_updated_epoch = options.epoch || 0;
  } else {
    record = {
      feed_id: feedId,
      owner: owner,
      description: spec.description,
      unit: spec.unit,
      decimals: decimals,
      update_interval_epochs: interval,
      min_verifiers: minVerifiers,
      max_deviation_bps: maxDev,
      verifiers: verifierList,
      status: "active",
      created_epoch: options.epoch || 0,
      last_updated_epoch: options.epoch || 0,
      last_value: null,
      last_finalized_epoch: null,
      total_reports: 0,
      total_finalizations: 0,
    };
  }

  feeds.set(feedId, record);
  return record;
}

/**
 * Retire a feed. Only the owner can retire. Marks status=retired; no
 * new reports can be submitted, but historical consensus values remain
 * readable.
 */
function retireFeed(owner, feedId, epoch) {
  var record = feeds.get(feedId);
  if (!record) throw new Error("feed not found");
  if (record.owner !== owner) {
    throw new Error("only the owner can retire this feed");
  }
  if (record.status === "retired") return record;
  record.status = "retired";
  record.retired_epoch = epoch || 0;
  feeds.set(feedId, record);
  return record;
}

/**
 * Pause a feed without retiring — reports are rejected but the feed
 * can be un-paused via registerFeed (which resets status to active).
 */
function pauseFeed(owner, feedId, epoch) {
  var record = feeds.get(feedId);
  if (!record) throw new Error("feed not found");
  if (record.owner !== owner) {
    throw new Error("only the owner can pause this feed");
  }
  if (record.status === "retired") {
    throw new Error("cannot pause a retired feed");
  }
  record.status = "paused";
  record.paused_epoch = epoch || 0;
  feeds.set(feedId, record);
  return record;
}

function resumeFeed(owner, feedId, epoch) {
  var record = feeds.get(feedId);
  if (!record) throw new Error("feed not found");
  if (record.owner !== owner) {
    throw new Error("only the owner can resume this feed");
  }
  if (record.status !== "paused") {
    throw new Error("feed is not paused");
  }
  record.status = "active";
  record.resumed_epoch = epoch || 0;
  feeds.set(feedId, record);
  return record;
}

/**
 * Submit a verifier report for a feed/epoch.
 * Rejects:
 *   - unknown feed
 *   - paused/retired feed
 *   - verifier not in allowlist (when allowlist is non-empty)
 *   - verifier already reported for this feed+epoch
 *   - non-finite value
 */
function submitReport(verifier, feedId, value, epoch) {
  if (!verifier) throw new Error("verifier required");
  var record = feeds.get(feedId);
  if (!record) throw new Error("feed not found");
  if (record.status !== "active") {
    throw new Error("feed is not active: " + record.status);
  }
  if (record.verifiers.length > 0 && record.verifiers.indexOf(verifier) < 0) {
    throw new Error("verifier not in allowlist for this feed");
  }
  var numeric = Number(value);
  if (!Number.isFinite(numeric)) {
    throw new Error("report value must be finite");
  }

  var key = _reportsKey(feedId, epoch || 0);

  if (finalizedEpochs.has(key)) {
    throw new Error("feed epoch already finalized");
  }

  var reports = reportsByEpoch.get(key) || [];
  for (var i = 0; i < reports.length; i++) {
    if (reports[i].verifier === verifier) {
      throw new Error("verifier already reported for this epoch");
    }
  }

  var report = {
    feed_id: feedId,
    verifier: verifier,
    epoch: epoch || 0,
    value: _round10(numeric),
    timestamp: Date.now(),
  };

  reports.push(report);
  reportsByEpoch.set(key, reports);

  record.total_reports = (record.total_reports || 0) + 1;
  feeds.set(feedId, record);

  return report;
}

/**
 * Finalize a feed epoch — compute median, flag outliers, update the
 * feed's last_value. Idempotent: calling twice returns the same record.
 *
 * Returns { median, report_count, outliers: [...], feed: updatedRecord }
 * or throws if min_verifiers not met.
 */
function finalizeFeedEpoch(feedId, epoch) {
  var record = feeds.get(feedId);
  if (!record) throw new Error("feed not found");

  var key = _reportsKey(feedId, epoch || 0);
  if (finalizedEpochs.has(key)) {
    return finalizedEpochs.get(key);
  }

  var reports = reportsByEpoch.get(key) || [];
  if (reports.length < record.min_verifiers) {
    throw new Error(
      "insufficient reports: got " + reports.length +
      ", need " + record.min_verifiers
    );
  }

  var values = reports.map(function (r) { return r.value; });
  var median = _median(values);
  var medianRounded = _round10(median);

  // Flag outliers — anything more than max_deviation_bps from median
  // For median 0, treat any non-zero report as an outlier
  var outliers = [];
  var insiders = [];
  for (var i = 0; i < reports.length; i++) {
    var r = reports[i];
    var isOutlier = false;
    if (medianRounded === 0) {
      isOutlier = r.value !== 0;
    } else {
      var deviation = Math.abs((r.value - medianRounded) / medianRounded) * 10000; // bps
      if (deviation > record.max_deviation_bps) isOutlier = true;
    }
    if (isOutlier) {
      outliers.push({ verifier: r.verifier, value: r.value, deviation_bps: medianRounded === 0 ? Infinity : Math.abs((r.value - medianRounded) / medianRounded) * 10000 });
    } else {
      insiders.push(r.verifier);
    }
    _bumpVerifierStats(r.verifier, isOutlier, epoch || 0, feedId);
  }

  record.last_value = medianRounded;
  record.last_finalized_epoch = epoch || 0;
  record.total_finalizations = (record.total_finalizations || 0) + 1;
  feeds.set(feedId, record);

  var finalization = {
    feed_id: feedId,
    epoch: epoch || 0,
    median: medianRounded,
    report_count: reports.length,
    insiders: insiders,
    outliers: outliers,
    finalized_at: Date.now(),
  };

  finalizedEpochs.set(key, finalization);
  return finalization;
}

// ─────────────────────────────────────────────────────────────────
// Read accessors
// ─────────────────────────────────────────────────────────────────

function getFeed(feedId) {
  return feeds.get(feedId) || null;
}

function getAllFeeds(filter) {
  var result = [];
  for (var entry of feeds) {
    var f = entry[1];
    if (filter) {
      if (filter.owner && f.owner !== filter.owner) continue;
      if (filter.status && f.status !== filter.status) continue;
    }
    result.push(f);
  }
  return result;
}

function getReportsForEpoch(feedId, epoch) {
  var key = _reportsKey(feedId, epoch || 0);
  return (reportsByEpoch.get(key) || []).slice();
}

function getFinalization(feedId, epoch) {
  var key = _reportsKey(feedId, epoch || 0);
  return finalizedEpochs.get(key) || null;
}

/**
 * Current median value for a feed — last finalized consensus value.
 * Returns null if the feed has never been finalized.
 */
function getFeedValue(feedId) {
  var record = feeds.get(feedId);
  if (!record) return null;
  return record.last_value;
}

function getVerifierStats(verifier) {
  var stats = verifierStats.get(verifier);
  if (!stats) return null;
  return {
    verifier: verifier,
    total_reports: stats.total_reports,
    total_outliers: stats.total_outliers,
    outlier_rate: stats.total_reports > 0 ? stats.total_outliers / stats.total_reports : 0,
    last_epoch: stats.last_epoch,
    feeds_seen: Array.from(stats.feeds_seen),
  };
}

function resetForTests() {
  feeds.clear();
  reportsByEpoch.clear();
  finalizedEpochs.clear();
  verifierStats.clear();
}

module.exports = {
  // Mutators
  registerFeed: registerFeed,
  retireFeed: retireFeed,
  pauseFeed: pauseFeed,
  resumeFeed: resumeFeed,
  submitReport: submitReport,
  finalizeFeedEpoch: finalizeFeedEpoch,
  // Readers
  getFeed: getFeed,
  getAllFeeds: getAllFeeds,
  getReportsForEpoch: getReportsForEpoch,
  getFinalization: getFinalization,
  getFeedValue: getFeedValue,
  getVerifierStats: getVerifierStats,
  // Helpers
  parseFeedId: parseFeedId,
  resetForTests: resetForTests,
  ID_PATTERN: ID_PATTERN,
  // Exposed for testing
  _median: _median,
};
