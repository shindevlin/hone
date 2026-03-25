"use strict";

const layout = require("./layout");

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
  const { user, node, stake, transactions, miningRewards, balance } = data;

  if (!user) {
    return layout("Account Not Found", `
      <h1 class="page-title">Account <span>Not Found</span></h1>
      <div class="card">
        <div class="empty-state">No account found with this username.</div>
      </div>
    `);
  }

  const txRows = (transactions || []).map(t => `
    <tr>
      <td><span class="type-badge type-${t.type}">${t.type.replace("_", " ")}</span></td>
      <td>${t.from === user.username ? `<strong>${t.from}</strong>` : `<a href="/account/${t.from}">${t.from}</a>`}</td>
      <td>${t.to === user.username ? `<strong>${t.to}</strong>` : `<a href="/account/${t.to}">${t.to}</a>`}</td>
      <td class="amount ${t.to === user.username ? 'positive' : t.from === user.username ? 'negative' : ''}">${t.to === user.username ? '+' : '-'}${formatNumber(t.amount)}</td>
      <td>${t.memo || "--"}</td>
      <td>${formatDate(t.timestamp)}</td>
    </tr>
  `).join("");

  const stakedAmount = stake ? stake.staked_amount : 0;
  const totalMined = (miningRewards || []).reduce((sum, t) => sum + t.amount, 0);

  const content = `
    <h1 class="page-title">Account: <span>${user.username}</span></h1>

    <div class="stats-grid">
      <div class="stat-card">
        <div class="stat-label">Available Balance</div>
        <div class="stat-value accent">${formatNumber(balance)}</div>
        <div class="stat-sub">BTCPC</div>
      </div>
      <div class="stat-card">
        <div class="stat-label">Staked</div>
        <div class="stat-value">${formatNumber(stakedAmount)}</div>
        <div class="stat-sub">${stake ? `Status: ${stake.status}` : "No active stake"}</div>
      </div>
      <div class="stat-card">
        <div class="stat-label">Total Mined</div>
        <div class="stat-value">${formatNumber(totalMined)}</div>
        <div class="stat-sub">${(miningRewards || []).length} rewards received</div>
      </div>
      <div class="stat-card">
        <div class="stat-label">Member Since</div>
        <div class="stat-value" style="font-size: 16px;">${formatDate(user.createdAt)}</div>
        <div class="stat-sub">Last login: ${formatDate(user.lastLogin)}</div>
      </div>
    </div>

    ${node ? `
    <div class="card">
      <div class="card-header">
        <h2>Mining Node</h2>
        <span class="status status-${node.status}">${node.status}</span>
      </div>
      <dl class="detail-grid">
        <dt>Endpoint</dt>
        <dd>${node.endpoint}</dd>
        <dt>GPU</dt>
        <dd>${node.hardware && node.hardware.gpu ? node.hardware.gpu : "Not declared"}</dd>
        <dt>VRAM</dt>
        <dd>${node.hardware && node.hardware.vram_gb ? node.hardware.vram_gb + " GB" : "--"}</dd>
        <dt>CPU Cores</dt>
        <dd>${node.hardware && node.hardware.cpu_cores ? node.hardware.cpu_cores : "--"}</dd>
        <dt>RAM</dt>
        <dd>${node.hardware && node.hardware.ram_gb ? node.hardware.ram_gb + " GB" : "--"}</dd>
        <dt>Stake</dt>
        <dd class="amount">${formatNumber(node.stake_amount)} BTCPC</dd>
        <dt>Reputation</dt>
        <dd>${node.reputation}/100</dd>
        <dt>Models</dt>
        <dd>${(node.models || []).length ? node.models.map(m => `<span class="model-tag">${m}</span>`).join(" ") : "None declared"}</dd>
        <dt>Last Commitment</dt>
        <dd>${node.last_epoch_commitment >= 0 ? `<a href="/block/${node.last_epoch_commitment}">Epoch #${node.last_epoch_commitment}</a>` : "Never"}</dd>
        <dt>Registered</dt>
        <dd>${formatDate(node.registered_at)}</dd>
      </dl>
    </div>
    ` : ""}

    <div class="card">
      <div class="card-header">
        <h2>Transaction History</h2>
        <span class="badge">${(transactions || []).length} transactions</span>
      </div>
      ${txRows.length ? `
      <table>
        <thead>
          <tr>
            <th>Type</th>
            <th>From</th>
            <th>To</th>
            <th>Amount</th>
            <th>Memo</th>
            <th>Time</th>
          </tr>
        </thead>
        <tbody>${txRows}</tbody>
      </table>
      ` : '<div class="empty-state">No transactions found for this account</div>'}
    </div>
  `;

  return layout(user.username, content);
}

module.exports = accountView;
