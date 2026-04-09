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

function userDashboardView(data) {
  // Landing page with search when no account specified
  if (!data || !data.user) {
    var content = '\
      <h1 class="page-title">Account <span>Dashboard</span></h1>\
      <div class="card">\
        <div style="padding:32px 20px;text-align:center;">\
          <p style="color:var(--text-secondary);margin-bottom:16px;">Enter a BTCPC account name to view their dashboard.</p>\
          <div class="search-bar" style="max-width:400px;margin:0 auto;">\
            <input type="text" id="dash-search" placeholder="Enter account name..." />\
            <button onclick="goDash()">View</button>\
          </div>\
        </div>\
      </div>\
      <script>\
        function goDash() {\
          var q = document.getElementById("dash-search").value.trim();\
          if (q) window.location.href = "/dashboard?account=" + encodeURIComponent(q);\
        }\
        document.getElementById("dash-search").addEventListener("keydown", function(e) {\
          if (e.key === "Enter") goDash();\
        });\
      </script>\
    ';
    return layout("Dashboard", content);
  }

  var user = data.user;
  var node = data.node;
  var stake = data.stake;
  var balance = data.balance || 0;
  var pendingDebit = data.pendingDebit || 0;
  var availableBalance = data.availableBalance || balance;
  var miningRewards = data.miningRewards || [];
  var recentTxs = data.recentTxs || [];
  var linkedAddresses = data.linkedAddresses || [];
  var pendingClaims = data.pendingClaims || [];
  var delegation = data.delegation;
  var clockNode = data.clockNode;
  var smtProof = data.smtProof;

  var totalMined = miningRewards.reduce(function (sum, t) { return sum + (t.amount || 0); }, 0);
  var stakedAmount = stake ? stake.staked_amount : 0;

  // Cross-chain balances section
  var crossChainRows = linkedAddresses.map(function (link) {
    return '<tr>' +
      '<td style="text-transform:capitalize;font-weight:600;">' + (link.chain || "--") + '</td>' +
      '<td><span class="hash">' + (link.address || "--") + '</span></td>' +
      '<td class="amount">' + formatNumber(link.balance || 0) + ' wBTCPC</td>' +
      '<td><span class="status ' + (link.verified ? 'status-active' : 'status-committed') + '">' + (link.verified ? 'verified' : 'pending') + '</span></td>' +
      '</tr>';
  }).join("");

  // Pending claims
  var claimRows = pendingClaims.map(function (c) {
    return '<tr>' +
      '<td style="text-transform:capitalize;">' + (c.chain || "--") + '</td>' +
      '<td class="amount">' + formatNumber(c.amount || 0) + ' wBTCPC</td>' +
      '<td>' + (c.epoch ? '<a href="/block/' + c.epoch + '">#' + c.epoch + '</a>' : "--") + '</td>' +
      '<td><span class="status status-committed">pending</span></td>' +
      '</tr>';
  }).join("");

  // Recent transactions
  var txRows = recentTxs.map(function (t) {
    var dirClass = t.to === user.username ? "positive" : t.from === user.username ? "negative" : "";
    var dirSign = t.to === user.username ? "+" : "-";
    return '<tr>' +
      '<td><span class="type-badge type-' + (t.type || "").replace(/_/g, "-") + '">' + (t.type || "").replace(/_/g, " ") + '</span></td>' +
      '<td>' + (t.from === user.username ? '<strong>' + t.from + '</strong>' : t.from ? '<a href="/account/' + t.from + '">' + t.from + '</a>' : "--") + '</td>' +
      '<td>' + (t.to === user.username ? '<strong>' + t.to + '</strong>' : t.to ? '<a href="/account/' + t.to + '">' + t.to + '</a>' : "--") + '</td>' +
      '<td class="amount ' + dirClass + '">' + dirSign + formatNumber(t.amount) + ' ' + (t.token || "BTCPC") + '</td>' +
      '<td>' + formatDate(t.timestamp) + '</td>' +
      '</tr>';
  }).join("");

  var content = '\
    <h1 class="page-title">Dashboard: <span>' + user.username + '</span></h1>\
\
    <div class="stats-grid">\
      <div class="stat-card">\
        <div class="stat-label">BTCPC Balance</div>\
        <div class="stat-value accent">' + formatNumber(balance) + '</div>\
        <div class="stat-sub">Native chain balance</div>\
      </div>\
      <div class="stat-card">\
        <div class="stat-label">Available</div>\
        <div class="stat-value">' + formatNumber(availableBalance) + '</div>\
        <div class="stat-sub">' + (pendingDebit > 0 ? formatNumber(pendingDebit) + ' pending debit' : 'No pending txs') + '</div>\
      </div>\
      <div class="stat-card">\
        <div class="stat-label">Total Mined</div>\
        <div class="stat-value">' + formatNumber(totalMined) + '</div>\
        <div class="stat-sub">' + miningRewards.length + ' rewards received</div>\
      </div>\
      <div class="stat-card">\
        <div class="stat-label">Staked</div>\
        <div class="stat-value">' + formatNumber(stakedAmount) + '</div>\
        <div class="stat-sub">' + (stake ? 'Status: ' + stake.status : 'Not staking') + '</div>\
      </div>\
    </div>\
\
    ' + (node ? '\
    <div class="card">\
      <div class="card-header">\
        <h2>Mining Node</h2>\
        <span class="status status-' + node.status + '">' + node.status + '</span>\
      </div>\
      <dl class="detail-grid">\
        <dt>Endpoint</dt>\
        <dd>' + (node.endpoint || "--") + '</dd>\
        <dt>GPU</dt>\
        <dd>' + (node.hardware && node.hardware.gpu ? node.hardware.gpu : "Not declared") + '</dd>\
        <dt>VRAM</dt>\
        <dd>' + (node.hardware && node.hardware.vram_gb ? node.hardware.vram_gb + " GB" : "--") + '</dd>\
        <dt>Stake</dt>\
        <dd class="amount">' + formatNumber(node.stake_amount) + ' BTCPC</dd>\
        <dt>Reputation</dt>\
        <dd>' + (node.reputation || 0) + '/100</dd>\
        <dt>Models</dt>\
        <dd>' + ((node.models || []).length ? node.models.map(function (m) { return "<span class=\"model-tag\">" + m + "</span>"; }).join(" ") : "None") + '</dd>\
        <dt>Total Work</dt>\
        <dd>' + formatNumber(node.total_work || 0) + '</dd>\
      </dl>\
    </div>\
    ' : '') + '\
\
    ' + (clockNode ? '\
    <div class="card">\
      <div class="card-header">\
        <h2>Clock Node</h2>\
        <span class="status status-active">active</span>\
      </div>\
      <dl class="detail-grid">\
        <dt>Node ID</dt>\
        <dd>' + (clockNode.node_id || user.username) + '</dd>\
        <dt>Registered</dt>\
        <dd>' + formatDate(clockNode.registered_at || clockNode.createdAt) + '</dd>\
        <dt>Epochs Timed</dt>\
        <dd>' + formatNumber(clockNode.epochs_timed || 0) + '</dd>\
        <dt>Last Heartbeat</dt>\
        <dd>' + formatDate(clockNode.last_heartbeat) + '</dd>\
      </dl>\
    </div>\
    ' : '') + '\
\
    ' + (delegation ? '\
    <div class="card">\
      <div class="card-header">\
        <h2>Delegation</h2>\
        <span class="badge">' + formatNumber(delegation.amount || 0) + ' BTCPC</span>\
      </div>\
      <dl class="detail-grid">\
        <dt>Delegated To</dt>\
        <dd><a href="/account/' + (delegation.delegatee || "--") + '">' + (delegation.delegatee || "--") + '</a></dd>\
        <dt>Amount</dt>\
        <dd class="amount">' + formatNumber(delegation.amount || 0) + ' BTCPC</dd>\
        <dt>Status</dt>\
        <dd><span class="status status-active">' + (delegation.status || "active") + '</span></dd>\
      </dl>\
    </div>\
    ' : '') + '\
\
    ' + (smtProof ? '\
    <div class="card">\
      <div class="card-header">\
        <h2>State Proof (SMT)</h2>\
        <span class="status status-active">verified</span>\
      </div>\
      <dl class="detail-grid">\
        <dt>State Root</dt>\
        <dd><span class="hash" style="max-width:none;">' + smtProof.root + '</span></dd>\
        <dt>Balance (SMT)</dt>\
        <dd class="amount">' + formatNumber(smtProof.state.balance) + ' BTCPC</dd>\
        <dt>Staked (SMT)</dt>\
        <dd>' + formatNumber(smtProof.state.staked) + ' BTCPC</dd>\
        <dt>Delegated (SMT)</dt>\
        <dd>' + formatNumber(smtProof.state.delegated) + ' BTCPC</dd>\
        <dt>Nonce</dt>\
        <dd>' + smtProof.state.nonce + '</dd>\
      </dl>\
    </div>\
    ' : '') + '\
\
    <div class="card">\
      <div class="card-header">\
        <h2>Cross-Chain Balances</h2>\
        <span class="badge">' + linkedAddresses.length + ' linked</span>\
      </div>\
      ' + (crossChainRows.length ? '\
      <table>\
        <thead>\
          <tr>\
            <th>Chain</th>\
            <th>Address</th>\
            <th>wBTCPC Balance</th>\
            <th>Status</th>\
          </tr>\
        </thead>\
        <tbody>' + crossChainRows + '</tbody>\
      </table>\
      ' : '<div class="empty-state">No linked cross-chain addresses</div>') + '\
    </div>\
\
    ' + (claimRows.length ? '\
    <div class="card">\
      <div class="card-header">\
        <h2>Pending Claims</h2>\
        <span class="badge">' + pendingClaims.length + ' pending</span>\
      </div>\
      <table>\
        <thead>\
          <tr>\
            <th>Chain</th>\
            <th>Amount</th>\
            <th>Epoch</th>\
            <th>Status</th>\
          </tr>\
        </thead>\
        <tbody>' + claimRows + '</tbody>\
      </table>\
    </div>\
    ' : '') + '\
\
    <div class="card">\
      <div class="card-header">\
        <h2>Recent Activity</h2>\
        <a href="/account/' + user.username + '"><span class="badge">Full History</span></a>\
      </div>\
      ' + (txRows.length ? '\
      <table>\
        <thead>\
          <tr>\
            <th>Type</th>\
            <th>From</th>\
            <th>To</th>\
            <th>Amount</th>\
            <th>Time</th>\
          </tr>\
        </thead>\
        <tbody>' + txRows + '</tbody>\
      </table>\
      ' : '<div class="empty-state">No recent activity</div>') + '\
    </div>\
  ';

  return layout("Dashboard: " + user.username, content);
}

module.exports = userDashboardView;
