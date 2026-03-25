"use strict";

const layout = require("./layout");

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

function blockView(epoch, period) {
  if (!epoch) {
    return layout("Epoch Not Found", `
      <h1 class="page-title">Epoch <span>Not Found</span></h1>
      <div class="card">
        <div class="empty-state">This epoch does not exist in the database.</div>
      </div>
    `);
  }

  const commitmentRows = (epoch.commitments || []).map((c, i) => `
    <tr>
      <td>${i + 1}</td>
      <td>${c.node_id ? `<span class="hash">${c.node_id}</span>` : "--"}</td>
      <td><span class="hash">${c.state_hash || "--"}</span></td>
      <td>${c.tx_count || 0}</td>
      <td>${c.inference_count || 0}</td>
      <td>${formatDate(c.submitted_at)}</td>
    </tr>
  `).join("");

  const rewardRows = (epoch.rewards_distributed || []).map(r => `
    <tr>
      <td>${r.node_id ? `<span class="hash">${r.node_id}</span>` : "--"}</td>
      <td class="amount">${formatNumber(r.amount)} BTCPC</td>
    </tr>
  `).join("");

  const prevEpoch = epoch.epoch_number > 0 ? epoch.epoch_number - 1 : null;
  const nextEpoch = epoch.epoch_number + 1;

  const content = `
    <div style="display: flex; align-items: center; justify-content: space-between; margin-bottom: 24px;">
      <h1 class="page-title" style="margin-bottom: 0;">Epoch <span>#${epoch.epoch_number}</span></h1>
      <div style="display: flex; gap: 8px;">
        ${prevEpoch !== null ? `<a href="/block/${prevEpoch}" style="background: var(--bg-card); border: 1px solid var(--border); padding: 6px 14px; border-radius: 6px; font-size: 13px;">&#8592; Prev</a>` : ""}
        <a href="/block/${nextEpoch}" style="background: var(--bg-card); border: 1px solid var(--border); padding: 6px 14px; border-radius: 6px; font-size: 13px;">Next &#8594;</a>
      </div>
    </div>

    <div class="card">
      <div class="card-header">
        <h2>Epoch Details</h2>
        <span class="status status-${epoch.status}">${epoch.status}</span>
      </div>
      <dl class="detail-grid">
        <dt>Epoch Number</dt>
        <dd>${epoch.epoch_number}</dd>
        <dt>Status</dt>
        <dd><span class="status status-${epoch.status}">${epoch.status}</span></dd>
        <dt>Block Reward</dt>
        <dd class="amount">${formatNumber(epoch.block_reward)} BTCPC</dd>
        <dt>Total Work</dt>
        <dd>${formatNumber(epoch.total_work)}</dd>
        <dt>Emission Period</dt>
        <dd>${period ? `Period ${period.period} (${period.duration_months} months, ${formatNumber(period.allotment)} BTCPC allotment)` : "--"}</dd>
        <dt>Consensus Hash</dt>
        <dd><span class="hash" style="max-width: none;">${epoch.consensus_hash || "pending"}</span></dd>
        <dt>Commitments</dt>
        <dd>${(epoch.commitments || []).length} nodes</dd>
        <dt>Started</dt>
        <dd>${formatDate(epoch.started_at)}</dd>
        <dt>Ended</dt>
        <dd>${formatDate(epoch.ended_at)}</dd>
      </dl>
    </div>

    <div class="two-col">
      <div class="card">
        <div class="card-header">
          <h2>Node Commitments</h2>
          <span class="badge">${(epoch.commitments || []).length}</span>
        </div>
        ${commitmentRows.length ? `
        <table>
          <thead>
            <tr>
              <th>#</th>
              <th>Node</th>
              <th>State Hash</th>
              <th>Txns</th>
              <th>Inferences</th>
              <th>Submitted</th>
            </tr>
          </thead>
          <tbody>${commitmentRows}</tbody>
        </table>
        ` : '<div class="empty-state">No commitments for this epoch</div>'}
      </div>

      <div class="card">
        <div class="card-header">
          <h2>Reward Distribution</h2>
          <span class="badge">${formatNumber(epoch.block_reward)} BTCPC</span>
        </div>
        ${rewardRows.length ? `
        <table>
          <thead>
            <tr>
              <th>Node</th>
              <th>Reward</th>
            </tr>
          </thead>
          <tbody>${rewardRows}</tbody>
        </table>
        ` : '<div class="empty-state">No rewards distributed yet</div>'}
      </div>
    </div>
  `;

  return layout(`Epoch #${epoch.epoch_number}`, content);
}

module.exports = blockView;
