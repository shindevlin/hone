/* HONE camera listener: install cards shared by /join and /listener.
   HoneListener.render(el, {ref, only, compact}) fills `el` with one card per OS.
   Downloads come only from https://honemesh.net/app/listener/ (version.json lists them).
   Until version.json exists the cards say "almost ready" and no command is offered. */
(function () {
  var BASE = "https://honemesh.net/app/listener/";
  var CONTACT = "shindevlin@proton.me";

  function detect() {
    var ua = navigator.userAgent || "";
    var touchMac = /Macintosh/.test(ua) && navigator.maxTouchPoints > 1;
    if (/iPhone|iPad|iPod/.test(ua) || touchMac) return "ios";
    if (/Android/.test(ua)) return "android";
    if (/Windows/.test(ua)) return "windows";
    if (/Mac OS X|Macintosh/.test(ua)) return "mac";
    if (/CrOS/.test(ua)) return "chromeos";
    if (/Linux|X11/.test(ua)) return "linux";
    return "other";
  }

  function cleanRef(r) {
    r = String(r || "").toLowerCase();
    return /^[a-z0-9][a-z0-9._-]{0,31}$/.test(r) ? r : "";
  }

  function cmd(os, ref) {
    if (os === "windows") {
      return (ref ? "$env:HONE_REF='" + ref + "'; " : "") + "irm " + BASE + "install.ps1 | iex";
    }
    return "curl -fsSL " + BASE + "install.sh | " + (ref ? "HONE_REF=" + ref + " " : "") + "sh";
  }

  var OS = {
    windows: { title: "Windows", sub: "Windows 10 or 11 PC", term: "Open PowerShell (press Start, type PowerShell, press Enter), paste this and press Enter:" },
    mac: { title: "Mac", sub: "macOS 12 or newer", term: "Open Terminal (press Cmd+Space, type Terminal, press Enter), paste this and press Enter:" },
    linux: { title: "Linux and Raspberry Pi", sub: "Ubuntu, Debian, Raspberry Pi OS and most others", term: "Open a terminal, paste this and press Enter:" }
  };

  function el(tag, cls, text) {
    var e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text != null) e.textContent = text;
    return e;
  }

  function copyBtn(text, ready) {
    var b = el("button", "hl-copy", ready ? "Copy" : "Not ready yet");
    b.type = "button";
    b.disabled = !ready;
    b.addEventListener("click", function () {
      function done() { b.textContent = "Copied"; setTimeout(function () { b.textContent = "Copy"; }, 1800); }
      if (navigator.clipboard && window.isSecureContext) {
        navigator.clipboard.writeText(text).then(done, fallback);
      } else fallback();
      function fallback() {
        var t = document.createElement("textarea"); t.value = text; t.style.position = "fixed"; t.style.opacity = "0";
        document.body.appendChild(t); t.select();
        try { document.execCommand("copy"); done(); } catch (e) { b.textContent = "Select and copy"; }
        document.body.removeChild(t);
      }
    });
    return b;
  }

  function card(os, ref, m, primary) {
    var ready = !!m;
    var c = el("section", "hl-card" + (primary ? " hl-primary" : ""));
    c.id = "install-" + os;
    var h = el("div", "hl-head");
    h.appendChild(el("h3", null, OS[os].title));
    if (primary) h.appendChild(el("span", "hl-tag", "This computer"));
    c.appendChild(h);
    c.appendChild(el("p", "hl-sub", OS[os].sub));

    if (os === "windows") {
      var store = m && typeof m.store_url === "string" && /^https:\/\/apps\.microsoft\.com\//.test(m.store_url) ? m.store_url : "";
      var a = el("a", "hl-store" + (store ? "" : " hl-soon"));
      a.innerHTML = '<svg viewBox="0 0 24 24" aria-hidden="true"><path fill="currentColor" d="M3 5.5 10.5 4.4v7.1H3V5.5Zm0 13 7.5 1.1v-7.1H3v6Zm8.4 1.2L21 21V12.5h-9.6v7.2Zm0-15.4v7.2H21V3l-9.6 1.3Z"/></svg>';
      a.appendChild(el("span", null, store ? "Get it from the Microsoft Store" : "Microsoft Store: coming soon"));
      if (store) { a.href = store; a.rel = "noopener"; } else { a.setAttribute("aria-disabled", "true"); }
      c.appendChild(a);
      c.appendChild(el("p", "hl-or", store ? "Or install it with one line:" : "For now, install it with one line:"));
    }
    c.appendChild(el("p", "hl-term", OS[os].term));
    var line = cmd(os, ref);
    var box = el("div", "hl-cmd" + (ready ? "" : " hl-off"));
    var code = el("code", null, line);
    box.appendChild(code);
    box.appendChild(copyBtn(line, ready));
    c.appendChild(box);
    if (!ready) {
      var n = el("p", "hl-wait");
      n.appendChild(document.createTextNode("Almost ready. The installer goes live here in the next few days. "));
      var mail = el("a", null, "Email me when it's ready");
      mail.href = "mailto:" + CONTACT + "?subject=" + encodeURIComponent("Tell me when the HONE listener is ready") +
        "&body=" + encodeURIComponent("Please let me know when I can install the HONE camera listener." + (ref ? "\nInvite code: " + ref : ""));
      n.appendChild(mail);
      c.appendChild(n);
    }
    if (os === "linux") {
      var pi = el("details", "hl-pi");
      pi.appendChild(el("summary", null, "No computer that stays on? Buy a Raspberry Pi 5 kit"));
      var ul = el("ul");
      [
        "A Raspberry Pi 5 with 4 GB is plenty. It's a quiet little computer that uses about as much power as a night light.",
        "Get a starter kit with the official 27 W USB-C power supply, a case with a fan, and a 32 GB or larger microSD card. Kits usually cost around $100 to $130.",
        "Put Raspberry Pi OS on the card with Raspberry Pi Imager, turn on Wi-Fi in the imager, then start the Pi.",
        "Plug it into your router with a network cable if you can. It hears the camera more reliably than over Wi-Fi.",
        "Open the Pi's terminal and run the command above. That's it: leave it plugged in and it listens day and night."
      ].forEach(function (t) { ul.appendChild(el("li", null, t)); });
      pi.appendChild(ul);
      var p = el("p", "hl-sub");
      var link = el("a", null, "Find an official Raspberry Pi reseller near you");
      link.href = "https://www.raspberrypi.com/products/raspberry-pi-5/"; link.rel = "noopener";
      p.appendChild(link);
      pi.appendChild(p);
      c.appendChild(pi);
    }
    return c;
  }

  var CSS = ".hl-card{background:var(--surface);border:1px solid var(--line);border-radius:16px;padding:16px;margin:0 0 14px}" +
    ".hl-primary{border-color:var(--accent);box-shadow:0 0 0 1px var(--accent) inset}" +
    ".hl-head{display:flex;align-items:center;gap:10px;justify-content:space-between}" +
    ".hl-card h3{margin:0;font-size:19px}" +
    ".hl-tag{font-size:12px;font-weight:700;color:#000;background:var(--accent);border-radius:99px;padding:2px 10px}" +
    ".hl-sub{color:var(--faint);font-size:14px;margin:2px 0 12px}" +
    ".hl-term,.hl-or{color:var(--dim);font-size:14px;margin:10px 0 8px}" +
    ".hl-cmd{display:flex;gap:8px;align-items:stretch;background:var(--bg);border:1px solid var(--line);border-radius:12px;padding:6px 6px 6px 12px}" +
    ".hl-cmd code{flex:1;min-width:0;font:13px/1.45 ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;color:var(--text);overflow-wrap:anywhere;align-self:center;padding:4px 0}" +
    ".hl-off code{opacity:.45}" +
    ".hl-copy{flex:none;border:0;border-radius:9px;background:var(--accent);color:#000;font-weight:700;font-size:14px;padding:8px 14px;cursor:pointer}" +
    ".hl-copy:disabled{background:var(--hi);color:var(--faint);cursor:default}" +
    ".hl-store{display:flex;align-items:center;gap:10px;justify-content:center;background:var(--text);color:var(--bg);border-radius:12px;padding:12px;font-weight:700;text-decoration:none}" +
    ".hl-store svg{width:20px;height:20px}" +
    ".hl-soon{opacity:.55;cursor:default}" +
    ".hl-wait{font-size:14px;color:var(--dim);margin:10px 0 0}.hl-wait a{color:var(--accent)}" +
    ".hl-pi{margin-top:14px;background:var(--hi);border-radius:12px;padding:10px 12px}" +
    ".hl-pi summary{cursor:pointer;font-weight:600}.hl-pi ul{padding-left:18px;margin:10px 0 6px;color:var(--dim);font-size:14px}.hl-pi li{margin:0 0 6px}.hl-pi a{color:var(--accent)}" +
    ".hl-ver{font-size:12px;color:var(--faint);text-align:center;margin:4px 0 0}";
  function style() {
    if (document.getElementById("hl-style")) return;
    var s = document.createElement("style"); s.id = "hl-style"; s.textContent = CSS; document.head.appendChild(s);
  }

  var _manifest = null;
  function manifest() {
    if (_manifest) return _manifest;
    _manifest = fetch("/app/listener/version.json", { cache: "no-store" })
      .then(function (r) { return r.ok ? r.json() : null; })
      .catch(function () { return null; });
    return _manifest;
  }

  function render(root, opts) {
    opts = opts || {};
    var ref = cleanRef(opts.ref);
    var here = detect();
    var mine = here === "windows" || here === "mac" ? here : (here === "linux" || here === "chromeos" ? "linux" : null);
    var order = opts.only ? [opts.only] : (mine ? [mine] : []).concat(["windows", "mac", "linux"].filter(function (o) { return o !== mine; }));
    if (opts.compact && mine) order = [mine];
    function paint(m) {
      root.innerHTML = "";
      order.forEach(function (os, i) { root.appendChild(card(os, ref, m, i === 0 && os === mine)); });
      if (m && m.version) {
        var v = el("p", "hl-ver", "Version " + m.version + " · every download is checked and signed by HONE");
        root.appendChild(v);
      }
    }
    style();
    root.textContent = "Checking for the latest version…";
    manifest().then(paint);
  }

  window.HoneListener = { render: render, detect: detect, cleanRef: cleanRef, command: cmd };
})();
