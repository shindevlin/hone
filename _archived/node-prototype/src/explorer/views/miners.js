"use strict";

const layout = require("./layout");
const { escapeHtml } = require("./escape");

function formatNumber(n) {
  if (n === null || n === undefined) return "0";
  return Number(n).toLocaleString("en-US", { maximumFractionDigits: 4 });
}

function formatDate(d) {
  if (!d) return "--";
  return new Date(d).toLocaleString("en-US", {
    month: "short", day: "numeric", year: "numeric",
    hour: "2-digit", minute: "2-digit"
  });
}

function minersView(data) {
  const { miners, totalStaked, activeCount, totalCount } = data;

  const minerRows = (miners || []).map(m => {
    const hw = m.hardware || {};
    const username = m._account ? m._account.username : (m.hive_account || "unknown");
    const models = (m.models || []).slice(0, 4).map(mod => `<span class="model-tag">${escapeHtml(mod)}</span>`).join(" ");
    const moreModels = (m.models || []).length > 4 ? `<span class="model-tag">+${m.models.length - 4} more</span>` : "";

    return `
    <tr>
      <td><a href="/account/${encodeURIComponent(username)}">${escapeHtml(username)}</a></td>
      <td><span class="status status-${escapeHtml(m.status)}">${escapeHtml(m.status)}</span></td>
      <td>
        <div class="hw-specs">
          ${hw.gpu ? escapeHtml(hw.gpu) : "N/A"}${hw.vram_gb ? ` (${escapeHtml(String(hw.vram_gb))}GB)` : ""}
        </div>
      </td>
      <td>${hw.cpu_cores || "--"} cores / ${hw.ram_gb || "--"} GB</td>
      <td>${models}${moreModels}</td>
      <td class="amount">${formatNumber(m.stake_amount)}</td>
      <td>${m.reputation}</td>
      <td>${m.last_epoch_commitment >= 0 ? `<a href="/block/${m.last_epoch_commitment}">#${m.last_epoch_commitment}</a>` : "--"}</td>
      <td>${formatDate(m.registered_at)}</td>
    </tr>`;
  }).join("");

  const content = `
    <h1 class="page-title">Network <span>Miners</span></h1>

    <div class="stats-grid" style="grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));">
      <div class="stat-card">
        <div class="stat-label">Total Nodes</div>
        <div class="stat-value">${formatNumber(totalCount)}</div>
      </div>
      <div class="stat-card">
        <div class="stat-label">Active Nodes</div>
        <div class="stat-value accent">${formatNumber(activeCount)}</div>
      </div>
      <div class="stat-card">
        <div class="stat-label">Total Staked</div>
        <div class="stat-value">${formatNumber(totalStaked)}</div>
        <div class="stat-sub">HONE</div>
      </div>
    </div>

    <div class="card">
      <div class="card-header">
        <h2>Registered Mining Nodes</h2>
        <span class="badge">${formatNumber(totalCount)} nodes</span>
      </div>
      ${minerRows.length ? `
      <div style="overflow-x: auto;">
        <table>
          <thead>
            <tr>
              <th>Account</th>
              <th>Status</th>
              <th>GPU</th>
              <th>CPU / RAM</th>
              <th>Models</th>
              <th>Stake</th>
              <th>Rep</th>
              <th>Last Epoch</th>
              <th>Registered</th>
            </tr>
          </thead>
          <tbody>${minerRows}</tbody>
        </table>
      </div>
      ` : '<div class="empty-state">No mining nodes registered yet</div>'}
    </div>
  `;

  return layout("Miners", content);
}

module.exports = minersView;
