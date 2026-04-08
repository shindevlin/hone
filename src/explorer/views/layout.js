"use strict";

function layout(title, content) {
  return `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>${title} | BTCPC Block Explorer</title>
  <style>
    *, *::before, *::after { box-sizing: border-box; margin: 0; padding: 0; }

    :root {
      --bg-primary: #0a0e17;
      --bg-secondary: #111827;
      --bg-card: #1a2235;
      --bg-card-hover: #1f2a40;
      --border: #2a3550;
      --text-primary: #e2e8f0;
      --text-secondary: #94a3b8;
      --text-muted: #64748b;
      --accent: #f7931a;
      --accent-dim: #c47615;
      --accent-glow: rgba(247, 147, 26, 0.15);
      --green: #22c55e;
      --red: #ef4444;
      --blue: #3b82f6;
      --purple: #a855f7;
    }

    body {
      font-family: 'SF Mono', 'Fira Code', 'Cascadia Code', 'Consolas', monospace;
      background: var(--bg-primary);
      color: var(--text-primary);
      line-height: 1.6;
      min-height: 100vh;
    }

    a { color: var(--accent); text-decoration: none; transition: color 0.2s; }
    a:hover { color: #fbbf24; }

    .header {
      background: var(--bg-secondary);
      border-bottom: 1px solid var(--border);
      padding: 0;
      position: sticky;
      top: 0;
      z-index: 100;
    }

    .header-inner {
      max-width: 1400px;
      margin: 0 auto;
      padding: 0 24px;
      display: flex;
      align-items: center;
      justify-content: space-between;
      height: 64px;
    }

    .logo {
      display: flex;
      align-items: center;
      gap: 12px;
      font-weight: 700;
      font-size: 16px;
      color: var(--text-primary);
    }

    .logo-icon {
      width: 32px;
      height: 32px;
      background: var(--accent);
      border-radius: 6px;
      display: flex;
      align-items: center;
      justify-content: center;
      font-size: 18px;
      font-weight: 900;
      color: #000;
    }

    .logo-text span { color: var(--accent); }

    nav { display: flex; gap: 4px; }

    nav a {
      color: var(--text-secondary);
      padding: 8px 14px;
      border-radius: 6px;
      font-size: 13px;
      font-weight: 500;
      transition: all 0.2s;
    }

    nav a:hover, nav a.active {
      color: var(--text-primary);
      background: var(--bg-card);
    }

    .container {
      max-width: 1400px;
      margin: 0 auto;
      padding: 32px 24px;
    }

    .page-title {
      font-size: 24px;
      font-weight: 700;
      margin-bottom: 24px;
      color: var(--text-primary);
    }

    .page-title span { color: var(--accent); }

    .stats-grid {
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
      gap: 16px;
      margin-bottom: 32px;
    }

    .stat-card {
      background: var(--bg-card);
      border: 1px solid var(--border);
      border-radius: 10px;
      padding: 20px;
      transition: border-color 0.2s;
    }

    .stat-card:hover { border-color: var(--accent-dim); }

    .stat-label {
      font-size: 11px;
      text-transform: uppercase;
      letter-spacing: 1px;
      color: var(--text-muted);
      margin-bottom: 6px;
    }

    .stat-value {
      font-size: 24px;
      font-weight: 700;
      color: var(--text-primary);
    }

    .stat-value.accent { color: var(--accent); }

    .stat-sub {
      font-size: 12px;
      color: var(--text-muted);
      margin-top: 4px;
    }

    .card {
      background: var(--bg-card);
      border: 1px solid var(--border);
      border-radius: 10px;
      margin-bottom: 24px;
      overflow: hidden;
    }

    .card-header {
      padding: 16px 20px;
      border-bottom: 1px solid var(--border);
      display: flex;
      align-items: center;
      justify-content: space-between;
    }

    .card-header h2 {
      font-size: 15px;
      font-weight: 600;
    }

    .card-header .badge {
      background: var(--accent-glow);
      color: var(--accent);
      padding: 4px 10px;
      border-radius: 20px;
      font-size: 11px;
      font-weight: 600;
    }

    table {
      width: 100%;
      border-collapse: collapse;
    }

    thead th {
      text-align: left;
      padding: 12px 16px;
      font-size: 11px;
      text-transform: uppercase;
      letter-spacing: 0.5px;
      color: var(--text-muted);
      border-bottom: 1px solid var(--border);
      font-weight: 600;
      white-space: nowrap;
    }

    tbody td {
      padding: 12px 16px;
      font-size: 13px;
      border-bottom: 1px solid rgba(42, 53, 80, 0.5);
      color: var(--text-secondary);
    }

    tbody tr:hover { background: var(--bg-card-hover); }
    tbody tr:last-child td { border-bottom: none; }

    .hash {
      font-family: 'SF Mono', monospace;
      font-size: 12px;
      color: var(--text-muted);
      max-width: 160px;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
      display: inline-block;
    }

    .status {
      display: inline-block;
      padding: 3px 10px;
      border-radius: 20px;
      font-size: 11px;
      font-weight: 600;
      text-transform: uppercase;
      letter-spacing: 0.5px;
    }

    .status-active { background: rgba(34, 197, 94, 0.15); color: var(--green); }
    .status-finalized { background: rgba(59, 130, 246, 0.15); color: var(--blue); }
    .status-committed { background: rgba(168, 85, 247, 0.15); color: var(--purple); }
    .status-registered { background: rgba(247, 147, 26, 0.15); color: var(--accent); }
    .status-suspended { background: rgba(239, 68, 68, 0.15); color: var(--red); }
    .status-banned { background: rgba(239, 68, 68, 0.25); color: var(--red); }

    .type-badge {
      display: inline-block;
      padding: 3px 8px;
      border-radius: 4px;
      font-size: 11px;
      font-weight: 600;
      text-transform: uppercase;
    }

    .type-transfer { background: rgba(59, 130, 246, 0.15); color: var(--blue); }
    .type-mining_reward { background: rgba(247, 147, 26, 0.15); color: var(--accent); }
    .type-staking_reward { background: rgba(168, 85, 247, 0.15); color: var(--purple); }
    .type-staking { background: rgba(34, 197, 94, 0.15); color: var(--green); }
    .type-unstaking { background: rgba(239, 68, 68, 0.15); color: var(--red); }

    .amount { font-weight: 600; color: var(--text-primary); }
    .amount.positive { color: var(--green); }
    .amount.negative { color: var(--red); }

    .two-col {
      display: grid;
      grid-template-columns: 1fr 1fr;
      gap: 24px;
    }

    .detail-grid {
      display: grid;
      grid-template-columns: 160px 1fr;
      gap: 0;
    }

    .detail-grid dt {
      padding: 10px 16px;
      font-size: 12px;
      color: var(--text-muted);
      text-transform: uppercase;
      letter-spacing: 0.5px;
      border-bottom: 1px solid rgba(42, 53, 80, 0.5);
    }

    .detail-grid dd {
      padding: 10px 16px;
      font-size: 13px;
      color: var(--text-secondary);
      border-bottom: 1px solid rgba(42, 53, 80, 0.5);
      word-break: break-all;
    }

    .empty-state {
      text-align: center;
      padding: 48px 24px;
      color: var(--text-muted);
      font-size: 14px;
    }

    .search-bar {
      display: flex;
      gap: 8px;
      max-width: 500px;
    }

    .search-bar input {
      flex: 1;
      background: var(--bg-secondary);
      border: 1px solid var(--border);
      border-radius: 6px;
      padding: 8px 14px;
      color: var(--text-primary);
      font-family: inherit;
      font-size: 13px;
      outline: none;
      transition: border-color 0.2s;
    }

    .search-bar input:focus { border-color: var(--accent); }

    .search-bar button {
      background: var(--accent);
      color: #000;
      border: none;
      border-radius: 6px;
      padding: 8px 16px;
      font-family: inherit;
      font-size: 13px;
      font-weight: 600;
      cursor: pointer;
      transition: background 0.2s;
    }

    .search-bar button:hover { background: #fbbf24; }

    .footer {
      border-top: 1px solid var(--border);
      padding: 24px;
      text-align: center;
      color: var(--text-muted);
      font-size: 12px;
      margin-top: 48px;
    }

    .refresh-indicator {
      font-size: 11px;
      color: var(--text-muted);
    }

    .refresh-indicator .dot {
      display: inline-block;
      width: 6px;
      height: 6px;
      border-radius: 50%;
      background: var(--green);
      margin-right: 6px;
      animation: pulse 2s infinite;
    }

    @keyframes pulse {
      0%, 100% { opacity: 1; }
      50% { opacity: 0.3; }
    }

    .hw-specs {
      font-size: 12px;
      color: var(--text-muted);
      line-height: 1.8;
    }

    .model-tag {
      display: inline-block;
      background: var(--bg-secondary);
      border: 1px solid var(--border);
      padding: 2px 8px;
      border-radius: 4px;
      font-size: 11px;
      margin: 2px;
      color: var(--text-secondary);
    }

    .progress-bar {
      width: 100%;
      height: 6px;
      background: var(--bg-secondary);
      border-radius: 3px;
      overflow: hidden;
      margin-top: 8px;
    }

    .progress-bar .fill {
      height: 100%;
      background: linear-gradient(90deg, var(--accent), #fbbf24);
      border-radius: 3px;
      transition: width 0.3s;
    }

    @media (max-width: 768px) {
      .header-inner { padding: 0 16px; }
      nav a { padding: 6px 10px; font-size: 12px; }
      .container { padding: 16px; }
      .stats-grid { grid-template-columns: repeat(2, 1fr); gap: 10px; }
      .two-col { grid-template-columns: 1fr; }
      .stat-value { font-size: 18px; }
      .detail-grid { grid-template-columns: 120px 1fr; }
      table { font-size: 12px; }
      thead th, tbody td { padding: 8px 10px; }
    }

    @media (max-width: 480px) {
      .stats-grid { grid-template-columns: 1fr; }
      nav { gap: 0; }
      nav a { padding: 6px 6px; font-size: 11px; }
      .logo { font-size: 14px; }
    }
  </style>
</head>
<body>
  <header class="header">
    <div class="header-inner">
      <a href="/" class="logo">
        <div class="logo-icon">B</div>
        <div class="logo-text">BTCPC <span>Explorer</span></div>
      </a>
      <nav>
        <a href="/">Dashboard</a>
        <a href="/tx">Transactions</a>
        <a href="/miners">Miners</a>
        <a href="/settings">Settings</a>
      </nav>
      <div class="refresh-indicator">
        <span class="dot"></span>Live
      </div>
    </div>
  </header>
  <div class="container">
    ${content}
  </div>
  <footer class="footer">
    Bitcoin Proof of Compute &mdash; Block Explorer &mdash; Designed by Shin Devlin
  </footer>
  <script>
    setTimeout(function() { location.reload(); }, 30000);
  </script>
</body>
</html>`;
}

module.exports = layout;
