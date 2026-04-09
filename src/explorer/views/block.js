"use strict";

const layout = require("./layout");
const { escapeHtml } = require("./escape");

function formatNumber(n) {
  if (n === null || n === undefined) return "0";
  return Number(n).toLocaleString("en-US", { maximumFractionDigits: 8 });
}

function formatDate(d) {
  if (!d) return "--";
  return new Date(d).toLocaleString("en-US", {
    month: "short", day: "numeric", year: "numeric",
    hour: "2-digit", minute: "2-digit", second: "2-digit"
  });
}

function truncHash(h) {
  if (!h || h === "0".repeat(64)) return "--";
  return h.slice(0, 12) + "..." + h.slice(-8);
}

function blockView(epoch, period, blockData) {
  if (!epoch) {
    return layout("Block Not Found", `
      <h1 class="page-title">Block <span>Not Found</span></h1>
      <div class="card">
        <div class="empty-state">This block does not exist.</div>
      </div>
    `);
  }

  var block = blockData ? blockData.block : null;
  var payload = blockData ? blockData.payload : null;

  var blockHash = block ? block.computeHash() : null;
  var prevHash = block ? block.previous_block_hash : null;
  var stateRoot = block ? block.state_root : null;
  var txMerkle = block ? block.merkle_root_transactions : null;
  var cpMerkle = block ? block.merkle_root_compute_proofs : null;

  // Ledger entries from block file
  var ledgerEntries = payload ? payload.ledger_entries || [] : [];
  var ledgerRows = ledgerEntries.map(function (e) {
    var typeClass = (e.type || "").toLowerCase().replace(/_/g, "-");
    return `
    <tr>
      <td><span class="type-badge type-${escapeHtml(typeClass)}">${escapeHtml(e.type)}</span></td>
      <td>${e.from ? `<a href="/account/${encodeURIComponent(e.from)}">${escapeHtml(e.from)}</a>` : "--"}</td>
      <td>${e.to ? `<a href="/account/${encodeURIComponent(e.to)}">${escapeHtml(e.to)}</a>` : "--"}</td>
      <td class="amount">${formatNumber(e.amount)} ${escapeHtml(e.token || "BTCPC")}</td>
      <td>${escapeHtml(e.memo || "")}</td>
    </tr>`;
  }).join("");

  // Reward distribution
  var rewards = payload ? payload.rewards || [] : (epoch.rewards_distributed || []);
  var rewardRows = rewards.map(function (r) {
    var miner = r.miner || r.node_id || "--";
    return `
    <tr>
      <td><a href="/account/${encodeURIComponent(miner)}">${escapeHtml(miner)}</a></td>
      <td class="amount">${formatNumber(r.amount)} BTCPC</td>
    </tr>`;
  }).join("");

  // Compute proofs
  var proofs = payload ? payload.compute_proofs || [] : [];
  var proofRows = proofs.map(function (p) {
    return `
    <tr>
      <td><a href="/account/${encodeURIComponent(p.node_id)}">${escapeHtml(p.node_id)}</a></td>
      <td>${escapeHtml(p.model)}</td>
      <td>${formatNumber(p.tokens_generated)}</td>
      <td>${formatNumber(p.work_value)}</td>
      <td><span class="hash">${truncHash(p.result_hash)}</span></td>
    </tr>`;
  }).join("");

  var prevEpoch = epoch.epoch_number > 0 ? epoch.epoch_number - 1 : null;
  var nextEpoch = epoch.epoch_number + 1;

  var content = `
    <div style="display: flex; align-items: center; justify-content: space-between; margin-bottom: 24px;">
      <h1 class="page-title" style="margin-bottom: 0;">Block <span>#${epoch.epoch_number}</span></h1>
      <div style="display: flex; gap: 8px;">
        ${prevEpoch !== null ? `<a href="/block/${prevEpoch}" style="background: var(--bg-card); border: 1px solid var(--border); padding: 6px 14px; border-radius: 6px; font-size: 13px;">&#8592; Prev</a>` : ""}
        <a href="/block/${nextEpoch}" style="background: var(--bg-card); border: 1px solid var(--border); padding: 6px 14px; border-radius: 6px; font-size: 13px;">Next &#8594;</a>
      </div>
    </div>

    <div class="card">
      <div class="card-header">
        <h2>Block Header</h2>
        <span class="status status-${escapeHtml(epoch.status)}">${escapeHtml(epoch.status)}</span>
      </div>
      <dl class="detail-grid">
        <dt>Block Number</dt>
        <dd>${epoch.epoch_number}</dd>
        ${blockHash ? `<dt>Block Hash</dt><dd><span class="hash" style="max-width: none;">${blockHash}</span></dd>` : ""}
        ${prevHash ? `<dt>Previous Hash</dt><dd><span class="hash" style="max-width: none;">${prevEpoch !== null ? `<a href="/block/${prevEpoch}">${prevHash}</a>` : prevHash}</span></dd>` : ""}
        ${stateRoot ? `<dt>State Root (SMT)</dt><dd><span class="hash" style="max-width: none;">${stateRoot}</span></dd>` : ""}
        ${txMerkle ? `<dt>Tx Merkle Root</dt><dd><span class="hash" style="max-width: none;">${truncHash(txMerkle)}</span></dd>` : ""}
        ${cpMerkle ? `<dt>Compute Merkle Root</dt><dd><span class="hash" style="max-width: none;">${truncHash(cpMerkle)}</span></dd>` : ""}
        <dt>Block Reward</dt>
        <dd class="amount">${formatNumber(epoch.block_reward)} BTCPC</dd>
        <dt>Total Work</dt>
        <dd>${formatNumber(epoch.total_work)}</dd>
        <dt>Emission Period</dt>
        <dd>${period ? `Period ${period.period} (${period.duration_months} months, ${formatNumber(period.allotment)} BTCPC allotment)` : "--"}</dd>
        <dt>Difficulty</dt>
        <dd>${block ? block.difficulty : epoch.difficulty || 1}</dd>
        <dt>Timestamp</dt>
        <dd>${block ? formatDate(new Date(block.timestamp)) : formatDate(epoch.ended_at)}</dd>
        <dt>On Disk</dt>
        <dd>${block ? '<span class="status status-active">yes</span>' : '<span class="status status-committed">no</span>'}</dd>
      </dl>
    </div>

    <div class="card">
      <div class="card-header">
        <h2>Ledger Entries</h2>
        <span class="badge">${ledgerEntries.length} on-chain</span>
      </div>
      ${ledgerRows.length ? `
      <table>
        <thead>
          <tr>
            <th>Type</th>
            <th>From</th>
            <th>To</th>
            <th>Amount</th>
            <th>Memo</th>
          </tr>
        </thead>
        <tbody>${ledgerRows}</tbody>
      </table>
      ` : '<div class="empty-state">No ledger entries in this block</div>'}
    </div>

    <div class="two-col">
      <div class="card">
        <div class="card-header">
          <h2>Reward Distribution</h2>
          <span class="badge">${formatNumber(epoch.block_reward)} BTCPC</span>
        </div>
        ${rewardRows.length ? `
        <table>
          <thead><tr><th>Miner</th><th>Reward</th></tr></thead>
          <tbody>${rewardRows}</tbody>
        </table>
        ` : '<div class="empty-state">No rewards distributed</div>'}
      </div>

      <div class="card">
        <div class="card-header">
          <h2>Compute Proofs</h2>
          <span class="badge">${proofs.length}</span>
        </div>
        ${proofRows.length ? `
        <table>
          <thead><tr><th>Node</th><th>Model</th><th>Tokens</th><th>Work</th><th>Result Hash</th></tr></thead>
          <tbody>${proofRows}</tbody>
        </table>
        ` : '<div class="empty-state">No compute proofs</div>'}
      </div>
    </div>
  `;

  return layout(`Block #${epoch.epoch_number}`, content);
}

module.exports = blockView;
