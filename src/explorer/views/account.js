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
    hour: "2-digit", minute: "2-digit", second: "2-digit"
  });
}

function accountView(data) {
  var user = data.user;
  var node = data.node;
  var stake = data.stake;
  var transactions = data.transactions || [];
  var miningRewards = data.miningRewards || [];
  var balance = data.balance || 0;
  var pendingDebit = data.pendingDebit || 0;
  var availableBalance = data.availableBalance || balance;
  var smtProof = data.smtProof;

  if (!user) {
    return layout("Account Not Found", `
      <h1 class="page-title">Account <span>Not Found</span></h1>
      <div class="card">
        <div class="empty-state">No account found with this username.</div>
      </div>
    `);
  }

  var txRows = transactions.map(function (t) {
    var typeClass = (t.type || "").replace(/_/g, "-");
    var dirClass = t.to === user.username ? "positive" : t.from === user.username ? "negative" : "";
    var dirSign = t.to === user.username ? "+" : "-";
    return `
    <tr>
      <td><span class="type-badge type-${escapeHtml(typeClass)}">${escapeHtml((t.type || "").replace(/_/g, " "))}</span></td>
      <td>${t.from === user.username ? `<strong>${escapeHtml(t.from)}</strong>` : t.from ? `<a href="/account/${encodeURIComponent(t.from)}">${escapeHtml(t.from)}</a>` : "--"}</td>
      <td>${t.to === user.username ? `<strong>${escapeHtml(t.to)}</strong>` : t.to ? `<a href="/account/${encodeURIComponent(t.to)}">${escapeHtml(t.to)}</a>` : "--"}</td>
      <td class="amount ${dirClass}">${dirSign}${formatNumber(t.amount)} ${escapeHtml(t.token || "HONE")}</td>
      <td>${t.epoch !== undefined ? `<a href="/block/${t.epoch}">#${t.epoch}</a>` : "--"}</td>
      <td>${formatDate(t.timestamp)}</td>
    </tr>`;
  }).join("");

  var stakedAmount = stake ? stake.staked_amount : 0;
  var totalMined = miningRewards.reduce(function (sum, t) { return sum + t.amount; }, 0);

  var content = `
    <h1 class="page-title">Account: <span>${escapeHtml(user.username)}</span></h1>

    <div class="stats-grid">
      <div class="stat-card">
        <div class="stat-label">Ledger Balance</div>
        <div class="stat-value accent">${formatNumber(balance)}</div>
        <div class="stat-sub">HONE (source of truth)</div>
      </div>
      <div class="stat-card">
        <div class="stat-label">Available</div>
        <div class="stat-value">${formatNumber(availableBalance)}</div>
        <div class="stat-sub">${pendingDebit > 0 ? formatNumber(pendingDebit) + " pending" : "No pending txs"}</div>
      </div>
      <div class="stat-card">
        <div class="stat-label">Staked</div>
        <div class="stat-value">${formatNumber(stakedAmount)}</div>
        <div class="stat-sub">${stake ? `Status: ${escapeHtml(stake.status)}` : "No active stake"}</div>
      </div>
      <div class="stat-card">
        <div class="stat-label">Total Mined</div>
        <div class="stat-value">${formatNumber(totalMined)}</div>
        <div class="stat-sub">${miningRewards.length} rewards received</div>
      </div>
    </div>

    ${smtProof ? `
    <div class="card">
      <div class="card-header">
        <h2>State Proof (SMT)</h2>
        <span class="status status-active">verified</span>
      </div>
      <dl class="detail-grid">
        <dt>State Root</dt>
        <dd><span class="hash" style="max-width: none;">${smtProof.root}</span></dd>
        <dt>Balance (SMT)</dt>
        <dd class="amount">${formatNumber(smtProof.state.balance)} HONE</dd>
        <dt>Staked (SMT)</dt>
        <dd>${formatNumber(smtProof.state.staked)} HONE</dd>
        <dt>Delegated (SMT)</dt>
        <dd>${formatNumber(smtProof.state.delegated)} HONE</dd>
        <dt>Nonce</dt>
        <dd>${smtProof.state.nonce}</dd>
      </dl>
    </div>
    ` : ""}

    ${node ? `
    <div class="card">
      <div class="card-header">
        <h2>Mining Node</h2>
        <span class="status status-${escapeHtml(node.status)}">${escapeHtml(node.status)}</span>
      </div>
      <dl class="detail-grid">
        <dt>Endpoint</dt>
        <dd>${escapeHtml(node.endpoint)}</dd>
        <dt>GPU</dt>
        <dd>${node.hardware && node.hardware.gpu ? escapeHtml(node.hardware.gpu) : "Not declared"}</dd>
        <dt>VRAM</dt>
        <dd>${node.hardware && node.hardware.vram_gb ? escapeHtml(String(node.hardware.vram_gb)) + " GB" : "--"}</dd>
        <dt>Stake</dt>
        <dd class="amount">${formatNumber(node.stake_amount)} HONE</dd>
        <dt>Reputation</dt>
        <dd>${node.reputation}/100</dd>
        <dt>Models</dt>
        <dd>${(node.models || []).length ? node.models.map(function (m) { return '<span class="model-tag">' + escapeHtml(m) + '</span>'; }).join(" ") : "None"}</dd>
      </dl>
    </div>
    ` : ""}

    <div class="card">
      <div class="card-header">
        <h2>Ledger History</h2>
        <span class="badge">${transactions.length} entries</span>
      </div>
      ${txRows.length ? `
      <table>
        <thead>
          <tr>
            <th>Type</th>
            <th>From</th>
            <th>To</th>
            <th>Amount</th>
            <th>Block</th>
            <th>Time</th>
          </tr>
        </thead>
        <tbody>${txRows}</tbody>
      </table>
      ` : '<div class="empty-state">No ledger entries for this account</div>'}
    </div>
  `;

  return layout(escapeHtml(user.username), content);
}

module.exports = accountView;
