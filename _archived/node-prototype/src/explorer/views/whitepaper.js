"use strict";

const layout = require("./layout");
const { escapeHtml } = require("./escape");

function formatDate(d) {
  if (!d) return "--";
  return new Date(d).toLocaleString("en-US", {
    month: "short", day: "numeric", year: "numeric",
    hour: "2-digit", minute: "2-digit", second: "2-digit"
  });
}

function whitepaperView(data) {
  const latest = data.latest || null;
  const revisions = data.revisions || [];

  const revisionRows = revisions.map((r) => `
    <tr id="revision-${escapeHtml(String(r.epoch))}">
      <td><a href="/block/${r.epoch}">#${r.epoch}</a></td>
      <td>${escapeHtml(r.whitepaper_title || "--")}</td>
      <td>${escapeHtml(r.whitepaper_version || "--")}</td>
      <td><span class="hash">${escapeHtml(r.whitepaper_hash || "--")}</span></td>
      <td>${escapeHtml(r.whitepaper_source_kind || "local")}</td>
      <td>${escapeHtml(r.whitepaper_source || "--")}</td>
      <td>${escapeHtml(r.whitepaper_path || "--")}</td>
    </tr>
  `).join("");

  const content = `
    <h1 class="page-title">Whitepaper <span>Revisions</span></h1>

    <div class="stats-grid">
      <div class="stat-card">
        <div class="stat-label">Latest Revision</div>
        <div class="stat-value accent" style="font-size: 18px;">${escapeHtml(latest ? latest.title || "HONE Whitepaper" : "--")}</div>
        <div class="stat-sub">epoch ${latest ? escapeHtml(String(latest.epoch)) : "--"} · ${escapeHtml(latest ? latest.version || "latest" : "--")}</div>
      </div>
      <div class="stat-card">
        <div class="stat-label">Hash</div>
        <div class="stat-value" style="font-size: 16px;">${escapeHtml(latest ? latest.hash || "--" : "--")}</div>
        <div class="stat-sub">${escapeHtml(latest ? latest.source_kind || "local" : "--")} source</div>
      </div>
      <div class="stat-card">
        <div class="stat-label">Source</div>
        <div class="stat-value" style="font-size: 16px;">${escapeHtml(latest ? latest.source || "--" : "--")}</div>
        <div class="stat-sub">canonical whitepaper source</div>
      </div>
      <div class="stat-card">
        <div class="stat-label">History</div>
        <div class="stat-value">${revisions.length}</div>
        <div class="stat-sub">stored revisions</div>
      </div>
    </div>

    <div class="card">
      <div class="card-header">
        <h2>Revision History</h2>
        <span class="badge">${revisions.length} shown</span>
      </div>
      ${revisionRows.length ? `
      <table>
        <thead>
          <tr>
            <th>Epoch</th>
            <th>Title</th>
            <th>Version</th>
            <th>Hash</th>
            <th>Source</th>
            <th>Origin</th>
            <th>Snapshot Path</th>
          </tr>
        </thead>
        <tbody>${revisionRows}</tbody>
      </table>
      ` : '<div class="empty-state">No whitepaper revisions published yet</div>'}
    </div>

    <div class="card">
      <div class="card-header">
        <h2>How It Works</h2>
        <span class="badge">10,000 epochs</span>
      </div>
      <div style="padding: 18px 20px; color: var(--text-secondary);">
        <p style="margin-bottom: 12px;">Dream #0 remains the original inscription for historical permanence. Every 10,000 epochs, HONE records the current whitepaper as a native <code>WHITEPAPER_REVISION</code> entry so the living document stays anchored without rewriting the past.</p>
        <p>When configured, the revision source can be a canonical raw GitHub URL. Otherwise, HONE uses the local <code>docs/HONE_WHITEPAPER.md</code> copy.</p>
      </div>
    </div>
  `;

  return layout("Whitepaper Revisions", content);
}

module.exports = whitepaperView;
