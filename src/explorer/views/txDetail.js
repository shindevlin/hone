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

function txDetailView(data) {
  if (!data || !data.tx) {
    var hash = data ? data.txHash : "";
    return layout("Transaction Not Found", '\
      <h1 class="page-title">Transaction <span>Not Found</span></h1>\
      <div class="card">\
        <div class="empty-state">No transaction found with hash: ' + escapeHtml(hash || "unknown") + '</div>\
      </div>\
    ');
  }

  var tx = data.tx;
  var typeClass = (tx.type || "").replace(/_/g, "-");
  var statusLabel = tx.status || "confirmed";
  var statusClass = statusLabel === "pending" ? "status-committed" : "status-active";

  var content = '\
    <h1 class="page-title">Transaction <span>Detail</span></h1>\
\
    <div class="card">\
      <div class="card-header">\
        <h2>Transaction</h2>\
        <span class="status ' + statusClass + '">' + statusLabel + '</span>\
      </div>\
      <dl class="detail-grid">\
        <dt>TX Hash</dt>\
        <dd><span class="hash" style="max-width:none;">' + escapeHtml(tx.txHash || "--") + '</span></dd>\
        <dt>Type</dt>\
        <dd><span class="type-badge type-' + typeClass + '">' + (tx.type || "").replace(/_/g, " ") + '</span></dd>\
        <dt>From</dt>\
        <dd>' + (tx.from ? '<a href="/account/' + encodeURIComponent(tx.from) + '">' + escapeHtml(tx.from) + '</a>' : "--") + '</dd>\
        <dt>To</dt>\
        <dd>' + (tx.to ? '<a href="/account/' + encodeURIComponent(tx.to) + '">' + escapeHtml(tx.to) + '</a>' : "--") + '</dd>\
        <dt>Amount</dt>\
        <dd class="amount">' + formatNumber(tx.amount) + ' ' + (tx.token || "BTCPC") + '</dd>\
        <dt>Epoch</dt>\
        <dd>' + (tx.epoch !== undefined && tx.epoch !== null ? '<a href="/block/' + tx.epoch + '">#' + tx.epoch + '</a>' : "--") + '</dd>\
        <dt>Timestamp</dt>\
        <dd>' + formatDate(tx.timestamp) + '</dd>\
        <dt>Memo</dt>\
        <dd>' + escapeHtml(tx.memo || "--") + '</dd>\
        ' + (tx.signed_by ? '<dt>Signed By</dt><dd>' + escapeHtml(tx.signed_by) + '</dd>' : "") + '\
      </dl>\
    </div>\
  ';

  return layout("TX " + (tx.txHash ? tx.txHash.slice(0, 12) + "..." : ""), content);
}

module.exports = txDetailView;
