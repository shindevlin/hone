(() => {
  // Inject global nav styles
  const style = document.createElement('style');
  style.textContent = `
    #btcpc-nav {
      position: fixed;
      top: 0; left: 0; right: 0;
      z-index: 9999;
      display: flex;
      align-items: center;
      gap: 0;
      padding: 0 16px;
      height: 46px;
      background: rgba(8, 8, 12, 0.97);
      border-bottom: 1px solid rgba(247, 147, 26, 0.25);
      backdrop-filter: blur(12px);
      -webkit-backdrop-filter: blur(12px);
      font-family: 'SF Mono', 'Fira Code', 'Cascadia Code', Consolas, monospace;
      font-size: 12px;
      overflow-x: auto;
      scrollbar-width: none;
    }
    #btcpc-nav::-webkit-scrollbar { display: none; }

    #btcpc-nav-logo {
      display: inline-flex;
      align-items: center;
      gap: 8px;
      text-decoration: none;
      color: #f7931a;
      font-weight: 700;
      font-size: 14px;
      letter-spacing: 0.5px;
      margin-right: 18px;
      flex-shrink: 0;
    }
    #btcpc-nav-logo img {
      width: 26px;
      height: 26px;
      border-radius: 7px;
      object-fit: cover;
      box-shadow: 0 0 14px rgba(247, 147, 26, 0.5), 0 0 28px rgba(247, 147, 26, 0.2);
      transition: box-shadow 0.3s;
    }
    #btcpc-nav-logo:hover img {
      box-shadow: 0 0 20px rgba(247, 147, 26, 0.75), 0 0 40px rgba(247, 147, 26, 0.35);
    }

    .btcpc-nav-link {
      color: #8892a4;
      text-decoration: none;
      padding: 0 10px;
      height: 46px;
      display: inline-flex;
      align-items: center;
      white-space: nowrap;
      transition: color 0.15s;
      border-bottom: 2px solid transparent;
      flex-shrink: 0;
    }
    .btcpc-nav-link:hover { color: #c8ccd4; }
    .btcpc-nav-link.active {
      color: #f7931a;
      border-bottom: 2px solid rgba(247, 147, 26, 0.6);
    }

    #btcpc-nav-spacer { flex: 1; min-width: 12px; }

    /* Live chain status pill */
    #btcpc-nav-status {
      display: inline-flex;
      align-items: center;
      gap: 6px;
      background: rgba(34, 197, 94, 0.08);
      border: 1px solid rgba(34, 197, 94, 0.2);
      border-radius: 20px;
      padding: 3px 10px;
      color: #22c55e;
      font-size: 11px;
      letter-spacing: 0.3px;
      flex-shrink: 0;
      margin-right: 10px;
      cursor: default;
    }
    #btcpc-nav-status.offline {
      background: rgba(239, 68, 68, 0.08);
      border-color: rgba(239, 68, 68, 0.2);
      color: #ef4444;
    }
    .btcpc-pulse-dot {
      width: 7px;
      height: 7px;
      border-radius: 50%;
      background: #22c55e;
      flex-shrink: 0;
      animation: btcpc-pulse 2s ease-in-out infinite;
    }
    .btcpc-pulse-dot.offline { background: #ef4444; animation: none; }
    @keyframes btcpc-pulse {
      0%, 100% { opacity: 1; box-shadow: 0 0 0 0 rgba(34, 197, 94, 0.5); }
      50% { opacity: 0.85; box-shadow: 0 0 0 4px rgba(34, 197, 94, 0); }
    }

    /* Epoch counter */
    #btcpc-nav-epoch {
      font-variant-numeric: tabular-nums;
    }

    /* My Node CTA */
    #btcpc-nav-mynode {
      display: inline-flex;
      align-items: center;
      height: 30px;
      padding: 0 14px;
      background: rgba(247, 147, 26, 0.1);
      border: 1px solid rgba(247, 147, 26, 0.35);
      border-radius: 6px;
      color: #f7931a;
      font-weight: 600;
      text-decoration: none;
      font-size: 12px;
      white-space: nowrap;
      flex-shrink: 0;
      transition: background 0.15s, border-color 0.15s;
    }
    #btcpc-nav-mynode:hover {
      background: rgba(247, 147, 26, 0.2);
      border-color: rgba(247, 147, 26, 0.6);
    }

    /* Push page body down */
    #btcpc-nav-spacer-block {
      height: 46px;
      display: block;
    }

    /* Safety hide for any remaining legacy nav bars */
    #nav-bar { display: none !important; }
  `;
  document.head.appendChild(style);

  // Nav link definitions — path fragments used for active detection
  const links = [
    { label: 'Install',  href: '/install',      match: '/install' },
    { label: 'Buy',      href: '/buy',          match: '/buy' },
    { label: 'Explorer', href: '/dashboard',    match: '/dashboard' },
    { label: 'Profile',  href: '/profile.html', match: '/profile' },
  ];

  const path = window.location.pathname;

  const navEl = document.createElement('nav');
  navEl.id = 'btcpc-nav';
  navEl.setAttribute('aria-label', 'BTCPC main navigation');

  // Logo
  navEl.innerHTML = `
    <a href="/" id="btcpc-nav-logo">
      <img src="/btcpc-logo.jpeg" alt="BTCPC logo">
      BTCPC
    </a>
  `;

  // Links
  for (const l of links) {
    const a = document.createElement('a');
    a.className = 'btcpc-nav-link';
    a.href = l.href;
    a.textContent = l.label;
    if (l.match && path.startsWith(l.match)) a.classList.add('active');
    navEl.appendChild(a);
  }

  // Spacer
  const spacer = document.createElement('div');
  spacer.id = 'btcpc-nav-spacer';
  navEl.appendChild(spacer);

  // Live status pill
  const statusEl = document.createElement('div');
  statusEl.id = 'btcpc-nav-status';
  statusEl.innerHTML = `<span class="btcpc-pulse-dot"></span><span id="btcpc-nav-epoch">...</span>`;
  navEl.appendChild(statusEl);

  // Telegram icon
  const tg = document.createElement('a');
  tg.href = 'https://t.me/btcpcbot';
  tg.target = '_blank';
  tg.rel = 'noopener';
  tg.title = 'Telegram';
  tg.style.cssText = 'display:inline-flex;align-items:center;color:#8892a4;text-decoration:none;padding:0 8px;transition:color 0.15s;flex-shrink:0;';
  tg.innerHTML = `<svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor"><path d="M12 0C5.373 0 0 5.373 0 12s5.373 12 12 12 12-5.373 12-12S18.627 0 12 0zm5.894 8.221-1.97 9.28c-.145.658-.537.818-1.084.508l-3-2.21-1.447 1.394c-.16.16-.295.295-.605.295l.213-3.053 5.56-5.023c.242-.213-.054-.333-.373-.12L7.18 13.888l-2.967-.924c-.643-.204-.657-.643.136-.953l11.57-4.461c.537-.194 1.006.131.975.671z"/></svg>`;
  tg.onmouseover = () => tg.style.color = '#229ed9';
  tg.onmouseout  = () => tg.style.color = '#8892a4';
  navEl.appendChild(tg);

  // My Node CTA
  const mynode = document.createElement('a');
  mynode.id = 'btcpc-nav-mynode';
  mynode.href = '/node';
  mynode.textContent = 'My Node →';
  navEl.appendChild(mynode);

  // Spacer block to push page content below the fixed bar
  const spacerBlock = document.createElement('div');
  spacerBlock.id = 'btcpc-nav-spacer-block';

  // Inject before the first child of body (or after current script)
  const body = document.body || document.documentElement;
  body.insertBefore(spacerBlock, body.firstChild);
  body.insertBefore(navEl, spacerBlock);

  // Fetch live chain data
  function updateChainStatus() {
    fetch('/public/network', { cache: 'no-store' })
      .then(r => r.json())
      .then(d => {
        const epochEl = document.getElementById('btcpc-nav-epoch');
        const dot = statusEl.querySelector('.btcpc-pulse-dot');
        if (epochEl && d.epoch) {
          epochEl.textContent = `Epoch ${d.epoch.toLocaleString()}`;
        }
        if (d.peer_count != null && epochEl) {
          epochEl.textContent += ` · ${d.peer_count}p`;
        }
        statusEl.classList.remove('offline');
        if (dot) dot.classList.remove('offline');
      })
      .catch(() => {
        const epochEl = document.getElementById('btcpc-nav-epoch');
        if (epochEl) epochEl.textContent = 'offline';
        statusEl.classList.add('offline');
        const dot = statusEl.querySelector('.btcpc-pulse-dot');
        if (dot) dot.classList.add('offline');
      });
  }

  updateChainStatus();
  setInterval(updateChainStatus, 15000);

  // Also check if local node is running and link My Node accordingly
  fetch('http://localhost:4242/api/node/info', { signal: AbortSignal.timeout(800) })
    .then(r => r.ok ? r.json() : null)
    .then(d => {
      if (d && mynode) {
        mynode.href = 'http://localhost:4242';
        mynode.title = 'Local node running';
        mynode.style.borderColor = 'rgba(34,197,94,0.5)';
        mynode.style.color = '#22c55e';
      }
    })
    .catch(() => {});
})();
