"use strict";

const layout = require("./layout");

function visualizerView() {
  const content = `
    <style>
      .visualizer-shell {
        display: grid;
        gap: 20px;
      }

      .nervous-system {
        position: relative;
        overflow: hidden;
        min-height: 680px;
        border: 1px solid rgba(42, 53, 80, 0.9);
        border-radius: 22px;
        background:
          radial-gradient(circle at 18% 12%, rgba(34, 211, 238, 0.16), transparent 24%),
          radial-gradient(circle at 72% 18%, rgba(236, 72, 153, 0.12), transparent 22%),
          radial-gradient(circle at 50% 90%, rgba(247, 147, 26, 0.10), transparent 30%),
          linear-gradient(145deg, #030712 0%, #060912 42%, #0a0e17 100%);
        box-shadow: inset 0 1px 0 rgba(255,255,255,0.04), 0 28px 80px rgba(0,0,0,0.28);
      }

      .nervous-system::before {
        content: "";
        position: absolute;
        inset: 0;
        background-image:
          linear-gradient(rgba(148, 163, 184, 0.035) 1px, transparent 1px),
          linear-gradient(90deg, rgba(148, 163, 184, 0.035) 1px, transparent 1px);
        background-size: 42px 42px;
        mask-image: radial-gradient(circle at 50% 48%, black 0%, transparent 78%);
        pointer-events: none;
      }

      .global-stage {
        position: absolute;
        inset: 0;
      }

      #globalCanvas {
        width: 100%;
        height: 100%;
        display: block;
      }

      .nervous-overlay {
        position: absolute;
        inset: 0;
        pointer-events: none;
        display: grid;
        grid-template-columns: minmax(0, 1fr) 360px;
        gap: 20px;
        padding: 24px;
      }

      .nervous-copy {
        align-self: start;
        max-width: 620px;
      }

      .nervous-copy h1 {
        font-size: clamp(32px, 5.5vw, 72px);
        line-height: 0.95;
        letter-spacing: -2.5px;
        margin: 12px 0 16px;
        text-shadow: 0 0 34px rgba(34, 211, 238, 0.12);
      }

      .nervous-copy p {
        max-width: 520px;
        color: var(--text-secondary);
      }

      .perspective-switch {
        pointer-events: auto;
        display: inline-flex;
        gap: 8px;
        padding: 6px;
        border: 1px solid rgba(42, 53, 80, 0.9);
        border-radius: 999px;
        background: rgba(3, 7, 18, 0.72);
        backdrop-filter: blur(16px);
      }

      .perspective-switch button {
        border: 0;
        border-radius: 999px;
        padding: 8px 14px;
        background: transparent;
        color: var(--text-secondary);
        font: inherit;
        font-size: 12px;
        cursor: pointer;
      }

      .perspective-switch button.active {
        background: linear-gradient(135deg, rgba(34, 211, 238, 0.20), rgba(236, 72, 153, 0.18));
        color: var(--text-primary);
        box-shadow: 0 0 22px rgba(34, 211, 238, 0.18);
      }

      .legend-row {
        display: flex;
        flex-wrap: wrap;
        gap: 12px;
        margin-top: 20px;
        color: var(--text-secondary);
        font-size: 12px;
      }

      .legend-row span {
        display: inline-flex;
        align-items: center;
        gap: 8px;
        padding: 7px 10px;
        border: 1px solid rgba(42, 53, 80, 0.9);
        border-radius: 999px;
        background: rgba(3, 7, 18, 0.58);
        backdrop-filter: blur(12px);
      }

      .legend-dot {
        width: 9px;
        height: 9px;
        border-radius: 999px;
        box-shadow: 0 0 18px currentColor;
      }

      .ledger-panel {
        pointer-events: auto;
        align-self: stretch;
        display: grid;
        grid-template-rows: auto 1fr auto;
        min-height: 560px;
        border: 1px solid rgba(42, 53, 80, 0.9);
        border-radius: 18px;
        background: rgba(3, 7, 18, 0.70);
        backdrop-filter: blur(18px);
        overflow: hidden;
      }

      .ledger-panel header {
        padding: 16px;
        border-bottom: 1px solid rgba(42, 53, 80, 0.82);
      }

      .ledger-panel header strong {
        display: block;
        color: var(--text-primary);
      }

      .ledger-panel header span {
        color: var(--text-muted);
        font-size: 12px;
      }

      .ledger-waterfall {
        display: flex;
        flex-direction: column-reverse;
        gap: 8px;
        padding: 14px;
        overflow: hidden;
        font-size: 11px;
        color: #9ee7ff;
      }

      .ledger-event {
        display: grid;
        grid-template-columns: 54px 1fr auto;
        gap: 8px;
        align-items: center;
        padding: 9px 10px;
        border: 1px solid rgba(34, 211, 238, 0.15);
        border-radius: 12px;
        background: rgba(8, 13, 25, 0.82);
        animation: ledgerDrop 420ms ease-out both;
      }

      .ledger-event.storage {
        color: #67e8f9;
      }

      .ledger-event.inference {
        color: #f0abfc;
      }

      .ledger-event small {
        color: var(--text-muted);
      }

      .privacy-strip {
        padding: 12px 16px;
        border-top: 1px solid rgba(42, 53, 80, 0.82);
        color: var(--text-muted);
        font-size: 11px;
      }

      .visualizer-hero {
        position: relative;
        overflow: hidden;
        border: 1px solid var(--border);
        border-radius: 18px;
        padding: 28px;
        background:
          radial-gradient(circle at 12% 18%, rgba(247, 147, 26, 0.18), transparent 28%),
          radial-gradient(circle at 90% 15%, rgba(34, 197, 94, 0.11), transparent 24%),
          linear-gradient(135deg, #111827 0%, #0a0e17 58%, #15110a 100%);
      }

      .visualizer-hero h1 {
        font-size: clamp(26px, 4vw, 48px);
        line-height: 1.05;
        margin-bottom: 10px;
        letter-spacing: -1px;
      }

      .visualizer-hero p {
        max-width: 720px;
        color: var(--text-secondary);
      }

      .live-pill {
        display: inline-flex;
        align-items: center;
        gap: 8px;
        margin-bottom: 18px;
        padding: 6px 12px;
        border: 1px solid rgba(247, 147, 26, 0.34);
        border-radius: 999px;
        color: var(--accent);
        background: rgba(247, 147, 26, 0.08);
        font-size: 12px;
        text-transform: uppercase;
        letter-spacing: 1px;
      }

      .live-pill::before {
        content: "";
        width: 8px;
        height: 8px;
        border-radius: 999px;
        background: var(--green);
        box-shadow: 0 0 16px rgba(34, 197, 94, 0.9);
        animation: visualizerPulse 1.7s infinite;
      }

      .chain-flow {
        position: relative;
        display: grid;
        grid-template-columns: repeat(3, minmax(220px, 1fr));
        gap: 14px;
        margin-top: 24px;
      }

      .chain-flow::before {
        content: "";
        position: absolute;
        left: 24px;
        right: 24px;
        bottom: 26px;
        height: 2px;
        background: linear-gradient(90deg, transparent, var(--accent), var(--green), transparent);
        background-size: 220% 100%;
        opacity: 0.48;
        animation: streamRail 2.2s linear infinite;
      }

      .flow-node {
        min-height: 150px;
        position: relative;
        overflow: hidden;
        border: 1px solid var(--border);
        border-radius: 16px;
        padding: 16px;
        background: rgba(26, 34, 53, 0.78);
        box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.03);
        z-index: 1;
      }

      .flow-node.hot {
        border-color: rgba(247, 147, 26, 0.54);
        box-shadow: 0 0 24px rgba(247, 147, 26, 0.10), inset 0 1px 0 rgba(255, 255, 255, 0.04);
      }

      .flow-node.hot::before {
        content: "";
        position: absolute;
        inset: 0;
        background: linear-gradient(120deg, transparent 0%, rgba(247, 147, 26, 0.10) 45%, transparent 58%);
        transform: translateX(-100%);
        animation: liveSweep 2.8s ease-in-out infinite;
      }

      .flow-node::after {
        content: "";
        position: absolute;
        top: 20px;
        right: -18px;
        width: 36px;
        height: 2px;
        background: linear-gradient(90deg, var(--accent), transparent);
        opacity: 0.55;
      }

      .flow-node:last-child::after {
        display: none;
      }

      .node-kicker {
        color: var(--text-muted);
        font-size: 11px;
        text-transform: uppercase;
        letter-spacing: 1px;
        margin-bottom: 10px;
      }

      .node-value {
        font-size: 26px;
        font-weight: 800;
        color: var(--text-primary);
        overflow-wrap: anywhere;
      }

      .node-label {
        color: var(--accent);
        font-size: 13px;
        margin-top: 4px;
      }

      .node-detail {
        margin-top: 12px;
        color: var(--text-secondary);
        font-size: 12px;
        overflow-wrap: anywhere;
      }

      .node-bars {
        display: grid;
        gap: 8px;
        margin-top: 12px;
      }

      .node-bar {
        display: grid;
        grid-template-columns: 1fr auto;
        gap: 8px;
        color: var(--text-secondary);
        font-size: 12px;
      }

      .visualizer-grid {
        display: grid;
        grid-template-columns: 1.35fr 0.65fr;
        gap: 20px;
      }

      .anchor-list,
      .bridge-list {
        display: grid;
        gap: 10px;
        padding: 18px;
      }

      .anchor-row,
      .bridge-row {
        display: grid;
        grid-template-columns: 120px 1fr auto;
        gap: 12px;
        align-items: center;
        padding: 12px;
        border: 1px solid rgba(42, 53, 80, 0.7);
        border-radius: 12px;
        background: rgba(17, 24, 39, 0.62);
      }

      .anchor-row strong,
      .bridge-row strong {
        color: var(--text-primary);
      }

      .anchor-meta,
      .bridge-meta {
        color: var(--text-secondary);
        font-size: 12px;
        overflow-wrap: anywhere;
      }

      .visualizer-status {
        color: var(--text-muted);
        font-size: 12px;
      }

      .epoch-meter {
        margin-top: 14px;
        height: 7px;
        border-radius: 999px;
        overflow: hidden;
        background: rgba(10, 14, 23, 0.8);
      }

      .epoch-meter span {
        display: block;
        width: 0%;
        height: 100%;
        border-radius: inherit;
        background: linear-gradient(90deg, var(--accent), var(--green));
        transition: width 0.8s linear;
      }

      @keyframes visualizerPulse {
        0% { transform: scale(1); opacity: 1; }
        70% { transform: scale(1.7); opacity: 0.25; }
        100% { transform: scale(1); opacity: 1; }
      }

      @keyframes streamRail {
        from { background-position: 220% 0; }
        to { background-position: 0 0; }
      }

      @keyframes liveSweep {
        0% { transform: translateX(-100%); opacity: 0; }
        25% { opacity: 1; }
        65% { opacity: 1; }
        100% { transform: translateX(100%); opacity: 0; }
      }

      @keyframes ledgerDrop {
        from { opacity: 0; transform: translateY(-14px); }
        to { opacity: 1; transform: translateY(0); }
      }

      @media (max-width: 1100px) {
        .nervous-system { min-height: 920px; }
        .nervous-overlay {
          grid-template-columns: 1fr;
          grid-template-rows: auto 1fr;
        }
        .ledger-panel {
          min-height: 280px;
          align-self: end;
        }
        .chain-flow { grid-template-columns: repeat(2, 1fr); }
        .visualizer-grid { grid-template-columns: 1fr; }
      }

      @media (max-width: 640px) {
        .nervous-system { min-height: 960px; }
        .nervous-overlay { padding: 16px; }
        .nervous-copy h1 { letter-spacing: -1px; }
        .ledger-event { grid-template-columns: 1fr; }
        .visualizer-hero { padding: 20px; }
        .chain-flow { grid-template-columns: 1fr; }
        .anchor-row, .bridge-row { grid-template-columns: 1fr; }
      }
    </style>

    <div class="visualizer-shell">
      <section class="nervous-system">
        <div class="global-stage" id="globalStage">
          <canvas id="globalCanvas"></canvas>
        </div>
        <div class="nervous-overlay">
          <div class="nervous-copy">
            <div class="live-pill">The Global Nervous System</div>
            <h1>Useful work lighting up a borderless chain.</h1>
            <p>Inference bursts travel as magenta arcs. Storage persistence moves as slower cyan trails. Node positions resolve to metro-area buckets with jitter, not exact physical locations.</p>
            <div class="perspective-switch" aria-label="Perspective switch">
              <button type="button" class="active" id="globeMode">Globe View</button>
              <button type="button" id="mapMode">Command Map</button>
            </div>
            <div class="legend-row">
              <span><i class="legend-dot" style="color:#f0abfc;background:#f0abfc;"></i>Inference laser arcs</span>
              <span><i class="legend-dot" style="color:#67e8f9;background:#67e8f9;"></i>Storage liquid trails</span>
              <span><i class="legend-dot" style="color:#f7931a;background:#f7931a;"></i>Role hubs</span>
            </div>
          </div>
          <aside class="ledger-panel">
            <header>
              <strong>Live Ledger</strong>
              <span id="ledgerSubtitle">waiting for chain pulses</span>
            </header>
            <div class="ledger-waterfall" id="ledgerWaterfall"></div>
            <div class="privacy-strip">Coordinates use deterministic metro-sector placement, roughly 15-50km from the centroid, so nearby users do not render as neighbors. No exact node location is displayed.</div>
          </aside>
        </div>
      </section>

      <section class="visualizer-hero">
        <div class="live-pill">Live Chain Visualizer</div>
        <h1>HONE turns useful work into finalized chain state.</h1>
        <p>This page reads the live explorer API and shows the current epoch, mempool, compute, storage, sensors, finality anchors, and cross-chain bridge state in one flow.</p>
        <div class="chain-flow" id="chainFlow">
          <div class="flow-node"><div class="node-kicker">Loading</div><div class="node-value">...</div><div class="node-label">Waiting for chain state</div></div>
        </div>
      </section>

      <div class="visualizer-grid">
        <div class="card">
          <div class="card-header">
            <h2>External Finality</h2>
            <span class="badge" id="finalityBadge">polling</span>
          </div>
          <div class="anchor-list" id="anchorList"></div>
        </div>

        <div class="card">
          <div class="card-header">
            <h2>Bridge Liquidity</h2>
            <span class="badge" id="bridgeBadge">polling</span>
          </div>
          <div class="bridge-list" id="bridgeList"></div>
        </div>
      </div>

      <div class="visualizer-status" id="visualizerStatus">Starting live feed...</div>
    </div>

    <script>
      (function () {
        var chainFlow = document.getElementById("chainFlow");
        var anchorList = document.getElementById("anchorList");
        var bridgeList = document.getElementById("bridgeList");
        var status = document.getElementById("visualizerStatus");
        var finalityBadge = document.getElementById("finalityBadge");
        var bridgeBadge = document.getElementById("bridgeBadge");
        var lastData = null;
        var lastObservedAt = Date.now();
        var epochDurationMs = 30000;

        function fmt(value) {
          if (value === null || value === undefined || value === "") return "0";
          var num = Number(value);
          if (!Number.isFinite(num)) return String(value);
          return num.toLocaleString(undefined, { maximumFractionDigits: 4 });
        }

        function shortHash(value) {
          if (!value) return "not available";
          var str = String(value);
          return str.length > 18 ? str.slice(0, 14) + "..." : str;
        }

        function safeText(value) {
          return String(value === null || value === undefined ? "" : value)
            .replace(/&/g, "&amp;")
            .replace(/</g, "&lt;")
            .replace(/>/g, "&gt;")
            .replace(/"/g, "&quot;")
            .replace(/'/g, "&#39;");
        }

        function flowNode(kicker, value, label, detail) {
          return '<div class="flow-node hot">' +
            '<div class="node-kicker">' + safeText(kicker) + '</div>' +
            '<div class="node-value">' + safeText(value) + '</div>' +
            '<div class="node-label">' + safeText(label) + '</div>' +
            '<div class="node-detail">' + safeText(detail) + '</div>' +
          '</div>';
        }

        function workBars(work) {
          var rows = [
            ["Physical PCs", work.machines],
            ["Role points", work.role_points],
            ["Miners", work.miners],
            ["Verifiers", work.verifiers],
            ["Clocks", work.clocks],
            ["Storage", work.storage_hosts],
            ["Services", work.service_hosts],
            ["Sensors", work.sensors],
            ["Gateways", work.gateways]
          ];
          return '<div class="node-bars">' + rows.map(function (row) {
            return '<div class="node-bar"><span>' + safeText(row[0]) + '</span><strong>' + safeText(fmt(row[1] || 0)) + '</strong></div>';
          }).join("") + '</div>';
        }

        function multiRoleDetail(work) {
          var nodes = Array.isArray(work.multi_role_nodes) ? work.multi_role_nodes : [];
          if (!nodes.length) return "No multi-role PCs reported yet";
          return nodes.slice(0, 4).map(function (node) {
            return node.username + ": " + (node.roles || []).join(" + ");
          }).join(" | ");
        }

        function renderAnchors(data) {
          var anchors = (data.finality && data.finality.anchors) || {};
          var tiers = ["l2", "ethereum", "bitcoin"];
          finalityBadge.textContent = "native #" + fmt(data.finality ? data.finality.latest_native : 0);
          anchorList.innerHTML = tiers.map(function (tier) {
            var anchor = anchors[tier];
            var epoch = anchor && anchor.epoch_number !== undefined ? "#" + fmt(anchor.epoch_number) : "waiting";
            var proof = anchor && (anchor.batch_root || anchor.state_root || anchor.tx_hash || anchor.transaction_hash);
            return '<div class="anchor-row">' +
              '<strong>' + safeText(tier.toUpperCase()) + '</strong>' +
              '<div class="anchor-meta">epoch ' + safeText(epoch) + '<br>proof ' + safeText(shortHash(proof)) + '</div>' +
              '<span class="status status-' + (anchor ? "finalized" : "pending") + '">' + (anchor ? "anchored" : "pending") + '</span>' +
            '</div>';
          }).join("");
        }

        function renderBridge(data) {
          var chains = (data.bridge && data.bridge.chains) || [];
          bridgeBadge.textContent = fmt(chains.length) + " chain(s)";
          if (!chains.length) {
            bridgeList.innerHTML = '<div class="empty-state">No bridge chains registered yet</div>';
            return;
          }
          bridgeList.innerHTML = chains.map(function (chain) {
            return '<div class="bridge-row">' +
              '<strong>' + safeText(chain.name || chain.chain_id) + '</strong>' +
              '<div class="bridge-meta">locked ' + safeText(fmt(chain.total_locked_hone || 0)) + ' HONE<br>wrapped ' + safeText(fmt(chain.circulating_whone || 0)) + ' wHONE</div>' +
              '<span class="status status-finalized">live</span>' +
            '</div>';
          }).join("");
        }

        function render(data) {
          lastData = data;
          lastObservedAt = data.observed_at || Date.now();
          var epoch = data.epoch || {};
          var work = data.work || {};
          chainFlow.innerHTML =
            '<div class="flow-node hot"><div class="node-kicker">Epoch Clock</div><div class="node-value">#' + safeText(fmt(epoch.number || 0)) + '</div><div class="node-label">' + safeText(epoch.status || "starting") + '</div><div class="node-detail" id="epochTick">streaming epoch time</div><div class="epoch-meter"><span id="epochMeter"></span></div></div>' +
            flowNode("Mempool", fmt(epoch.mempool_size || 0), "pending spends", "transactions waiting for block inclusion") +
            '<div class="flow-node"><div class="node-kicker">Useful Work</div><div class="node-value">' + safeText(fmt(work.role_points || 0)) + '</div><div class="node-label">role instances, not PCs</div><div class="node-detail">' + safeText(multiRoleDetail(work)) + '</div>' + workBars(work) + '</div>' +
            flowNode("State Root", shortHash(epoch.state_root), "account state", "blocks on disk: " + fmt(epoch.blocks_on_disk || 0)) +
            flowNode("Sensors", fmt((work.sensors || 0) + (work.gateways || 0)), "IoT stream", fmt(work.sensors || 0) + " sensors, " + fmt(work.gateways || 0) + " gateways") +
            flowNode("Reward", fmt(epoch.reward_per_epoch || 0), "HONE per epoch", "constant reward through weekly doublings");
          renderAnchors(data);
          renderBridge(data);
          status.textContent = "Last update: " + new Date().toLocaleTimeString();
          updateHeartbeat();
        }

        function updateHeartbeat() {
          var now = Date.now();
          var tick = document.getElementById("epochTick");
          var meter = document.getElementById("epochMeter");
          if (!tick || !meter) return;
          var phase = now % epochDurationMs;
          var percent = Math.min(100, Math.max(0, (phase / epochDurationMs) * 100));
          var secondsLeft = Math.max(0, Math.ceil((epochDurationMs - phase) / 1000));
          var age = Math.max(0, Math.round((now - lastObservedAt) / 1000));
          tick.textContent = "stream age " + age + "s, next epoch window in ~" + secondsLeft + "s";
          meter.style.width = percent.toFixed(1) + "%";
        }

        function load() {
          fetch("/api/visualizer", { cache: "no-store" })
            .then(function (res) {
              if (!res.ok) throw new Error("HTTP " + res.status);
              return res.json();
            })
            .then(render)
            .catch(function (err) {
              status.textContent = "Visualizer feed unavailable: " + err.message;
            });
        }

        if (window.EventSource) {
          var source = new EventSource("/api/visualizer/stream");
          source.addEventListener("chain", function (event) {
            render(JSON.parse(event.data));
          });
          source.onerror = function () {
            status.textContent = "Streaming feed reconnecting...";
          };
        } else {
          load();
          setInterval(load, 5000);
        }
        setInterval(updateHeartbeat, 250);
      })();
    </script>
    <script type="module">
      import * as THREE from "https://cdn.jsdelivr.net/npm/three@0.164.1/build/three.module.js";

      const canvas = document.getElementById("globalCanvas");
      const stage = document.getElementById("globalStage");
      const ledger = document.getElementById("ledgerWaterfall");
      const ledgerSubtitle = document.getElementById("ledgerSubtitle");
      const globeMode = document.getElementById("globeMode");
      const mapMode = document.getElementById("mapMode");

      const scene = new THREE.Scene();
      const camera = new THREE.PerspectiveCamera(42, 1, 0.1, 100);
      camera.position.set(0, 0.38, 3.45);

      const renderer = new THREE.WebGLRenderer({ canvas, antialias: true, alpha: true, powerPreference: "high-performance" });
      renderer.setClearColor(0x000000, 0);
      renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));

      const root = new THREE.Group();
      const hubGroup = new THREE.Group();
      const pulseGroup = new THREE.Group();
      scene.add(root);
      root.add(hubGroup);
      root.add(pulseGroup);

      const globe = new THREE.Mesh(
        new THREE.SphereGeometry(1, 96, 64),
        new THREE.MeshBasicMaterial({ color: 0x07111f, transparent: true, opacity: 0.92 })
      );
      root.add(globe);

      const wire = new THREE.Mesh(
        new THREE.SphereGeometry(1.006, 72, 36),
        new THREE.MeshBasicMaterial({ color: 0x1f3a4b, wireframe: true, transparent: true, opacity: 0.12 })
      );
      root.add(wire);

      const atmosphere = new THREE.Mesh(
        new THREE.SphereGeometry(1.035, 96, 64),
        new THREE.MeshBasicMaterial({ color: 0x0ea5e9, transparent: true, opacity: 0.055, blending: THREE.AdditiveBlending })
      );
      root.add(atmosphere);

      const metros = [
        { name: "Tokyo", lat: 35.6762, lon: 139.6503 },
        { name: "Singapore", lat: 1.3521, lon: 103.8198 },
        { name: "London", lat: 51.5072, lon: -0.1276 },
        { name: "Frankfurt", lat: 50.1109, lon: 8.6821 },
        { name: "New York", lat: 40.7128, lon: -74.0060 },
        { name: "Los Angeles", lat: 34.0522, lon: -118.2437 },
        { name: "Sao Paulo", lat: -23.5558, lon: -46.6396 },
        { name: "Mexico City", lat: 19.4326, lon: -99.1332 },
        { name: "Lagos", lat: 6.5244, lon: 3.3792 },
        { name: "Dubai", lat: 25.2048, lon: 55.2708 },
        { name: "Mumbai", lat: 19.0760, lon: 72.8777 },
        { name: "Sydney", lat: -33.8688, lon: 151.2093 }
      ];

      let mode = "globe";
      let lastState = null;
      let hubs = [];
      let pulses = [];
      let rotationVelocity = 0.00085;
      let dragStart = null;

      function hashString(value) {
        let h = 2166136261;
        for (let i = 0; i < String(value).length; i++) {
          h ^= String(value).charCodeAt(i);
          h = Math.imul(h, 16777619);
        }
        return h >>> 0;
      }

      function rand01(seed, salt) {
        const x = Math.sin((hashString(seed + ":" + salt) + 1) * 12.9898) * 43758.5453;
        return x - Math.floor(x);
      }

      function metroFor(seed) {
        return metros[hashString(seed) % metros.length];
      }

      function fuzzMetro(seed) {
        const metro = metroFor(seed);
        const minKm = 15;
        const maxKm = 50;
        const distanceKm = minKm + rand01(seed, "distance") * (maxKm - minKm);
        const angle = rand01(seed, "bearing") * Math.PI * 2;
        const latOffset = (Math.cos(angle) * distanceKm) / 111;
        const lonScale = Math.max(0.25, Math.cos((metro.lat * Math.PI) / 180));
        const lonOffset = (Math.sin(angle) * distanceKm) / (111 * lonScale);
        return { name: metro.name, lat: metro.lat + latOffset, lon: metro.lon + lonOffset };
      }

      function latLonToSphere(lat, lon, radius) {
        const phi = (90 - lat) * Math.PI / 180;
        const theta = (lon + 180) * Math.PI / 180;
        return new THREE.Vector3(
          -radius * Math.sin(phi) * Math.cos(theta),
          radius * Math.cos(phi),
          radius * Math.sin(phi) * Math.sin(theta)
        );
      }

      function latLonToMap(lat, lon) {
        return new THREE.Vector3((lon / 180) * 1.55, (lat / 90) * 0.86, 0);
      }

      function pointFor(hub, radius) {
        return mode === "globe"
          ? latLonToSphere(hub.lat, hub.lon, radius)
          : latLonToMap(hub.lat, hub.lon);
      }

      function resize() {
        const rect = stage.getBoundingClientRect();
        renderer.setSize(rect.width, rect.height, false);
        camera.aspect = rect.width / Math.max(1, rect.height);
        camera.updateProjectionMatrix();
      }

      function clearGroup(group) {
        while (group.children.length) {
          const child = group.children.pop();
          if (child.geometry) child.geometry.dispose();
          if (child.material) child.material.dispose();
        }
      }

      function makeHub(seed, role, count) {
        const loc = fuzzMetro(seed);
        return { seed, role, count: Math.max(1, Number(count || 1)), metro: loc.name, lat: loc.lat, lon: loc.lon, mesh: null, halo: null };
      }

      function deriveHubs(data) {
        const work = data.work || {};
        const nodes = Array.isArray(work.multi_role_nodes) ? work.multi_role_nodes : [];
        const result = [];
        nodes.forEach((node) => {
          (node.roles || []).forEach((role) => result.push(makeHub(node.username + ":" + role, role, 1)));
        });
        [
          ["miners", work.miners],
          ["verifiers", work.verifiers],
          ["clocks", work.clocks],
          ["storage", work.storage_hosts],
          ["services", work.service_hosts],
          ["sensors", work.sensors],
          ["gateways", work.gateways]
        ].forEach(([role, count]) => {
          if (Number(count || 0) > 0) result.push(makeHub("role:" + role, role, count));
        });
        return result.length ? result : [makeHub("hone:genesis", "clock", 1)];
      }

      function renderHubs(data) {
        hubs = deriveHubs(data);
        clearGroup(hubGroup);
        hubs.forEach((hub) => {
          const scale = 0.018 + Math.min(0.08, Math.sqrt(hub.count) * 0.009);
          const color = hub.role.indexOf("storage") >= 0 ? 0x67e8f9 : hub.role.indexOf("miner") >= 0 || hub.role.indexOf("verifier") >= 0 ? 0xf0abfc : 0xf7931a;
          const mesh = new THREE.Mesh(
            new THREE.CylinderGeometry(scale, scale, 0.010, 6),
            new THREE.MeshBasicMaterial({ color, transparent: true, opacity: 0.95, blending: THREE.AdditiveBlending })
          );
          const halo = new THREE.Mesh(
            new THREE.SphereGeometry(scale * 2.4, 16, 8),
            new THREE.MeshBasicMaterial({ color, transparent: true, opacity: 0.12, blending: THREE.AdditiveBlending })
          );
          hub.mesh = mesh;
          hub.halo = halo;
          hubGroup.add(mesh);
          hubGroup.add(halo);
        });
        positionHubs();
      }

      function positionHubs() {
        hubs.forEach((hub) => {
          const pos = pointFor(hub, mode === "globe" ? 1.04 : 1);
          hub.mesh.position.copy(pos);
          hub.halo.position.copy(pos);
          if (mode === "globe") {
            hub.mesh.lookAt(0, 0, 0);
          } else {
            hub.mesh.rotation.set(Math.PI / 2, 0, 0);
          }
        });
        globe.visible = mode === "globe";
        wire.visible = mode === "globe";
        atmosphere.visible = mode === "globe";
      }

      function curveBetween(a, b, kind) {
        const start = pointFor(a, mode === "globe" ? 1.055 : 1);
        const end = pointFor(b, mode === "globe" ? 1.055 : 1);
        let points;
        if (mode === "globe") {
          const mid = start.clone().add(end).multiplyScalar(0.5).normalize().multiplyScalar(kind === "storage" ? 1.22 : 1.42);
          points = new THREE.QuadraticBezierCurve3(start, mid, end).getPoints(kind === "storage" ? 42 : 26);
        } else {
          points = [start, end];
        }
        return new THREE.BufferGeometry().setFromPoints(points);
      }

      function spawnPulse(kind) {
        if (hubs.length < 2) return;
        const source = hubs[Math.floor(Math.random() * hubs.length)];
        let target = hubs[Math.floor(Math.random() * hubs.length)];
        if (target === source) target = hubs[(hubs.indexOf(source) + 1) % hubs.length];
        const material = new THREE.LineBasicMaterial({
          color: kind === "storage" ? 0x67e8f9 : 0xf0abfc,
          transparent: true,
          opacity: kind === "storage" ? 0.82 : 0.95,
          blending: THREE.AdditiveBlending
        });
        const line = new THREE.Line(curveBetween(source, target, kind), material);
        pulseGroup.add(line);
        pulses.push({ line, born: performance.now(), ttl: kind === "storage" ? 4200 : 2100, kind });
        addLedgerEvent(kind, source, target);
      }

      function addLedgerEvent(kind, source, target) {
        const epoch = lastState && lastState.epoch ? lastState.epoch.number : 0;
        const ms = kind === "storage" ? 180 + Math.floor(Math.random() * 420) : 28 + Math.floor(Math.random() * 95);
        const hash = hashString(kind + ":" + source.seed + ":" + target.seed + ":" + Date.now()).toString(16).padStart(8, "0");
        const row = document.createElement("div");
        row.className = "ledger-event " + kind;
        row.innerHTML = "<strong>" + kind + "</strong><span>#" + epoch + " " + hash + "<br><small>" + source.metro + " -> " + target.metro + "</small></span><small>" + ms + "ms</small>";
        ledger.appendChild(row);
        while (ledger.children.length > 18) ledger.removeChild(ledger.firstChild);
        ledgerSubtitle.textContent = "epoch #" + epoch + " synced to visual pulses";
      }

      function updateFromChain(data) {
        lastState = data;
        renderHubs(data);
        const work = data.work || {};
        const inferenceBursts = Math.max(1, Math.min(5, Math.ceil(((work.miners || 0) + (work.verifiers || 0)) / 20)));
        const storageBursts = Math.max(0, Math.min(4, Math.ceil(((work.storage_hosts || 0) + (work.service_hosts || 0)) / 15)));
        for (let i = 0; i < inferenceBursts; i++) setTimeout(() => spawnPulse("inference"), i * 120);
        for (let i = 0; i < storageBursts; i++) setTimeout(() => spawnPulse("storage"), i * 220);
      }

      function animate() {
        requestAnimationFrame(animate);
        if (mode === "globe" && !dragStart) {
          root.rotation.y += rotationVelocity;
          rotationVelocity = rotationVelocity * 0.992 + 0.00085 * 0.008;
        }
        const now = performance.now();
        pulses = pulses.filter((pulse) => {
          const age = now - pulse.born;
          const life = age / pulse.ttl;
          if (life >= 1) {
            pulseGroup.remove(pulse.line);
            pulse.line.geometry.dispose();
            pulse.line.material.dispose();
            return false;
          }
          pulse.line.material.opacity = (pulse.kind === "storage" ? 0.80 : 0.96) * Math.exp(-3.2 * life);
          return true;
        });
        renderer.render(scene, camera);
      }

      function setMode(next) {
        mode = next;
        globeMode.classList.toggle("active", mode === "globe");
        mapMode.classList.toggle("active", mode === "map");
        root.rotation.set(0, 0, 0);
        positionHubs();
        clearGroup(pulseGroup);
        pulses = [];
        if (lastState) updateFromChain(lastState);
      }

      globeMode.addEventListener("click", () => setMode("globe"));
      mapMode.addEventListener("click", () => setMode("map"));
      window.addEventListener("resize", resize);
      canvas.addEventListener("pointerdown", (event) => {
        dragStart = { x: event.clientX, rotation: root.rotation.y };
        canvas.setPointerCapture(event.pointerId);
      });
      canvas.addEventListener("pointermove", (event) => {
        if (!dragStart || mode !== "globe") return;
        const dx = event.clientX - dragStart.x;
        root.rotation.y = dragStart.rotation + dx * 0.006;
        rotationVelocity = dx * 0.00002;
      });
      canvas.addEventListener("pointerup", () => { dragStart = null; });
      canvas.addEventListener("pointercancel", () => { dragStart = null; });

      resize();
      animate();

      if (window.EventSource) {
        const stream = new EventSource("/api/visualizer/stream");
        stream.addEventListener("chain", (event) => updateFromChain(JSON.parse(event.data)));
        stream.onerror = () => { ledgerSubtitle.textContent = "stream reconnecting"; };
      } else {
        const poll = () => fetch("/api/visualizer", { cache: "no-store" }).then((res) => res.json()).then(updateFromChain).catch(() => {});
        poll();
        setInterval(poll, 5000);
      }
    </script>
  `;

  return layout("Live Visualizer", content);
}

module.exports = visualizerView;
