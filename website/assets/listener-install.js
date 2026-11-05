/* Ornigo listener install cards, shared by /ornigo (and /listener) and /join.
   OrnigoInstall.render(el, {ref, voucher, only, compact}) fills `el` with one card per computer OS.
   Every download and command points at https://honemesh.net/app/listener/ (version.json lists the files).
   Invite code (?ref=) goes to the installer as HONE_REF, sponsor voucher (?v=) as HONE_VOUCHER.
   Needs /assets/ornigo.css. */
(function () {
  var BASE = "https://honemesh.net/app/listener/";
  var HELP = "ornigo@honemesh.net";

  function detect() {
    var ua = navigator.userAgent || "";
    var touchMac = /Macintosh/.test(ua) && navigator.maxTouchPoints > 1;
    if (/iPhone|iPad|iPod/.test(ua) || touchMac) return "ios";
    if (/Android/.test(ua)) return "android";
    if (/Windows/.test(ua)) return "windows";
    if (/Mac OS X|Macintosh/.test(ua)) return "mac";
    if (/CrOS|Linux|X11/.test(ua)) return "linux";
    return "other";
  }
  function cleanRef(r) {
    r = String(r || "").toLowerCase();
    return /^[a-z0-9][a-z0-9._-]{0,31}$/.test(r) ? r : "";
  }
  function cleanVoucher(v) {
    v = String(v || "");
    return /^[A-Za-z0-9_-]{16,128}$/.test(v) ? v : "";
  }
  /** "?ref=..&v=.." for links that carry the invite on. */
  function carry(ref, v) {
    var p = [];
    if (ref) p.push("ref=" + encodeURIComponent(ref));
    if (v) p.push("v=" + encodeURIComponent(v));
    return p.length ? "?" + p.join("&") : "";
  }
  function command(os, ref, v) {
    if (os === "windows") {
      return (ref ? "$env:HONE_REF='" + ref + "'; " : "") + (v ? "$env:HONE_VOUCHER='" + v + "'; " : "") + "irm " + BASE + "install.ps1 | iex";
    }
    return "curl -fsSL " + BASE + "install.sh | " + (ref ? "HONE_REF=" + ref + " " : "") + (v ? "HONE_VOUCHER=" + v + " " : "") + "sh";
  }

  var CSS = ".oi{display:grid;grid-template-columns:repeat(auto-fit,minmax(260px,1fr));gap:14px}" +
    ".oi-c{padding:22px;border-radius:24px;background:var(--canopy);border:1px solid var(--twig);display:flex;flex-direction:column;gap:10px;min-width:0}" +
    ".oi-c.mine{border-color:var(--chorus)}" +
    ".oi-h{display:flex;align-items:center;justify-content:space-between;gap:8px}" +
    ".oi-h b{font:800 22px var(--disp)}" +
    ".oi-tag{font-size:12px;font-weight:700;color:var(--night);background:var(--chorus);border-radius:99px;padding:2px 10px}" +
    ".oi-c .go{align-self:flex-start}" +
    ".oi-cmd{display:flex;gap:8px;align-items:stretch;background:var(--deep);border:1px solid var(--twig);border-radius:14px;padding:6px 6px 6px 12px}" +
    ".oi-cmd code{flex:1;min-width:0;font:500 13px/1.5 var(--mono);color:var(--ink);overflow-wrap:anywhere;align-self:center;padding:4px 0}" +
    ".oi-cp{flex:none;min-height:44px;border:0;border-radius:12px;background:var(--bough);color:var(--ink);font-weight:700;font-size:14px;padding:0 14px;cursor:pointer}" +
    ".oi-small{font-size:13px;color:var(--dim);margin:0}.oi-small a{color:var(--chorus)}" +
    ".oi-pi{background:var(--bough);border-radius:16px;padding:10px 14px}" +
    ".oi-pi summary{cursor:pointer;font-weight:700}.oi-pi ul{padding-left:18px;margin:10px 0 4px;color:var(--ink2);font-size:14px}.oi-pi li{margin:0 0 6px}" +
    ".oi-ver{grid-column:1/-1;font-size:13px;color:var(--dim);margin:2px 0 0}";
  function style() {
    if (document.getElementById("oi-style")) return;
    var s = document.createElement("style"); s.id = "oi-style"; s.textContent = CSS; document.head.appendChild(s);
  }
  function el(tag, cls, text) {
    var e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text != null) e.textContent = text;
    return e;
  }
  function link(cls, text, href) { var a = el("a", cls, text); a.href = href; return a; }
  function copyBox(text) {
    var box = el("div", "oi-cmd");
    box.appendChild(el("code", null, text));
    var b = el("button", "oi-cp", "Copy"); b.type = "button";
    b.addEventListener("click", function () {
      function done() { b.textContent = "Copied"; setTimeout(function () { b.textContent = "Copy"; }, 1800); }
      function fallback() {
        var t = document.createElement("textarea"); t.value = text; t.style.position = "fixed"; t.style.opacity = "0";
        document.body.appendChild(t); t.select();
        try { document.execCommand("copy"); done(); } catch (e) { b.textContent = "Select and copy"; }
        document.body.removeChild(t);
      }
      if (navigator.clipboard && window.isSecureContext) navigator.clipboard.writeText(text).then(done, fallback); else fallback();
    });
    box.appendChild(b);
    return box;
  }
  function reveal(label, open, body) {
    var wrap = el("div"); wrap.style.cssText = "display:flex;flex-direction:column;gap:8px";
    var b = el("button", "ghost", open ? "Hide the install command" : label); b.type = "button";
    b.style.alignSelf = "flex-start";
    b.setAttribute("aria-expanded", open ? "true" : "false");
    body.hidden = !open;
    b.addEventListener("click", function () {
      body.hidden = !body.hidden;
      b.setAttribute("aria-expanded", body.hidden ? "false" : "true");
      b.textContent = body.hidden ? label : "Hide the install command";
    });
    wrap.appendChild(b); wrap.appendChild(body);
    return wrap;
  }
  function soon(what, ref) {
    var p = el("p", "oi-small");
    p.appendChild(document.createTextNode(what + " "));
    p.appendChild(link(null, "Email me when it's ready", "mailto:" + HELP + "?subject=" + encodeURIComponent("Tell me when Ornigo is ready") +
      "&body=" + encodeURIComponent("Please let me know when I can install Ornigo." + (ref ? "\nInvite code: " + ref : ""))));
    return p;
  }
  function fileLike(m, re) {
    var f = (m && m.files) || {};
    return Object.keys(f).filter(function (k) { return re.test(k); })[0] || "";
  }

  function card(os, o, m, mine) {
    var c = el("section", "oi-c" + (mine ? " mine" : ""));
    c.id = "install-" + os;
    var h = el("div", "oi-h");
    h.appendChild(el("b", null, { windows: "Windows", mac: "Mac", linux: "Linux and Raspberry Pi" }[os]));
    if (mine) h.appendChild(el("span", "oi-tag", "This computer"));
    c.appendChild(h);
    c.appendChild(el("p", "mut", { windows: "Windows 10 or 11", mac: "macOS 12 or newer", linux: "Debian, Ubuntu, Raspberry Pi OS" }[os]));
    var invite = !!(o.ref || o.voucher);
    var cmdBody = el("div"); cmdBody.style.cssText = "display:flex;flex-direction:column;gap:6px";
    cmdBody.appendChild(el("p", "oi-small", os === "windows" ? "Open PowerShell (press Start, type PowerShell, press Enter), paste this, press Enter:" :
      os === "mac" ? "Open Terminal (Cmd+Space, type Terminal, press Enter), paste this, press Enter:" : "Open a terminal, paste this, press Enter:"));
    cmdBody.appendChild(copyBox(command(os, o.ref, o.voucher)));
    if (invite) cmdBody.appendChild(el("p", "oi-small", "Your invite comes along with this line."));

    if (!m) {
      c.appendChild(soon("Almost ready. Ornigo goes live here in the next few days.", o.ref));
      return c;
    }
    if (os === "windows") {
      var store = typeof m.store_url === "string" && /^https:\/\/apps\.microsoft\.com\//.test(m.store_url) ? m.store_url : "";
      var exe = fileLike(m, /windows.*\.exe$/i);
      if (store) { var s = link("go", "Get it from Microsoft Store", store); s.rel = "noopener"; c.appendChild(s); }
      else if (exe) c.appendChild(link("go", "Download for Windows", BASE + exe));
      c.appendChild(el("p", "oi-small", store ? "Updates itself through the Store." : "Microsoft Store listing coming soon."));
      c.appendChild(reveal("Or install with one line", invite || !(store || exe), cmdBody));
    } else if (os === "mac") {
      if (!fileLike(m, /macos/i)) { c.appendChild(soon("The Mac version is almost ready.", o.ref)); return c; }
      c.appendChild(cmdBody);
    } else {
      c.appendChild(reveal("Show the install command", mine || invite, cmdBody));
      var debs = Object.keys(m.files || {}).filter(function (k) { return /\.deb$/.test(k); });
      if (debs.length) {
        var p = el("p", "oi-small"); p.appendChild(document.createTextNode("Or download a package: "));
        debs.forEach(function (d, i) {
          if (i) p.appendChild(document.createTextNode(" · "));
          p.appendChild(link(null, /arm64/.test(d) ? "Raspberry Pi / ARM (.deb)" : "PC / x86 (.deb)", BASE + d));
        });
        c.appendChild(p);
      }
      var pi = el("details", "oi-pi");
      pi.appendChild(el("summary", null, "No computer that stays on? A Raspberry Pi 5 works"));
      var ul = el("ul");
      ["A Raspberry Pi 5 with 4 GB is plenty, and it's quiet and uses very little power.",
        "Get a kit with the official 27 W USB-C power supply, a case with a fan and a 32 GB or larger microSD card.",
        "Put Raspberry Pi OS on the card with Raspberry Pi Imager, then run the command above on the Pi.",
        "An Android phone on a charger by the window works too."].forEach(function (t) { ul.appendChild(el("li", null, t)); });
      pi.appendChild(ul);
      c.appendChild(pi);
    }
    return c;
  }

  var _m = null;
  function manifest() {
    if (!_m) _m = fetch("/app/listener/version.json", { cache: "no-store" })
      .then(function (r) { return r.ok ? r.json() : null; }).catch(function () { return null; });
    return _m;
  }

  function render(root, o) {
    o = o || {};
    var q = new URLSearchParams(location.search);
    o.ref = cleanRef(o.ref != null ? o.ref : q.get("ref"));
    o.voucher = cleanVoucher(o.voucher != null ? o.voucher : q.get("v"));
    var here = detect();
    var mine = here === "windows" || here === "mac" || here === "linux" ? here : null;
    var order = o.only ? [o.only] : (mine ? [mine] : []).concat(["windows", "mac", "linux"].filter(function (x) { return x !== mine; }));
    if (o.compact && mine) order = [mine];
    style();
    root.classList.add("oi");
    root.textContent = "Checking for the latest version…";
    manifest().then(function (m) {
      root.textContent = "";
      order.forEach(function (os) { root.appendChild(card(os, o, m, os === mine)); });
      if (m && m.version) root.appendChild(el("p", "oi-ver", "Version " + m.version + " · every download is checked and signed by Honemesh"));
    });
  }

  window.OrnigoInstall = window.HoneListener = { render: render, detect: detect, cleanRef: cleanRef, cleanVoucher: cleanVoucher, carry: carry, command: command, manifest: manifest };
})();
