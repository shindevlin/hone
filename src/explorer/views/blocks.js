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

function blocksView(data) {
  var epochs = data.epochs;
  var total = data.total;
  var page = data.page;
  var perPage = data.perPage;
  var totalPages = Math.ceil(total / perPage);

  var epochRows = (epochs || []).map(function (e) {
    var commitCount = (e.commitments || []).length;
    return '<tr>' +
      '<td><a href="/block/' + e.epoch_number + '">' + e.epoch_number + '</a></td>' +
      '<td><span class="status status-' + e.status + '">' + e.status + '</span></td>' +
      '<td class="amount">' + formatNumber(e.block_reward) + ' BTCPC</td>' +
      '<td>' + formatNumber(e.total_work) + '</td>' +
      '<td>' + commitCount + '</td>' +
      '<td>' + (e.miner_id || "--") + '</td>' +
      '<td>' + formatDate(e.started_at) + '</td>' +
      '<td>' + formatDate(e.ended_at) + '</td>' +
      '</tr>';
  }).join("");

  var pagination = "";
  if (totalPages > 1) {
    var pages = [];
    if (page > 1) pages.push('<a href="/blocks?page=' + (page - 1) + '" style="background:var(--bg-card);border:1px solid var(--border);padding:6px 14px;border-radius:6px;font-size:13px;">&#8592; Prev</a>');
    pages.push('<span style="color:var(--text-muted);font-size:13px;padding:6px 10px;">Page ' + page + ' of ' + totalPages + '</span>');
    if (page < totalPages) pages.push('<a href="/blocks?page=' + (page + 1) + '" style="background:var(--bg-card);border:1px solid var(--border);padding:6px 14px;border-radius:6px;font-size:13px;">Next &#8594;</a>');
    pagination = '<div style="display:flex;align-items:center;justify-content:center;gap:8px;padding:16px;">' + pages.join("") + '</div>';
  }

  var content = '\
    <h1 class="page-title">All <span>Blocks</span></h1>\
\
    <div class="stats-grid" style="grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));">\
      <div class="stat-card">\
        <div class="stat-label">Total Blocks</div>\
        <div class="stat-value">' + formatNumber(total) + '</div>\
      </div>\
    </div>\
\
    <div class="card">\
      <div class="card-header">\
        <h2>Blocks</h2>\
        <span class="badge">' + formatNumber(total) + ' total</span>\
      </div>\
      ' + (epochRows.length ? '\
      <table>\
        <thead>\
          <tr>\
            <th>Epoch</th>\
            <th>Status</th>\
            <th>Reward</th>\
            <th>Work</th>\
            <th>Nodes</th>\
            <th>Miner</th>\
            <th>Started</th>\
            <th>Ended</th>\
          </tr>\
        </thead>\
        <tbody>' + epochRows + '</tbody>\
      </table>\
      ' + pagination : '<div class="empty-state">No blocks recorded yet</div>') + '\
    </div>\
  ';

  return layout("Blocks", content);
}

module.exports = blocksView;
