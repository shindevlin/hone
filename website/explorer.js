/* btcpcscan — blockchain explorer logic */
/* Rewired for Rust node on port 4242 — May 2026 */
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
  const cls = type.includes("Mine") || type.includes("MINING") ? "badge-miner"
    : type.includes("Storage") || type.includes("STORAGE") ? "badge-storage"
    : type.includes("Clock") || type.includes("CLOCK") ? "badge-clock"
    : type.includes("Sensor") || type.includes("SENSOR") ? "badge-sensor"
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
    // Rust node routes: /api/explorer/status, /api/explorer/blocks, /api/explorer/activity
    const [status, blocks, activity] = await Promise.all([
      apiFetch("/api/explorer/status"),
      apiFetch("/api/explorer/blocks?limit=10"),
      apiFetch("/api/explorer/activity?limit=20"),
    ]);
    renderStats(status);
    renderRecentBlocks(blocks.blocks || []);
    renderRecentActivity(activity.entries || []);
  } catch (err) {
    errMsg("stats-grid", err.message);
  }
}

function renderStats(s) {
  // Rust node /api/explorer/status fields:
  //   chain_height, current_epoch, epoch_ms, accounts,
  //   circulating_btcpc, max_supply_btcpc,
  //   active_nodes_last_100, miners, clock_nodes, storage_nodes
  const epochSecs = ((s.epoch_ms || 30000) / 1000).toFixed(0);
  setHTML("stats-grid", `
    <div class="stat-card">
      <div class="stat-label">Block Height</div>
      <div class="stat-value orange">${fmtInt(s.chain_height)}</div>
    </div>
    <div class="stat-card">
      <div class="stat-label">Epoch Time</div>
      <div class="stat-value">${epochSecs}s</div>
    </div>
    <div class="stat-card">
      <div class="stat-label">Active Nodes (last 100)</div>
      <div class="stat-value green">${fmtInt(s.active_nodes_last_100)}</div>
      <div class="stat-sub">${s.miners || 0} miners · ${s.storage_nodes || 0} storage · ${s.clock_nodes || 0} clocks</div>
    </div>
    <div class="stat-card">
      <div class="stat-label">Accounts</div>
      <div class="stat-value">${fmtInt(s.accounts)}</div>
    </div>
    <div class="stat-card">
      <div class="stat-label">Circulating Supply</div>
      <div class="stat-value mono">${fmt(s.circulating_btcpc, 2)}</div>
      <div class="stat-sub">of ${fmtInt(s.max_supply_btcpc || 42000000)} BTCPC max</div>
    </div>
  `);

  const circ = s.circulating_btcpc || 0;
  const maxS = s.max_supply_btcpc || 42000000;
  setHTML("supply-bar-section", `
    <div class="card">
      <div class="card-title">Supply Distribution</div>
      <div style="display:flex;justify-content:space-between;font-size:12px;margin-bottom:6px;">
        <span>Circulating: <strong>${fmt(circ, 4)} BTCPC</strong></span>
        <span style="color:var(--text-dim)">Max: ${fmtInt(maxS)} BTCPC</span>
      </div>
      <div class="supply-bar-wrap">
        <div class="supply-bar" id="supply-bar" style="width:${Math.min(100, (circ / maxS) * 100).toFixed(3)}%"></div>
      </div>
      <div style="font-size:11px;color:var(--text-dim);margin-top:6px;">
        ${((circ / maxS) * 100).toFixed(4)}% of max supply issued
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
          <th>Epoch</th><th>Age</th><th>Miner</th><th>Entries</th>
        </tr></thead>
        <tbody>${blocks.map(b => `
          <tr>
            <td><a class="link" onclick="navigate('#block/${b.epoch}')">${fmtInt(b.epoch)}</a></td>
            <td style="color:var(--text-dim)">${fmtAge(b.timestamp_ms)}</td>
            <td><a class="link" onclick="navigate('#account/${escHtml(b.miner || '')}')">${escHtml(b.miner || "—")}</a></td>
            <td>${b.entry_count}</td>
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
          <th>Epoch</th><th>Type</th><th>Account</th><th>Amount</th>
        </tr></thead>
        <tbody>${entries.map(e => {
    // Rust entries have a "type" field with the LedgerEntry variant name
    // and various field names depending on the entry type
    const acct = e.to || e.from || e.account || "—";
    const epoch = e.epoch || e._epoch || "—";
    return `<tr>
            <td><a class="link" onclick="navigate('#block/${epoch}')">${fmtInt(epoch)}</a></td>
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

// ─── Blocks list ──────────────────────────────────────────────────────────────

async function loadBlocks() {
  loading("blocks-table");
  try {
    // Rust: GET /api/explorer/blocks?limit=100
    // Returns: { blocks: [{epoch, hash, timestamp_ms, entry_count, entry_types, miner, status}], count, chain_height }
    const data = await apiFetch("/api/explorer/blocks?limit=100");
    const blocks = data.blocks || [];
    if (!blocks.length) { empty("blocks-table", "No blocks indexed."); return; }
    setHTML("blocks-table", `
      <div style="font-size:12px;color:var(--text-dim);margin-bottom:10px;">
        Showing ${blocks.length} blocks — chain height ${fmtInt(data.chain_height)}
      </div>
      <div class="tbl-wrap">
        <table>
          <thead><tr>
            <th>Epoch</th><th>Timestamp</th><th>Miner</th><th>Entries</th><th>Types</th><th>Status</th>
          </tr></thead>
          <tbody>${blocks.map(b => `
            <tr>
              <td><a class="link" onclick="navigate('#block/${b.epoch}')">${fmtInt(b.epoch)}</a></td>
              <td style="color:var(--text-dim);font-size:12px">${fmtDate(b.timestamp_ms)}</td>
              <td><a class="link" onclick="navigate('#account/${escHtml(b.miner || '')}')">${escHtml(b.miner || "—")}</a></td>
              <td>${b.entry_count}</td>
              <td style="font-size:11px">${(b.entry_types || []).map(t => entryTypeBadge(t)).join(" ")}</td>
              <td style="font-size:11px;color:var(--text-dim)">${b.status || "—"}</td>
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
    // Rust: GET /api/block/:epoch
    // Returns: { epoch, status, hash, header: {...}, payload: { ledger_entries: [...] } }
    const b = await apiFetch(`/api/block/${num}`);
    const header = b.header || {};
    const payload = b.payload || {};
    const entries = payload.ledger_entries || [];
    setHTML("block-detail-content", `
      <button class="back-btn" onclick="history.back()">← Back</button>
      <div class="page-header">
        <h2>Epoch ${fmtInt(b.epoch)}</h2>
        <span class="sub">${fmtDate(header.timestamp_ms)}</span>
      </div>
      <div class="card">
        <div class="card-title">Block Header</div>
        <div class="detail-grid">
          <span class="dk">Epoch</span>
          <span class="dv">${fmtInt(b.epoch)}</span>
          <span class="dk">Status</span>
          <span class="dv">${escHtml(b.status || "—")}</span>
          <span class="dk">Hash</span>
          <span class="dv hash">${escHtml(b.hash || "—")}</span>
          <span class="dk">Prev Hash</span>
          <span class="dv hash">${escHtml(header.previous_hash || "—")}</span>
          <span class="dk">State Root</span>
          <span class="dv hash">${escHtml(header.state_root || "—")}</span>
          <span class="dk">Entry Count</span>
          <span class="dv">${entries.length}</span>
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
    // Rust: GET /api/account/:account
    // Returns: { account, created_epoch, keys, nonce, stake, chains_proven, key_policies }
    // Rust: GET /api/account/:account/history
    // Returns: { account, count, entries: [{type, from/to/account, amount, memo, _epoch, _role}] }
    // Rust: GET /api/balance/:account
    // Returns: { account, balance (float), dreams, token }
    const [acct, hist, balResp] = await Promise.all([
      apiFetch(`/api/account/${encodeURIComponent(name)}`),
      apiFetch(`/api/account/${encodeURIComponent(name)}/history?limit=50`),
      apiFetch(`/api/balance/${encodeURIComponent(name)}`),
    ]);
    const history = hist.entries || [];
    const balance = balResp.balance || 0;
    const stake = acct.stake || 0;

    setHTML("account-content", `
      <button class="back-btn" onclick="history.back()">← Back</button>
      <div class="page-header">
        <h2 class="mono">${escHtml(acct.account)}</h2>
      </div>

      <div class="stat-grid">
        <div class="stat-card">
          <div class="stat-label">Balance</div>
          <div class="stat-value orange">${fmt(balance, 4)}</div>
          <div class="stat-sub">BTCPC</div>
        </div>
        <div class="stat-card">
          <div class="stat-label">Staked</div>
          <div class="stat-value">${fmt(stake, 4)}</div>
          <div class="stat-sub">BTCPC</div>
        </div>
        <div class="stat-card">
          <div class="stat-label">Nonce</div>
          <div class="stat-value">${fmtInt(acct.nonce || 0)}</div>
        </div>
        <div class="stat-card">
          <div class="stat-label">Tx History</div>
          <div class="stat-value">${fmtInt(hist.count || 0)}</div>
          <div class="stat-sub">entries</div>
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
          <span class="dk">Stake</span>
          <span class="dv">${fmt(stake, 4)} BTCPC</span>
          ${(acct.chains_proven || []).length ? `
          <span class="dk">Proven Chains</span>
          <span class="dv">${(acct.chains_proven || []).map(c => escHtml(c.chain || c)).join(", ")}</span>
          ` : ""}
        </div>
      </div>

      <div class="card">
        <div class="card-title">Transaction History (${history.length})</div>
        ${history.length === 0 ? '<div class="empty-msg">No history found.</div>' : `
        <div class="tbl-wrap">
          <table>
            <thead><tr><th>Epoch</th><th>Type</th><th>From</th><th>To</th><th>Amount</th></tr></thead>
            <tbody>${history.map(e => {
    const epoch = e._epoch || e.epoch || "—";
    return `
              <tr>
                <td><a class="link" onclick="navigate('#block/${epoch}')">${fmtInt(epoch)}</a></td>
                <td>${entryTypeBadge(e.type)}</td>
                <td>${e.from ? `<a class="link" onclick="navigate('#account/${escHtml(e.from)}')">${escHtml(e.from)}</a>` : "—"}</td>
                <td>${(e.to || e.account) ? `<a class="link" onclick="navigate('#account/${escHtml(e.to || e.account)}')">${escHtml(e.to || e.account)}</a>` : "—"}</td>
                <td>${e.amount != null ? fmt(e.amount, 8) + " BTCPC" : "—"}</td>
              </tr>`;
  }).join("")}
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
    // Rust: GET /api/accounts
    // Returns: { count, accounts: [{account, keys, balances: {BTCPC: dreams_int}}] }
    const data = await apiFetch("/api/accounts");
    const accounts = data.accounts || [];
    if (!accounts.length) { empty("accounts-table", "No accounts found."); return; }

    // Convert dreams integer to BTCPC float (1 BTCPC = 10,000,000,000 dreams)
    const DREAMS_PER_BTCPC = 10_000_000_000;
    const rows = accounts.map((a, i) => {
      const dreamsRaw = (a.balances && a.balances.BTCPC) ? Number(a.balances.BTCPC) : 0;
      const btcpc = dreamsRaw / DREAMS_PER_BTCPC;
      return { account: a.account, btcpc, i };
    });
    // Sort by balance descending
    rows.sort((a, b) => b.btcpc - a.btcpc);

    setHTML("accounts-table", `
      <div style="font-size:12px;color:var(--text-dim);margin-bottom:10px;">
        ${fmtInt(accounts.length)} accounts
      </div>
      <div class="tbl-wrap">
        <table>
          <thead><tr>
            <th>#</th><th>Account</th><th>Balance (BTCPC)</th>
          </tr></thead>
          <tbody>${rows.map((a, i) => `
            <tr>
              <td style="color:var(--text-dim)">${i + 1}</td>
              <td><a class="link" onclick="navigate('#account/${escHtml(a.account)}')">${escHtml(a.account)}</a></td>
              <td class="mono">${fmt(a.btcpc, 4)}</td>
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
    // No sensor list endpoint exists yet in the Rust node.
    // Sensors are registered individually via /api/sensor/:id (GET) and /api/sensor/register (POST).
    // Show graceful empty state until a sensor index is added.
    throw new Error("sensor_list_unavailable");
  } catch (err) {
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
    // Rust: GET /api/sensor/:id
    // Returns sensor registration data stored on-chain
    const s = await apiFetch(`/api/sensor/${encodeURIComponent(id)}`);
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

  // Try account first — Rust: GET /api/account/:account
  try {
    const data = await apiFetch(`/api/account/${encodeURIComponent(raw)}`);
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
