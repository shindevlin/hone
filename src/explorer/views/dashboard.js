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

function dashboard(data) {
  const {
    totalSupply, totalMined, currentEpoch, totalMiners,
    totalTransactions, latestEpochs, recentTransactions,
    currentPeriod, networkHashRate, totalStaked,
    mempoolSize, latestBlockOnDisk, stateRoot
  } = data;

  const minedPercent = totalSupply > 0 ? ((totalMined / totalSupply) * 100).toFixed(2) : "0";

  const epochRows = (latestEpochs || []).map(e => `
    <tr>
      <td><a href="/block/${e.epoch_number}">${e.epoch_number}</a></td>
      <td><span class="status status-${e.status}">${e.status}</span></td>
      <td class="amount">${formatNumber(e.block_reward)} BTCPC</td>
      <td>${formatNumber(e.total_work)}</td>
      <td>${(e.commitments || []).length}</td>
      <td>${formatDate(e.started_at)}</td>
    </tr>
  `).join("");

  const txRows = (recentTransactions || []).map(t => `
    <tr>
      <td><span class="type-badge type-${t.type}">${t.type.replace("_", " ")}</span></td>
      <td><a href="/account/${t.from}">${t.from}</a></td>
      <td><a href="/account/${t.to}">${t.to}</a></td>
      <td class="amount">${formatNumber(t.amount)} BTCPC</td>
      <td>${formatDate(t.timestamp)}</td>
    </tr>
  `).join("");

  const content = `
    <h1 class="page-title">Bitcoin Proof of Compute &mdash; <span>Block Explorer</span></h1>

    <div class="stats-grid">
      <div class="stat-card">
        <div class="stat-label">Total Supply</div>
        <div class="stat-value">${formatNumber(totalSupply)}</div>
        <div class="stat-sub">BTCPC</div>
      </div>
      <div class="stat-card">
        <div class="stat-label">Total Mined</div>
        <div class="stat-value accent">${formatNumber(totalMined)}</div>
        <div class="stat-sub">${minedPercent}% of supply</div>
        <div class="progress-bar"><div class="fill" style="width: ${minedPercent}%"></div></div>
      </div>
      <div class="stat-card">
        <div class="stat-label">Current Epoch</div>
        <div class="stat-value">${formatNumber(currentEpoch)}</div>
        <div class="stat-sub">Period ${currentPeriod ? currentPeriod.period : "--"}</div>
      </div>
      <div class="stat-card">
        <div class="stat-label">Block Reward</div>
        <div class="stat-value accent">${currentPeriod ? formatNumber(currentPeriod.reward_per_epoch) : "--"}</div>
        <div class="stat-sub">BTCPC / epoch</div>
      </div>
      <div class="stat-card">
        <div class="stat-label">Active Miners</div>
        <div class="stat-value">${formatNumber(totalMiners)}</div>
        <div class="stat-sub">registered nodes</div>
      </div>
      <div class="stat-card">
        <div class="stat-label">Total Staked</div>
        <div class="stat-value">${formatNumber(totalStaked)}</div>
        <div class="stat-sub">BTCPC locked</div>
      </div>
      <div class="stat-card">
        <div class="stat-label">Mempool Size</div>
        <div class="stat-value">${formatNumber(mempoolSize)}</div>
        <div class="stat-sub">pending transactions</div>
      </div>
      <div class="stat-card">
        <div class="stat-label">Blocks On Disk</div>
        <div class="stat-value">${formatNumber(latestBlockOnDisk)}</div>
        <div class="stat-sub">latest block height</div>
      </div>
      <div class="stat-card">
        <div class="stat-label">State Root</div>
        <div class="stat-value" style="font-size: 20px;">${stateRoot ? stateRoot.slice(0, 12) + "..." : "--"}</div>
        <div class="stat-sub"><span class="hash">${stateRoot || "not available"}</span></div>
      </div>
    </div>

    <div class="search-bar" style="margin-bottom: 32px;">
      <input type="text" id="search-input" placeholder="Search by epoch number or username..." />
      <button onclick="doSearch()">Search</button>
    </div>

    <div class="two-col">
      <div class="card">
        <div class="card-header">
          <h2>Latest Epochs</h2>
          <span class="badge">${formatNumber(currentEpoch)} total</span>
        </div>
        ${epochRows.length ? `
        <table>
          <thead>
            <tr>
              <th>Epoch</th>
              <th>Status</th>
              <th>Reward</th>
              <th>Work</th>
              <th>Nodes</th>
              <th>Time</th>
            </tr>
          </thead>
          <tbody>${epochRows}</tbody>
        </table>
        ` : '<div class="empty-state">No epochs recorded yet</div>'}
      </div>

      <div class="card">
        <div class="card-header">
          <h2>Recent Transactions</h2>
          <a href="/tx"><span class="badge">View All</span></a>
        </div>
        ${txRows.length ? `
        <table>
          <thead>
            <tr>
              <th>Type</th>
              <th>From</th>
              <th>To</th>
              <th>Amount</th>
              <th>Time</th>
            </tr>
          </thead>
          <tbody>${txRows}</tbody>
        </table>
        ` : '<div class="empty-state">No transactions recorded yet</div>'}
      </div>
    </div>

    <script>
      function doSearch() {
        var q = document.getElementById('search-input').value.trim();
        if (!q) return;
        if (/^\\d+$/.test(q)) {
          window.location.href = '/block/' + q;
        } else {
          window.location.href = '/account/' + q;
        }
      }
      document.getElementById('search-input').addEventListener('keydown', function(e) {
        if (e.key === 'Enter') doSearch();
      });
    </script>
  `;

  return layout("Dashboard", content);
}

module.exports = dashboard;
