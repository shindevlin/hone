"use strict";

var layout = require("./layout");
var { escapeHtml } = require("./escape");

function settingsView(status) {
  var modeClass = status.mode === "full" ? "status-active" :
    status.mode === "reduced" ? "status-committed" : "status-finalized";

  var idleText = status.userIdle ? "Idle" : "Active";
  var idleTime = status.idleTimeMs ? Math.round(status.idleTimeMs / 1000) + "s" : "N/A";

  var content = `
    <h1 class="page-title">Miner <span>Settings</span></h1>

    <div class="stats-grid">
      <div class="stat-card">
        <div class="stat-label">Mining Mode</div>
        <div class="stat-value"><span class="${modeClass}">${escapeHtml(status.mode)}</span></div>
        <div class="stat-sub">${status.manualOverride ? "manual override" : "auto"}</div>
      </div>
      <div class="stat-card">
        <div class="stat-label">PC Status</div>
        <div class="stat-value">${idleText}</div>
        <div class="stat-sub">Idle time: ${idleTime}</div>
      </div>
      <div class="stat-card">
        <div class="stat-label">Work Items</div>
        <div class="stat-value">${status.workItems} / ${status.fullWorkItems}</div>
        <div class="stat-sub">per epoch</div>
      </div>
      <div class="stat-card">
        <div class="stat-label">Schedule</div>
        <div class="stat-value" style="font-size: 16px;">${escapeHtml(status.schedule || "None")}</div>
        <div class="stat-sub">${status.inScheduledReduction ? "reduced hours active" : "full speed"}</div>
      </div>
    </div>

    <div class="two-col">
      <div class="card">
        <div class="card-header">
          <h2>Mining Control</h2>
        </div>
        <div class="card-body" style="padding: 20px;">
          <form method="POST" action="/settings" style="display: flex; gap: 10px; flex-wrap: wrap;">
            <button type="submit" name="action" value="full" style="background: var(--accent); color: #000; border: none; padding: 10px 20px; border-radius: 6px; cursor: pointer; font-weight: 600; ${status.mode === 'full' ? 'outline: 2px solid #fff;' : ''}">
              Full Speed
            </button>
            <button type="submit" name="action" value="reduced" style="background: #3b82f6; color: #fff; border: none; padding: 10px 20px; border-radius: 6px; cursor: pointer; font-weight: 600; ${status.mode === 'reduced' ? 'outline: 2px solid #fff;' : ''}">
              Reduced
            </button>
            <button type="submit" name="action" value="pause" style="background: #ef4444; color: #fff; border: none; padding: 10px 20px; border-radius: 6px; cursor: pointer; font-weight: 600; ${status.mode === 'paused' ? 'outline: 2px solid #fff;' : ''}">
              Pause
            </button>
            <button type="submit" name="action" value="auto" style="background: var(--bg-card); color: var(--text-primary); border: 1px solid var(--border); padding: 10px 20px; border-radius: 6px; cursor: pointer; font-weight: 600; ${!status.manualOverride ? 'outline: 2px solid var(--accent);' : ''}">
              Auto (idle detect)
            </button>
          </form>
          <p style="margin-top: 12px; color: var(--text-secondary); font-size: 13px;">
            Auto: full speed when you're away, reduced when using your PC.
          </p>
        </div>
      </div>

      <div class="card">
        <div class="card-header">
          <h2>Resource Caps</h2>
        </div>
        <div class="card-body" style="padding: 20px;">
          <form method="POST" action="/settings">
            <div style="margin-bottom: 16px;">
              <label style="display: block; margin-bottom: 6px; font-weight: 600;">
                CPU: <span id="cpuVal">${status.cpuCap}%</span>
              </label>
              <input type="range" name="cpuCap" min="10" max="100" step="10" value="${status.cpuCap}"
                oninput="document.getElementById('cpuVal').textContent=this.value+'%'"
                style="width: 100%; accent-color: var(--accent);">
              <div style="display: flex; justify-content: space-between; font-size: 11px; color: var(--text-secondary);">
                <span>10%</span><span>50%</span><span>100%</span>
              </div>
            </div>
            <div style="margin-bottom: 16px;">
              <label style="display: block; margin-bottom: 6px; font-weight: 600;">
                GPU: <span id="gpuVal">${status.gpuCap}%</span>
              </label>
              <input type="range" name="gpuCap" min="0" max="100" step="10" value="${status.gpuCap}"
                oninput="document.getElementById('gpuVal').textContent=this.value+'%'"
                style="width: 100%; accent-color: var(--accent);">
              <div style="display: flex; justify-content: space-between; font-size: 11px; color: var(--text-secondary);">
                <span>Off</span><span>50%</span><span>100%</span>
              </div>
            </div>
            <button type="submit" style="background: var(--accent); color: #000; border: none; padding: 8px 20px; border-radius: 6px; cursor: pointer; font-weight: 600;">
              Apply
            </button>
          </form>
        </div>
      </div>
    </div>

    <div class="card" style="margin-top: 20px;">
      <div class="card-header">
        <h2>How It Works</h2>
      </div>
      <div class="card-body" style="padding: 20px; color: var(--text-secondary); font-size: 14px; line-height: 1.6;">
        <p><strong>Auto mode</strong> detects when you're using your PC (mouse/keyboard activity). Mining runs at full speed when you're away, and reduces automatically when you're active.</p>
        <p style="margin-top: 8px;"><strong>CPU cap</strong> limits the number of CPU threads used for inference. At 50%, only half your cores work on mining.</p>
        <p style="margin-top: 8px;"><strong>GPU cap</strong> controls how many model layers are offloaded to GPU. At 0%, mining runs on CPU only — your GPU is free for gaming or other work.</p>
        <p style="margin-top: 8px;"><strong>Schedule</strong> can be set via environment variable: <code>BTCPC_REDUCED_HOURS=09:00-17:00</code> to auto-reduce during work hours.</p>
      </div>
    </div>
  `;

  return layout("Miner Settings", content);
}

module.exports = settingsView;
