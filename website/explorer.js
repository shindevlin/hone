/* btcpcscan — blockchain explorer logic */
"use strict";

// ─── API helpers ─────────────────────────────────────────────────────────────

async function apiFetch(path) {
  const r = await fetch(path);
  if (!r.ok) {
    const txt = await r.text().catch(() => r.statusText);
    throw new Error(txt || r.statusText);
  }
  return r.json();
}

// ─── Formatters ──────────────────────────────────────────────────────────────

function fmt(n, dec = 2) {
  if (n == null || isNaN(n)) return "—";
  return Number(n).toLocaleString(undefined, { minimumFractionDigits: dec, maximumFractionDigits: dec });
}

function fmtInt(n) {
  if (n == null || isNaN(n)) return "—";
  return Number(n).toLocaleString();
}

function fmtDate(ms) {
  if (!ms) return "—";
  return new Date(ms).toLocaleString();
}

function fmtAge(ms) {
  if (!ms) return "—";
  const s = Math.round((Date.now() - ms) / 1000);
  if (s < 60)  return `${s}s ago`;
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  return `${Math.floor(s / 86400)}d ago`;
}

function truncHash(h, len = 12) {
  if (!h || h === "null" || h === "undefined") return "—";
  return h.length > len * 2 + 3 ? `${h.slice(0, len)}…${h.slice(-6)}` : h;
}

function badgeRole(role) {
  const map = {
    miner: "badge-miner", storage: "badge-storage",
    clock: "badge-clock", sensor: "badge-sensor",
  };
  return `<span class="badge ${map[role] || "badge-tx"}">${role}</span>`;
}

function entryTypeBadge(type) {
  if (!type) return "";
  const cls = type.includes("MINING") ? "badge-miner"
    : type.includes("STORAGE") ? "badge-storage"
    : type.includes("CLOCK") ? "badge-clock"
    : type.includes("SENSOR") ? "badge-sensor"
    : "badge-tx";
  return `<span class="badge ${cls}">${type.replace(/_/g, " ")}</span>`;
}

// ─── DOM helpers ──────────────────────────────────────────────────────────────

function el(id) { return document.getElementById(id); }

function setHTML(id, html) { const e = el(id); if (e) e.innerHTML = html; }

function loading(id) {
  setHTML(id, `<div class="loading-msg"><span class="spinner"></span> Loading…</div>`);
}

function errMsg(id, msg) {
  setHTML(id, `<div class="error-msg">Error: ${escHtml(String(msg))}</div>`);
}

function empty(id, msg = "No data found.") {
  setHTML(id, `<div class="empty-msg">${escHtml(msg)}</div>`);
}

function escHtml(s) {
  return String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

// ─── Router ────────────────────────────────────────────────────────────────────

const VIEWS = ["home", "blocks", "block-detail", "account", "sensors", "sensor-detail", "accounts"];

function showView(name) {
  VIEWS.forEach(v => {
    const e = el(`view-${v}`);
    if (e) e.classList.toggle("active", v === name);
  });
  document.querySelectorAll("#app-nav a.nav-link").forEach(a => {
    a.classList.toggle("active", a.dataset.view === name);
  });
}

function navigate(hash) {
  window.location.hash = hash;
}

function handleRoute() {
  const hash = window.location.hash || "#home";
  const [view, ...parts] = hash.replace(/^#/, "").split("/");
  const param = parts.join("/");

  switch (view) {
    case "home":      showView("home");    loadHome();    break;
    case "blocks":    showView("blocks");  loadBlocks();  break;
    case "accounts":  showView("accounts"); loadAccounts(); break;
    case "sensors":   showView("sensors"); loadSensors(); break;
    case "block":
      if (param) { showView("block-detail"); loadBlockDetail(param); }
      else navigate("#blocks");
      break;
    case "account":
      if (param) { showView("account"); loadAccount(param); }
      else navigate("#accounts");
      break;
    case "sensor":
      if (param) { showView("sensor-detail"); loadSensorDetail(decodeURIComponent(param)); }
      else navigate("#sensors");
      break;
    default:
      navigate("#home");
  }
}

// ─── Home ─────────────────────────────────────────────────────────────────────

let homeTimer = null;

async function loadHome() {
  clearInterval(homeTimer);
  await refreshHome();
  homeTimer = setInterval(refreshHome, 15000);
}

async function refreshHome() {
  try {
    const [status, blocks, activity] = await Promise.all([
      apiFetch("/status"),
      apiFetch("/blocks?limit=10"),
      apiFetch("/activity?limit=20"),
    ]);
    renderStats(status);
    renderRecentBlocks(blocks.blocks || []);
    renderRecentActivity(activity.entries || []);
    renderSupplyBar(status);
  } catch (err) {
    errMsg("stats-grid", err.message);
  }
}

function renderStats(s) {
  setHTML("stats-grid", `
    <div class="stat-card">
      <div class="stat-label">Block Height</div>
      <div class="stat-value orange">${fmtInt(s.chain_height)}</div>
    </div>
    <div class="stat-card">
      <div class="stat-label">Epoch Time</div>
      <div class="stat-value">${(s.epoch_time_ms / 1000)}s</div>
    </div>
    <div class="stat-card">
      <div class="stat-label">Active Nodes</div>
      <div class="stat-value green">${fmtInt(s.active_nodes)}</div>
      <div class="stat-sub">${s.miners || 0} miners · ${s.storage_nodes || 0} storage · ${s.clock_nodes || 0} clocks</div>
    </div>
    <div class="stat-card">
      <div class="stat-label">Accounts</div>
      <div class="stat-value">${fmtInt(s.accounts)}</div>
    </div>
    <div class="stat-card">
      <div class="stat-label">Circulating Supply</div>
      <div class="stat-value mono">${fmt(s.circulating_supply, 2)}</div>
      <div class="stat-sub">of ${fmtInt(s.max_supply)} BTCPC max</div>
    </div>
    <div class="stat-card">
      <div class="stat-label">Epoch Reward</div>
      <div class="stat-value">${fmt(s.current_reward_per_epoch, 2)}</div>
      <div class="stat-sub">BTCPC / epoch</div>
    </div>
  `);

  setHTML("supply-bar-section", `
    <div class="card">
      <div class="card-title">Supply Distribution</div>
      <div style="display:flex;justify-content:space-between;font-size:12px;margin-bottom:6px;">
        <span>Circulating: <strong>${fmt(s.circulating_supply, 4)} BTCPC</strong></span>
        <span style="color:var(--text-dim)">Max: 42,000,000 BTCPC</span>
      </div>
      <div class="supply-bar-wrap">
        <div class="supply-bar" id="supply-bar" style="width:${Math.min(100, (s.circulating_supply / 42000000) * 100).toFixed(3)}%"></div>
      </div>
      <div style="font-size:11px;color:var(--text-dim);margin-top:6px;">
        ${((s.circulating_supply / 42000000) * 100).toFixed(4)}% of max supply issued
      </div>
    </div>
  `);
}

function renderRecentBlocks(blocks) {
  if (!blocks.length) { empty("recent-blocks-table", "No blocks yet."); return; }
  setHTML("recent-blocks-table", `
    <div class="tbl-wrap">
      <table>
        <thead><tr>
          <th>Epoch</th><th>Age</th><th>Miner</th><th>Entries</th><th>Reward</th>
        </tr></thead>
        <tbody>${blocks.map(b => `
          <tr>
            <td><a class="link" onclick="navigate('#block/${b.epoch}')">${fmtInt(b.epoch)}</a></td>
            <td style="color:var(--text-dim)">${fmtAge(b.timestamp)}</td>
            <td><a class="link" onclick="navigate('#account/${escHtml(b.miner || b.proposer || '')}')">${escHtml(b.miner || b.proposer || "—")}</a></td>
            <td>${b.entry_count}</td>
            <td>${b.reward != null ? fmt(b.reward, 2) + " BTCPC" : "—"}</td>
          </tr>`).join("")}
        </tbody>
      </table>
    </div>
  `);
}

function renderRecentActivity(entries) {
  if (!entries.length) { empty("recent-activity-table", "No activity yet."); return; }
  setHTML("recent-activity-table", `
    <div class="tbl-wrap">
      <table>
        <thead><tr>
          <th>Epoch</th><th>Age</th><th>Type</th><th>Account</th><th>Amount</th>
        </tr></thead>
        <tbody>${entries.map(e => {
    const acct = e.to || e.from || e.account || "—";
    return `<tr>
            <td><a class="link" onclick="navigate('#block/${e.epoch}')">${fmtInt(e.epoch)}</a></td>
            <td style="color:var(--text-dim)">${fmtAge(e._ts)}</td>
            <td>${entryTypeBadge(e.type)}</td>
            <td><a class="link" onclick="navigate('#account/${escHtml(acct)}')">${escHtml(acct)}</a></td>
            <td>${e.amount != null ? fmt(e.amount, 4) + " BTCPC" : "—"}</td>
          </tr>`;
  }).join("")}
        </tbody>
      </table>
    </div>
  `);
}

function renderSupplyBar(s) { /* handled inline in renderStats */ }

// ─── Blocks list ──────────────────────────────────────────────────────────────

async function loadBlocks() {
  loading("blocks-table");
  try {
    const data = await apiFetch("/blocks?limit=100");
    const blocks = data.blocks || [];
    if (!blocks.length) { empty("blocks-table", "No blocks indexed."); return; }
    setHTML("blocks-table", `
      <div style="font-size:12px;color:var(--text-dim);margin-bottom:10px;">
        Showing ${blocks.length} of ${fmtInt(data.total_indexed)} indexed epochs
      </div>
      <div class="tbl-wrap">
        <table>
          <thead><tr>
            <th>Epoch</th><th>Timestamp</th><th>Miner</th><th>Entries</th><th>Types</th><th>Reward</th>
          </tr></thead>
          <tbody>${blocks.map(b => `
            <tr>
              <td><a class="link" onclick="navigate('#block/${b.epoch}')">${fmtInt(b.epoch)}</a></td>
              <td style="color:var(--text-dim);font-size:12px">${fmtDate(b.timestamp)}</td>
              <td><a class="link" onclick="navigate('#account/${escHtml(b.miner || b.proposer || '')}')">${escHtml(b.miner || b.proposer || "—")}</a></td>
              <td>${b.entry_count}</td>
              <td style="font-size:11px">${(b.entry_types || []).map(t => entryTypeBadge(t)).join(" ")}</td>
              <td>${b.reward != null ? fmt(b.reward, 2) + " BTCPC" : "—"}</td>
            </tr>`).join("")}
          </tbody>
        </table>
      </div>
    `);
  } catch (err) {
    errMsg("blocks-table", err.message);
  }
}

// ─── Block detail ─────────────────────────────────────────────────────────────

async function loadBlockDetail(num) {
  loading("block-detail-content");
  try {
    const b = await apiFetch(`/block/${num}`);
    const entries = b.ledger_entries || [];
    setHTML("block-detail-content", `
      <button class="back-btn" onclick="history.back()">← Back</button>
      <div class="page-header">
        <h2>Epoch ${fmtInt(b.epoch)}</h2>
        <span class="sub">${fmtDate(b.timestamp)}</span>
      </div>
      <div class="card">
        <div class="card-title">Block Header</div>
        <div class="detail-grid">
          <span class="dk">Epoch</span>
          <span class="dv">${fmtInt(b.epoch)}</span>
          <span class="dk">Timestamp</span>
          <span class="dv">${fmtDate(b.timestamp)}</span>
          <span class="dk">Proposer</span>
          <span class="dv"><a class="link" onclick="navigate('#account/${escHtml(b.proposer || '')}')">${escHtml(b.proposer || "—")}</a></span>
          <span class="dk">Hash</span>
          <span class="dv hash">${escHtml(b.hash || "—")}</span>
          <span class="dk">Prev Hash</span>
          <span class="dv hash">${escHtml(b.previous_hash || "—")}</span>
          <span class="dk">State Root</span>
          <span class="dv hash">${escHtml(b.state_root || "—")}</span>
          <span class="dk">Difficulty</span>
          <span class="dv">${b.difficulty != null ? b.difficulty : "—"}</span>
          <span class="dk">Entry Count</span>
          <span class="dv">${b.entry_count}</span>
        </div>
      </div>
      <div class="card">
        <div class="card-title">Ledger Entries (${entries.length})</div>
        ${entries.length === 0 ? '<div class="empty-msg">No entries in this block.</div>' : `
        <div class="tbl-wrap">
          <table>
            <thead><tr><th>Type</th><th>From</th><th>To / Account</th><th>Amount</th><th>Memo</th></tr></thead>
            <tbody>${entries.map(e => `
              <tr>
                <td>${entryTypeBadge(e.type)}</td>
                <td>${e.from ? `<a class="link" onclick="navigate('#account/${escHtml(e.from)}')">${escHtml(e.from)}</a>` : "—"}</td>
                <td>${(e.to || e.account) ? `<a class="link" onclick="navigate('#account/${escHtml(e.to || e.account)}')">${escHtml(e.to || e.account)}</a>` : "—"}</td>
                <td>${e.amount != null ? fmt(e.amount, 8) + " BTCPC" : "—"}</td>
                <td style="color:var(--text-dim);font-size:11px">${escHtml(e.memo || "")}</td>
              </tr>`).join("")}
            </tbody>
          </table>
        </div>`}
      </div>
    `);
  } catch (err) {
    errMsg("block-detail-content", err.message);
  }
}

// ─── Account ──────────────────────────────────────────────────────────────────

async function loadAccount(name) {
  loading("account-content");
  try {
    const [acct, hist] = await Promise.all([
      apiFetch(`/account/${encodeURIComponent(name)}`),
      apiFetch(`/account/${encodeURIComponent(name)}/history?limit=50`),
    ]);
    const history = hist.history || [];

    setHTML("account-content", `
      <button class="back-btn" onclick="history.back()">← Back</button>
      <div class="page-header">
        <h2 class="mono">${escHtml(acct.account)}</h2>
        ${acct.node_roles && acct.node_roles.length ? `<span>${acct.node_roles.map(badgeRole).join(" ")}</span>` : ""}
      </div>

      <div class="stat-grid">
        <div class="stat-card">
          <div class="stat-label">Balance</div>
          <div class="stat-value orange">${fmt(acct.balance, 4)}</div>
          <div class="stat-sub">BTCPC</div>
        </div>
        <div class="stat-card">
          <div class="stat-label">Staked</div>
          <div class="stat-value">${fmt(acct.staked || 0, 4)}</div>
          <div class="stat-sub">BTCPC</div>
        </div>
        <div class="stat-card">
          <div class="stat-label">Epochs Active (last 100)</div>
          <div class="stat-value green">${fmtInt(acct.epochs_active_last_100)}</div>
        </div>
        <div class="stat-card">
          <div class="stat-label">Rewards (last 100 ep)</div>
          <div class="stat-value">${fmt(acct.rewards_last_100_epochs, 4)}</div>
          <div class="stat-sub">BTCPC</div>
        </div>
      </div>

      <div class="card">
        <div class="card-title">Account Info</div>
        <div class="detail-grid">
          <span class="dk">Account</span>
          <span class="dv mono">${escHtml(acct.account)}</span>
          <span class="dk">Nonce</span>
          <span class="dv">${acct.nonce || 0}</span>
          <span class="dk">Created Epoch</span>
          <span class="dv"><a class="link" onclick="navigate('#block/${acct.created_epoch}')">${fmtInt(acct.created_epoch)}</a></span>
          <span class="dk">Created At</span>
          <span class="dv">${fmtDate(acct.created_at)}</span>
          <span class="dk">Last Active</span>
          <span class="dv">${acct.last_active_at ? fmtDate(acct.last_active_at) + ` (epoch ${fmtInt(acct.last_active_epoch)})` : "—"}</span>
          <span class="dk">Roles</span>
          <span class="dv">${acct.node_roles && acct.node_roles.length ? acct.node_roles.map(badgeRole).join(" ") : "—"}</span>
        </div>
      </div>

      <div class="card">
        <div class="card-title">Transaction History (${history.length})</div>
        ${history.length === 0 ? '<div class="empty-msg">No history found.</div>' : `
        <div class="tbl-wrap">
          <table>
            <thead><tr><th>Epoch</th><th>Age</th><th>Type</th><th>From</th><th>To</th><th>Amount</th></tr></thead>
            <tbody>${history.map(e => `
              <tr>
                <td><a class="link" onclick="navigate('#block/${e.epoch}')">${fmtInt(e.epoch)}</a></td>
                <td style="color:var(--text-dim);font-size:12px">${fmtAge(e._ts)}</td>
                <td>${entryTypeBadge(e.type)}</td>
                <td>${e.from ? `<a class="link" onclick="navigate('#account/${escHtml(e.from)}')">${escHtml(e.from)}</a>` : "—"}</td>
                <td>${(e.to || e.account) ? `<a class="link" onclick="navigate('#account/${escHtml(e.to || e.account)}')">${escHtml(e.to || e.account)}</a>` : "—"}</td>
                <td>${e.amount != null ? fmt(e.amount, 8) + " BTCPC" : "—"}</td>
              </tr>`).join("")}
            </tbody>
          </table>
        </div>`}
      </div>
    `);
  } catch (err) {
    errMsg("account-content", err.message);
  }
}

// ─── Accounts list ────────────────────────────────────────────────────────────

async function loadAccounts() {
  loading("accounts-table");
  try {
    const data = await apiFetch("/accounts");
    const accounts = data.accounts || [];
    if (!accounts.length) { empty("accounts-table", "No accounts found."); return; }
    setHTML("accounts-table", `
      <div style="font-size:12px;color:var(--text-dim);margin-bottom:10px;">
        ${fmtInt(accounts.length)} accounts
      </div>
      <div class="tbl-wrap">
        <table>
          <thead><tr>
            <th>#</th><th>Account</th><th>Balance</th><th>Staked</th><th>Nonce</th><th>Created Epoch</th>
          </tr></thead>
          <tbody>${accounts.map((a, i) => `
            <tr>
              <td style="color:var(--text-dim)">${i + 1}</td>
              <td><a class="link" onclick="navigate('#account/${escHtml(a.account)}')">${escHtml(a.account)}</a></td>
              <td class="mono">${fmt(a.balance, 4)} BTCPC</td>
              <td>${fmt(a.staked || 0, 4)}</td>
              <td>${a.nonce || 0}</td>
              <td><a class="link" onclick="navigate('#block/${a.created_epoch}')">${fmtInt(a.created_epoch)}</a></td>
            </tr>`).join("")}
          </tbody>
        </table>
      </div>
    `);
  } catch (err) {
    errMsg("accounts-table", err.message);
  }
}

// ─── Sensors ─────────────────────────────────────────────────────────────────

let allSensors = [];

async function loadSensors() {
  loading("sensors-table");
  try {
    const data = await apiFetch("/api/sensors");
    allSensors = data.sensors || data || [];
    renderSensors(allSensors);
    buildSensorFilters();
  } catch (err) {
    // Sensors may not be deployed — show graceful state
    setHTML("sensors-table", `
      <div class="empty-msg">
        Sensor data market not yet active — sensors will appear here once registered on-chain.
      </div>
    `);
  }
}

function buildSensorFilters() {
  const types = [...new Set(allSensors.map(s => s.sensor_type || s.type).filter(Boolean))];
  const regions = [...new Set(allSensors.map(s => s.region).filter(Boolean))];
  const typeOpts = `<option value="">All types</option>${types.map(t => `<option>${escHtml(t)}</option>`).join("")}`;
  const regionOpts = `<option value="">All regions</option>${regions.map(r => `<option>${escHtml(r)}</option>`).join("")}`;
  setHTML("sensor-filters", `
    <div class="filter-bar">
      <select id="filter-type" onchange="filterSensors()">${typeOpts}</select>
      <select id="filter-region" onchange="filterSensors()">${regionOpts}</select>
      <select id="filter-status" onchange="filterSensors()">
        <option value="">All status</option>
        <option>active</option><option>retired</option>
      </select>
      <input id="filter-search" type="text" placeholder="Search sensor ID or owner…" oninput="filterSensors()" style="flex:1;min-width:160px;">
    </div>
  `);
}

function filterSensors() {
  const type   = (el("filter-type")   || {}).value || "";
  const region = (el("filter-region") || {}).value || "";
  const status = (el("filter-status") || {}).value || "";
  const q      = ((el("filter-search") || {}).value || "").toLowerCase();

  const filtered = allSensors.filter(s => {
    if (type   && (s.sensor_type || s.type) !== type) return false;
    if (region && s.region !== region) return false;
    if (status && (s.status || "active") !== status) return false;
    if (q) {
      const hay = `${s.sensor_id || s.id || ""} ${s.owner || ""}`.toLowerCase();
      if (!hay.includes(q)) return false;
    }
    return true;
  });
  renderSensors(filtered);
}

function renderSensors(sensors) {
  if (!sensors.length) { empty("sensors-table", "No sensors match the current filters."); return; }
  setHTML("sensors-table", `
    <div class="tbl-wrap">
      <table>
        <thead><tr>
          <th>Sensor ID</th><th>Type</th><th>Region</th><th>Owner</th><th>Status</th><th>Last Seen</th>
        </tr></thead>
        <tbody>${sensors.map(s => {
    const id = s.sensor_id || s.id || "?";
    const status = s.status || "active";
    return `<tr>
            <td><a class="link" onclick="navigate('#sensor/${encodeURIComponent(id)}')">${escHtml(id)}</a></td>
            <td>${escHtml(s.sensor_type || s.type || "—")}</td>
            <td>${escHtml(s.region || "—")}</td>
            <td><a class="link" onclick="navigate('#account/${escHtml(s.owner || '')}')">${escHtml(s.owner || "—")}</a></td>
            <td><span class="badge ${status === "active" ? "badge-active" : "badge-retired"}">${status}</span></td>
            <td style="color:var(--text-dim);font-size:12px">${s.last_seen ? fmtAge(s.last_seen) : "—"}</td>
          </tr>`;
  }).join("")}
        </tbody>
      </table>
    </div>
  `);
}

// ─── Sensor detail ────────────────────────────────────────────────────────────

async function loadSensorDetail(id) {
  loading("sensor-detail-content");
  try {
    const s = await apiFetch(`/api/sensors/${encodeURIComponent(id)}`);
    const sensor = s.sensor || s;
    const readings = s.readings || sensor.recent_readings || [];
    const status = sensor.status || "active";

    setHTML("sensor-detail-content", `
      <button class="back-btn" onclick="navigate('#sensors')">← Sensors</button>
      <div class="page-header">
        <h2 class="mono">${escHtml(sensor.sensor_id || sensor.id || id)}</h2>
        <span class="badge ${status === "active" ? "badge-active" : "badge-retired"}">${status}</span>
      </div>

      <div class="card">
        <div class="card-title">Sensor Info</div>
        <div class="detail-grid">
          <span class="dk">ID</span>
          <span class="dv mono">${escHtml(sensor.sensor_id || sensor.id || "—")}</span>
          <span class="dk">Type</span>
          <span class="dv">${escHtml(sensor.sensor_type || sensor.type || "—")}</span>
          <span class="dk">Owner</span>
          <span class="dv"><a class="link" onclick="navigate('#account/${escHtml(sensor.owner || '')}')">${escHtml(sensor.owner || "—")}</a></span>
          <span class="dk">Region</span>
          <span class="dv">${escHtml(sensor.region || "—")}</span>
          <span class="dk">Status</span>
          <span class="dv">${status}</span>
          <span class="dk">Registered</span>
          <span class="dv">${sensor.registered_at ? fmtDate(sensor.registered_at) : "—"}</span>
          <span class="dk">Last Seen</span>
          <span class="dv">${sensor.last_seen ? fmtDate(sensor.last_seen) + ` (${fmtAge(sensor.last_seen)})` : "—"}</span>
          <span class="dk">Reading Count</span>
          <span class="dv">${fmtInt(sensor.reading_count || 0)}</span>
        </div>
      </div>

      ${readings.length ? `
      <div class="card">
        <div class="card-title">Recent Readings</div>
        <div class="tbl-wrap">
          <table>
            <thead><tr><th>Timestamp</th><th>Value</th><th>Unit</th><th>Epoch</th></tr></thead>
            <tbody>${readings.map(r => `
              <tr>
                <td style="font-size:12px;color:var(--text-dim)">${fmtDate(r.timestamp || r.ts)}</td>
                <td class="mono">${escHtml(String(r.value != null ? r.value : r.data || "—"))}</td>
                <td style="color:var(--text-dim)">${escHtml(r.unit || "")}</td>
                <td>${r.epoch != null ? `<a class="link" onclick="navigate('#block/${r.epoch}')">${fmtInt(r.epoch)}</a>` : "—"}</td>
              </tr>`).join("")}
            </tbody>
          </table>
        </div>
      </div>` : ""}
    `);
  } catch (err) {
    errMsg("sensor-detail-content", err.message);
  }
}

// ─── Search ────────────────────────────────────────────────────────────────────

async function doSearch() {
  const raw = (el("search-input").value || "").trim();
  if (!raw) return;

  // Block number?
  if (/^\d+$/.test(raw)) {
    navigate(`#block/${raw}`);
    el("search-input").value = "";
    return;
  }

  // Account or sensor?
  if (raw.includes("/")) {
    navigate(`#sensor/${encodeURIComponent(raw)}`);
    el("search-input").value = "";
    return;
  }

  // Try account first
  try {
    const data = await apiFetch(`/account/${encodeURIComponent(raw)}`);
    if (data && data.account) {
      navigate(`#account/${raw}`);
      el("search-input").value = "";
      return;
    }
  } catch (_) {}

  alert(`No result found for: "${raw}"`);
}

// ─── Init ────────────────────────────────────────────────────────────────────

window.addEventListener("hashchange", handleRoute);
document.addEventListener("DOMContentLoaded", () => {
  el("search-btn").addEventListener("click", doSearch);
  el("search-input").addEventListener("keydown", e => {
    if (e.key === "Enter") doSearch();
  });
  handleRoute();
});

// expose globally for inline onclick
window.navigate = navigate;
window.filterSensors = filterSensors;
