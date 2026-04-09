"use strict";

const layout = require("./layout");

function formatNumber(n) {
  if (n === null || n === undefined) return "0";
  return Number(n).toLocaleString("en-US", { maximumFractionDigits: 4 });
}

function tokenomicsView(data) {
  var totalSupply = data.totalSupply;
  var totalMined = data.totalMined;
  var currentEpoch = data.currentEpoch;
  var currentPeriod = data.currentPeriod;
  var periodTable = data.periodTable;
  var deployments = data.deployments;
  var totalStaked = data.totalStaked;
  var activeMiners = data.activeMiners;
  var activeClocks = data.activeClocks;
  var totalVerifiers = data.totalVerifiers;

  var unminted = totalSupply - totalMined;
  var circulating = totalMined - totalStaked;
  var minedPercent = totalSupply > 0 ? ((totalMined / totalSupply) * 100).toFixed(4) : "0";

  // Build emission curve SVG
  var svgWidth = 800;
  var svgHeight = 200;
  var barWidth = Math.floor((svgWidth - 40) / periodTable.length) - 4;
  var maxAllotment = Math.max.apply(null, periodTable.map(function (p) { return p.allotment; }));

  var bars = periodTable.map(function (p, i) {
    var barH = Math.max(4, (p.allotment / maxAllotment) * (svgHeight - 40));
    var x = 20 + i * (barWidth + 4);
    var y = svgHeight - 20 - barH;
    var fill = currentPeriod && p.period === currentPeriod.period ? "#f7931a" : "#2a3550";
    var stroke = currentPeriod && p.period === currentPeriod.period ? "#fbbf24" : "#3b5580";
    return '<rect x="' + x + '" y="' + y + '" width="' + barWidth + '" height="' + barH + '" rx="3" fill="' + fill + '" stroke="' + stroke + '" stroke-width="1"/>' +
      '<text x="' + (x + barWidth / 2) + '" y="' + (svgHeight - 4) + '" fill="#64748b" font-size="10" text-anchor="middle">P' + p.period + '</text>' +
      '<text x="' + (x + barWidth / 2) + '" y="' + (y - 4) + '" fill="#94a3b8" font-size="9" text-anchor="middle">' + (p.allotment / 1e6).toFixed(1) + 'M</text>';
  }).join("\n");

  var emissionSvg = '<svg viewBox="0 0 ' + svgWidth + ' ' + svgHeight + '" style="width:100%;max-width:800px;height:auto;">' +
    bars + '</svg>';

  // Period table rows
  var periodRows = periodTable.map(function (p) {
    var isCurrent = currentPeriod && p.period === currentPeriod.period;
    var rowStyle = isCurrent ? ' style="background:rgba(247,147,26,0.08);"' : "";
    var epochRange = formatNumber(p.start_epoch) + " - " + formatNumber(p.end_epoch);
    var durationLabel = p.duration_months >= 12 ? (p.duration_months / 12).toFixed(1) + " years" : p.duration_months + " months";
    return '<tr' + rowStyle + '>' +
      '<td>' + (isCurrent ? '<strong style="color:#f7931a;">P' + p.period + ' (current)</strong>' : 'P' + p.period) + '</td>' +
      '<td>' + durationLabel + '</td>' +
      '<td>' + epochRange + '</td>' +
      '<td>' + formatNumber(p.epochs) + '</td>' +
      '<td class="amount">' + formatNumber(p.allotment) + ' BTCPC</td>' +
      '<td class="amount accent">' + formatNumber(p.reward_per_epoch) + ' BTCPC</td>' +
      '</tr>';
  }).join("");

  // Cross-chain deployment table
  var wbtcpc = deployments.wBTCPC || {};
  var bondingCurve = deployments.wBTCPCBondingCurve || {};
  var sale = deployments.wBTCPCSale || {};
  var bridge = deployments.wBTCPCBridge || {};

  var chainNames = ["base", "arbitrum", "optimism", "ethereum", "polygon", "bsc", "solana", "hive"];
  var explorerUrls = {
    base: "https://basescan.org/address/",
    arbitrum: "https://arbiscan.io/address/",
    optimism: "https://optimistic.etherscan.io/address/",
    ethereum: "https://etherscan.io/address/",
    polygon: "https://polygonscan.com/address/",
    bsc: "https://bscscan.com/address/",
    solana: "https://solscan.io/token/",
    hive: null
  };

  var chainRows = chainNames.map(function (chain) {
    var info = wbtcpc[chain] || {};
    var addr = info.contract || info.mint || info.account || "--";
    var status = info.status || (info.contract || info.mint ? "deployed" : "pending");
    var statusClass = status === "deployed" || status === "live" ? "status-active" : "status-committed";
    var explorerBase = explorerUrls[chain];
    var addrDisplay = addr.length > 20 ? addr.slice(0, 10) + "..." + addr.slice(-8) : addr;
    var addrLink = explorerBase && addr !== "--" ? '<a href="' + explorerBase + addr + '" target="_blank">' + addrDisplay + '</a>' : addrDisplay;
    var chainId = info.chainId || "--";

    // Bonding curve price
    var bc = bondingCurve[chain] || {};
    var basePrice = bc.basePrice ? "$" + (parseInt(bc.basePrice) / 1e6).toFixed(2) : "--";

    // Sale contract
    var saleInfo = sale[chain] || {};
    var salePrice = saleInfo.priceUSD || "--";

    return '<tr>' +
      '<td style="text-transform:capitalize;font-weight:600;">' + chain + '</td>' +
      '<td>' + chainId + '</td>' +
      '<td><span class="status ' + statusClass + '">' + status + '</span></td>' +
      '<td>' + addrLink + '</td>' +
      '<td>' + salePrice + '</td>' +
      '<td>' + basePrice + '</td>' +
      '<td>' + (info.deployedAt || "--") + '</td>' +
      '</tr>';
  }).join("");

  // Reward split section
  var rewardSplitHtml = '<div class="stats-grid" style="grid-template-columns: repeat(3, 1fr);">' +
    '<div class="stat-card">' +
      '<div class="stat-label">Miners</div>' +
      '<div class="stat-value accent">85%</div>' +
      '<div class="stat-sub">Proportional to verified compute work</div>' +
    '</div>' +
    '<div class="stat-card">' +
      '<div class="stat-label">Verifiers</div>' +
      '<div class="stat-value" style="color:#3b82f6;">10%</div>' +
      '<div class="stat-sub">Commit-reveal verification nodes</div>' +
    '</div>' +
    '<div class="stat-card">' +
      '<div class="stat-label">Clocks</div>' +
      '<div class="stat-value" style="color:#a855f7;">5%</div>' +
      '<div class="stat-sub">Epoch timing authority nodes</div>' +
    '</div>' +
  '</div>' +
  '<div class="card" style="margin-top:16px;">' +
    '<div class="card-header"><h2>Idle Epoch Rules</h2></div>' +
    '<div style="padding:16px 20px;color:var(--text-secondary);font-size:13px;line-height:1.8;">' +
      '<p>When no inference work is submitted during an epoch, the block reward is still distributed:</p>' +
      '<ul style="margin:8px 0 0 20px;">' +
        '<li>Clock nodes receive their 5% share for maintaining epoch timing</li>' +
        '<li>The remaining 95% is held in escrow and distributed to miners/verifiers in the next active epoch</li>' +
        '<li>If 3 consecutive epochs are idle, the escrowed rewards return to the unminted pool</li>' +
      '</ul>' +
    '</div>' +
  '</div>';

  var content = '\
    <h1 class="page-title">BTCPC <span>Tokenomics</span></h1>\
\
    <div class="stats-grid">\
      <div class="stat-card">\
        <div class="stat-label">Total Supply</div>\
        <div class="stat-value">' + formatNumber(totalSupply) + '</div>\
        <div class="stat-sub">BTCPC (fixed, never changes)</div>\
      </div>\
      <div class="stat-card">\
        <div class="stat-label">Mined So Far</div>\
        <div class="stat-value accent">' + formatNumber(totalMined) + '</div>\
        <div class="stat-sub">' + minedPercent + '% of supply</div>\
        <div class="progress-bar"><div class="fill" style="width: ' + minedPercent + '%"></div></div>\
      </div>\
      <div class="stat-card">\
        <div class="stat-label">Unminted</div>\
        <div class="stat-value">' + formatNumber(unminted) + '</div>\
        <div class="stat-sub">Remaining to be mined</div>\
      </div>\
      <div class="stat-card">\
        <div class="stat-label">Circulating</div>\
        <div class="stat-value">' + formatNumber(circulating) + '</div>\
        <div class="stat-sub">Mined minus staked</div>\
      </div>\
      <div class="stat-card">\
        <div class="stat-label">Total Staked</div>\
        <div class="stat-value">' + formatNumber(totalStaked) + '</div>\
        <div class="stat-sub">Locked in staking pools</div>\
      </div>\
      <div class="stat-card">\
        <div class="stat-label">Current Epoch</div>\
        <div class="stat-value">' + formatNumber(currentEpoch) + '</div>\
        <div class="stat-sub">Period ' + (currentPeriod ? currentPeriod.period : "--") + '</div>\
      </div>\
      <div class="stat-card">\
        <div class="stat-label">Block Reward</div>\
        <div class="stat-value accent">' + (currentPeriod ? formatNumber(currentPeriod.reward_per_epoch) : "--") + '</div>\
        <div class="stat-sub">BTCPC per epoch</div>\
      </div>\
      <div class="stat-card">\
        <div class="stat-label">Network Nodes</div>\
        <div class="stat-value">' + formatNumber(activeMiners) + '</div>\
        <div class="stat-sub">' + formatNumber(activeClocks) + ' clocks, ' + formatNumber(totalVerifiers) + ' verifiers</div>\
      </div>\
    </div>\
\
    <div class="card">\
      <div class="card-header">\
        <h2>Emission Curve</h2>\
        <span class="badge">11 halving periods</span>\
      </div>\
      <div style="padding:20px;text-align:center;">\
        ' + emissionSvg + '\
        <div style="color:var(--text-muted);font-size:11px;margin-top:8px;">Each period: duration doubles, allotment grows by 13.4%, reward per epoch halves</div>\
      </div>\
    </div>\
\
    <div class="card">\
      <div class="card-header">\
        <h2>Emission Schedule</h2>\
        <span class="badge">' + periodTable.length + ' periods</span>\
      </div>\
      <table>\
        <thead>\
          <tr>\
            <th>Period</th>\
            <th>Duration</th>\
            <th>Epoch Range</th>\
            <th>Total Epochs</th>\
            <th>Allotment</th>\
            <th>Reward / Epoch</th>\
          </tr>\
        </thead>\
        <tbody>' + periodRows + '</tbody>\
      </table>\
    </div>\
\
    <h2 class="page-title" style="margin-top:32px;">Reward <span>Split</span></h2>\
    ' + rewardSplitHtml + '\
\
    <h2 class="page-title" style="margin-top:32px;">Cross-Chain <span>Deployments</span></h2>\
    <div class="card">\
      <div class="card-header">\
        <h2>wBTCPC Token Contracts</h2>\
        <span class="badge">' + chainNames.length + ' chains</span>\
      </div>\
      <table>\
        <thead>\
          <tr>\
            <th>Chain</th>\
            <th>Chain ID</th>\
            <th>Status</th>\
            <th>Contract Address</th>\
            <th>Sale Price</th>\
            <th>Bonding Price</th>\
            <th>Deployed</th>\
          </tr>\
        </thead>\
        <tbody>' + chainRows + '</tbody>\
      </table>\
    </div>\
  ';

  return layout("Tokenomics", content);
}

module.exports = tokenomicsView;
