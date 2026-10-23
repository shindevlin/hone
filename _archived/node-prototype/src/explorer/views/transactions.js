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

function transactionsView(data) {
  const { transactions, total, page, perPage } = data;
  const totalPages = Math.ceil(total / perPage);

  const txRows = (transactions || []).map(t => `
    <tr>
      <td><a class="hash" href="/tx/${encodeURIComponent(t.txHash)}">${escapeHtml(t.txHash)}</a></td>
      <td>${t.epoch !== null && t.epoch !== undefined ? `<a href="/block/${t.epoch}">${t.epoch}</a>` : "--"}</td>
      <td><span class="type-badge type-${escapeHtml(t.type)}">${escapeHtml(t.type.replace("_", " "))}</span></td>
      <td><a href="/account/${encodeURIComponent(t.from)}">${escapeHtml(t.from)}</a></td>
      <td><a href="/account/${encodeURIComponent(t.to)}">${escapeHtml(t.to)}</a></td>
      <td class="amount">${formatNumber(t.amount)} HONE</td>
      <td>${escapeHtml(t.memo || "--")}</td>
      <td>${formatDate(t.timestamp)}</td>
    </tr>
  `).join("");

  let pagination = "";
  if (totalPages > 1) {
    const pages = [];
    if (page > 1) pages.push(`<a href="/tx?page=${page - 1}" style="background: var(--bg-card); border: 1px solid var(--border); padding: 6px 14px; border-radius: 6px; font-size: 13px;">&#8592; Prev</a>`);
    pages.push(`<span style="color: var(--text-muted); font-size: 13px; padding: 6px 10px;">Page ${page} of ${totalPages}</span>`);
    if (page < totalPages) pages.push(`<a href="/tx?page=${page + 1}" style="background: var(--bg-card); border: 1px solid var(--border); padding: 6px 14px; border-radius: 6px; font-size: 13px;">Next &#8594;</a>`);
    pagination = `<div style="display: flex; align-items: center; justify-content: center; gap: 8px; padding: 16px;">${pages.join("")}</div>`;
  }

  const content = `
    <h1 class="page-title">Recent <span>Transactions</span></h1>

    <div class="stats-grid" style="grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));">
      <div class="stat-card">
        <div class="stat-label">Total Transactions</div>
        <div class="stat-value">${formatNumber(total)}</div>
      </div>
    </div>

    <div class="card">
      <div class="card-header">
        <h2>All Transactions</h2>
        <span class="badge">${formatNumber(total)} total</span>
      </div>
      ${txRows.length ? `
      <table>
        <thead>
          <tr>
            <th>TX Hash</th>
            <th>Epoch</th>
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
      ${pagination}
      ` : '<div class="empty-state">No transactions recorded yet</div>'}
    </div>
  `;

  return layout("Transactions", content);
}

module.exports = transactionsView;
