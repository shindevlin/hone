"""HONE web read API: the /me page and the listener MCP (served by the birds indexer process).

Read-only and owner-scoped. Nothing here writes sightings, profiles or devices.

  POST /api/accounts/web/login    {email,password}      -> {token, user}   read-only web session
  POST /api/accounts/web/setup-link {email,ref?,v?}      email the computer setup link (rate limited)
  POST /api/accounts/web/logout                          (Bearer web)
  POST /api/accounts/web/token    {label}                (Bearer web) -> {token}  extra read-only
                                                         token, e.g. for an AI assistant (MCP)
  GET  /api/accounts/web/tokens                          (Bearer) list labels + dates (no tokens)
  GET  /api/accounts/mine/summary                        (Bearer) name, counts
  GET  /api/accounts/mine/birds?since=<unix s>           (Bearer) species heard by your devices
  GET  /api/accounts/mine/birds/<latin>                  (Bearer) where you heard it (cell centres)
  GET  /api/accounts/mine/listeners                      (Bearer) your phones/listeners + status

Bearer = a web token (web_sessions, prefix "w_") or an app session from the phone.
Web tokens only work on these routes: they cannot change the profile, link devices or post.
"Yours" = rows signed under your app handle, or under any chain account a device you linked
(user_seats) is bound to (seat_keys). Linking needs the device's own key, so this stays owner-only.
"""
import hashlib, json, re, secrets, threading, time, urllib.parse, urllib.request
from concurrent.futures import ThreadPoolExecutor, wait

import h3
from argon2.exceptions import VerifyMismatchError, InvalidHashError

import hone_accounts as accounts

WEB_S = 180 * 86400
MAX_TOKENS = 10
LATIN = re.compile(r"^[A-Z][a-z]+ [a-z][a-z-]+$")
ONLINE_S = 35 * 60
_db = _lock = None
_pool = ThreadPoolExecutor(6)
_inflight = set()


def init(db, lock):
    global _db, _lock
    _db, _lock = db, lock
    db.executescript("""
    CREATE TABLE IF NOT EXISTS web_sessions(token_hash TEXT PRIMARY KEY, handle TEXT, label TEXT,
      created INTEGER, expires INTEGER);
    CREATE TABLE IF NOT EXISTS web_names(latin TEXT PRIMARY KEY, name TEXT, thumb TEXT, page TEXT, updated INTEGER);
    """)
    db.commit()


def _h(s):
    return hashlib.sha256(s.encode()).hexdigest()


# ── sessions ──────────────────────────────────────────────────────────────────────────────
def _new_token(handle, label):
    tok = "w_" + secrets.token_urlsafe(32)
    now = int(time.time())
    with _lock:
        _db.execute("DELETE FROM web_sessions WHERE expires < ?", (now,))
        old = _db.execute("SELECT token_hash FROM web_sessions WHERE handle=? ORDER BY created DESC", (handle,)).fetchall()
        for (th,) in old[MAX_TOKENS - 1:]:
            _db.execute("DELETE FROM web_sessions WHERE token_hash=?", (th,))
        _db.execute("INSERT INTO web_sessions VALUES(?,?,?,?,?)", (_h(tok), handle, label[:40], now, now + WEB_S))
        _db.commit()
    return tok


def login(b):
    email = accounts._norm_email(b.get("email"))
    w = accounts._login_rate.get(email)
    now = time.time()
    if w and now - w[0] < 3600 and w[1] >= 10:
        raise accounts.Refused("Too many tries. Please wait an hour and try again.")
    with _lock:
        r = _db.execute("SELECT handle, pw FROM users WHERE email=?", (email,)).fetchone()
    ok = False
    if r:
        try:
            ok = accounts.ph.verify(r[1], str(b.get("password", "")))
        except (VerifyMismatchError, InvalidHashError):
            ok = False
    if not ok:
        accounts._login_rate[email] = [w[0], w[1] + 1] if w and now - w[0] < 3600 else [now, 1]
        raise accounts.Refused("Wrong email or password.")
    accounts._login_rate.pop(email, None)
    u = accounts._user(r[0])
    return {"token": _new_token(r[0], "web"), "user": {"name": u["name"], "verified": u["verified"]}}


def _web_handle(headers):
    a = headers.get("Authorization", "")
    if not a.startswith("Bearer w_"):
        return None
    with _lock:
        r = _db.execute("SELECT handle FROM web_sessions WHERE token_hash=? AND expires>?",
                        (_h(a[7:].strip()), int(time.time()))).fetchone()
    if not r:
        raise accounts.AuthError("Please sign in again.")
    return r[0]


def auth(headers):
    """Web token or app session -> handle (read-only routes only)."""
    return _web_handle(headers) or accounts.auth(headers)


_link_rate = {}


def _rl(key, n, per):
    now = time.time()
    w = _link_rate.get(key)
    if not w or now - w[0] > per:
        _link_rate[key] = [now, 1]; return False
    w[1] += 1
    return w[1] > n


def setup_link(body, headers):
    """Email the computer setup link (/ornigo/) to the address given on /me's first visit.
    Fixed text; only a validated invite code and voucher go into the link. Rate limited per
    address (2 per hour, 4 per day) and per IP (6 per hour). Never says whether an account exists."""
    email = accounts._norm_email(body.get("email"))
    ip = headers.get("X-Forwarded-For", "").split(",")[0].strip() or "?"
    if _rl("ip:" + ip, 6, 3600) or _rl("e1:" + _h(email), 2, 3600) or _rl("e24:" + _h(email), 4, 86400):
        raise accounts.Refused("A link was just sent. Please check your email, or try again in an hour.")
    ref = str(body.get("ref") or "").lower()
    v = str(body.get("v") or "")
    q = []
    if accounts.REF.match(ref):
        q.append("ref=" + ref)
    if re.fullmatch(r"[A-Za-z0-9_-]{16,128}", v):
        q.append("v=" + v)
    link = "https://honemesh.net/ornigo/" + ("?" + "&".join(q) if q else "")
    text = ("Hi,\n\nHere's the link to set up Ornigo on a computer that stays on at home:\n\n" + link +
            "\n\nOpen it on that computer. It takes a couple of minutes. Then check your birds on your phone at "
            "https://honemesh.net/me (on iPhone: tap Share, then Add to Home Screen).\n\n"
            "Human conversation is automatically deleted. No audio is ever saved or sent.\n\n"
            "If you didn't ask for this, you can ignore this email.\n\nOrnigo · ornigo@honemesh.net")
    with _lock:
        _db.execute("INSERT INTO mail_outbox(to_email,subject,body,kind,created) VALUES(?,?,?,?,?)",
                    (email, "Set up Ornigo on your computer", text, "setup", int(time.time())))
        _db.commit()
    if accounts._agentmail_key():
        accounts._mail_kick.set()
    return {"ok": True}


def post(path, body, headers):
    if path == "/api/accounts/web/login":
        return login(body)
    if path == "/api/accounts/web/setup-link":
        return setup_link(body, headers)
    h = _web_handle(headers)
    if not h:
        raise accounts.AuthError("Please sign in again.")
    if path == "/api/accounts/web/logout":
        with _lock:
            _db.execute("DELETE FROM web_sessions WHERE token_hash=?", (_h(headers.get("Authorization", "")[7:].strip()),))
            _db.commit()
        return {"ok": True}
    if path == "/api/accounts/web/token":
        label = re.sub(r"[\x00-\x1f<>]", "", str(body.get("label") or "AI assistant"))[:40] or "AI assistant"
        return {"token": _new_token(h, label), "label": label, "read_only": True}
    if path == "/api/accounts/web/revoke":
        with _lock:
            _db.execute("DELETE FROM web_sessions WHERE handle=? AND label<>'web'", (h,)); _db.commit()
        return {"ok": True}
    raise ValueError("not found")


# ── scope ─────────────────────────────────────────────────────────────────────────────────
def _scope(handle):
    with _lock:
        seats = [r[0].lower() for r in _db.execute("SELECT seat_pub FROM user_seats WHERE handle=?", (handle,))]
        accts = {handle}
        for p in seats:
            for (a,) in _db.execute("SELECT account FROM seat_keys WHERE lower(pub)=?", (p,)):
                accts.add(a)
        labels = {p[:16]: l for p, l in _db.execute("SELECT seat_pub, label FROM user_seats WHERE handle=?", (handle,))}
    return sorted(accts), seats, labels


def _in(n):
    return ",".join("?" * n)


# ── names + pictures (Wikipedia summary, cached) ──────────────────────────────────────────
def _fetch_name(latin):
    try:
        s = json.load(urllib.request.urlopen(urllib.request.Request(
            "https://en.wikipedia.org/api/rest_v1/page/summary/" + urllib.parse.quote(latin.replace(" ", "_")),
            headers={"User-Agent": "hone-birds/1 (https://honemesh.net)"}), timeout=6))
        title = s.get("title") or ""
        name = title if title and title.lower() != latin.lower() else None
        thumb = (s.get("thumbnail") or {}).get("source")
        page = ((s.get("content_urls") or {}).get("desktop") or {}).get("page")
    except Exception:
        name = thumb = page = None
    try:
        with _lock:
            _db.execute("INSERT OR REPLACE INTO web_names VALUES(?,?,?,?,?)", (latin, name, thumb, page, int(time.time())))
            _db.commit()
    finally:
        _inflight.discard(latin)


def names(latins, budget_s=5.0):
    out = {}
    with _lock:
        for l in latins:
            r = _db.execute("SELECT name, thumb, page, updated FROM web_names WHERE latin=?", (l,)).fetchone()
            if r and (r[0] or time.time() - r[3] < 86400):
                out[l] = {"name": r[0], "thumb": r[1], "page": r[2]}
    missing = [l for l in latins if l not in out and l not in _inflight]
    futs = []
    for l in missing[:60]:
        _inflight.add(l); futs.append(_pool.submit(_fetch_name, l))
    if futs:
        wait(futs, timeout=budget_s)
        with _lock:
            for l in missing:
                r = _db.execute("SELECT name, thumb, page FROM web_names WHERE latin=?", (l,)).fetchone()
                if r:
                    out[l] = {"name": r[0], "thumb": r[1], "page": r[2]}
    return out


# ── reads ─────────────────────────────────────────────────────────────────────────────────
def _place(cell, res):
    return h3.cell_to_parent(cell, 7) if res > 7 else cell


def _sightings(accts, since_window=0, latin=None):
    sql = (f"SELECT latin, window, cell, res, conf_q8, verified, source, station FROM sightings "
           f"WHERE account IN ({_in(len(accts))}) AND verified IN (1,4) AND window>=?")
    args = [*accts, since_window]
    if latin:
        sql += " AND latin=?"; args.append(latin)
    with _lock:
        return _db.execute(sql + " ORDER BY window DESC LIMIT 20000", args).fetchall()


def birds(handle, args):
    accts, _seats, _l = _scope(handle)
    since = int(args.get("since", 0) or 0)
    rows = _sightings(accts, since // 900 if since else 0)
    sp = {}
    for latin, win, cell, res, conf, ver, src, station in rows:
        e = sp.setdefault(latin, {"latin": latin, "windows": set(), "places": set(), "first": win, "last": win,
                                  "best_q8": 0, "rare": None, "sources": set()})
        e["windows"].add(win); e["places"].add(_place(cell, res))
        e["first"] = min(e["first"], win); e["last"] = max(e["last"], win)
        e["best_q8"] = max(e["best_q8"], conf)
        if ver in (3, 4):
            e["rare"] = "confirmed" if ver == 4 or e["rare"] == "confirmed" else "unconfirmed"
        e["sources"].add(src or "phone")
    nm = names(list(sp))
    out = []
    with _lock:
        for latin, e in sp.items():
            mine = _db.execute(f"SELECT id FROM photos WHERE latin=? AND hidden=0 AND account IN ({_in(len(accts))}) "
                               "ORDER BY t DESC LIMIT 1", (latin, *accts)).fetchone()
            pub = mine or _db.execute("SELECT id FROM photos WHERE latin=? AND hidden=0 AND verified=1 ORDER BY t DESC LIMIT 1",
                                      (latin,)).fetchone()
            n = nm.get(latin) or {}
            photo = {"url": f"/api/birds/photos/{pub[0]}.jpg", "by": "you" if mine else "HONE listener",
                     "license": "UFL"} if pub else ({"url": n["thumb"], "by": "Wikipedia", "page": n.get("page")}
                                                    if n.get("thumb") else None)
            out.append({"latin": latin, "name": n.get("name"), "heard": len(e["windows"]), "places": len(e["places"]),
                        "first": e["first"] * 900, "last": e["last"] * 900, "best_conf": round(e["best_q8"] / 255, 2),
                        "rare": e["rare"], "sources": sorted(e["sources"]), "photo": photo,
                        "call": f"/api/birds/call/{urllib.parse.quote(latin)}"})
    out.sort(key=lambda x: -x["last"])
    return {"since": since, "species": out, "count": len(out)}


def bird(handle, latin):
    if not LATIN.match(latin):
        raise ValueError("bad species name")
    accts, _s, _l = _scope(handle)
    pl, days = {}, set()
    for _latin, win, cell, res, conf, ver, src, station in _sightings(accts, 0, latin):
        days.add(win * 900 // 86400)
        c = _place(cell, res)
        e = pl.setdefault(c, {"windows": set(), "last": win, "res": min(res, 7)})
        e["windows"].add(win); e["last"] = max(e["last"], win)
    places = []
    for c, e in pl.items():
        lat, lng = h3.cell_to_latlng(c)
        places.append({"lat": round(lat, 5), "lng": round(lng, 5), "res": e["res"], "heard": len(e["windows"]),
                       "last": e["last"] * 900})
    places.sort(key=lambda x: -x["last"])
    n = names([latin]).get(latin) or {}
    return {"latin": latin, "name": n.get("name"), "places": places, "count": len(places),
            "days": [d * 86400 for d in sorted(days, reverse=True)[:90]]}


def _kind(p):
    k = str(p.get("device_kind") or p.get("kind_device") or "").lower()
    if k:
        return k
    if p.get("listener") or p.get("station") or p.get("cameras"):
        return "camera listener"
    if "android" in p:
        return "phone"
    return "device"


def listeners(handle):
    accts, seats, labels = _scope(handle)
    devs = {s[:16] for s in seats}
    now = int(time.time())
    with _lock:
        rows = _db.execute(f"SELECT account, device, received, payload FROM device_latest WHERE account IN ({_in(len(accts))})"
                           + (f" OR device IN ({_in(len(devs))})" if devs else ""), (*accts, *devs)).fetchall()
        stations = _db.execute(f"SELECT station, MAX(window), COUNT(DISTINCT latin) FROM sightings WHERE station IS NOT NULL "
                               f"AND account IN ({_in(len(accts))}) GROUP BY station", accts).fetchall()
    out, seen = [], set()
    for acct, dev, received, payload in rows:
        try:
            p = json.loads(payload)
        except Exception:
            p = {}
        app = p.get("app") or {}
        b = p.get("birds") or {}
        seen.add(dev)
        out.append({"device": dev, "label": labels.get(dev) or p.get("label") or p.get("name") or None,
                    "kind": _kind(p), "online": now - received < ONLINE_S, "last_report": received,
                    "detections_today": b.get("detections_today"),
                    "version": app.get("version_name") or (p["listener"].get("version") if isinstance(p.get("listener"), dict) else None)})
    for st, last_win, n in stations:
        if st in seen:
            continue
        seen.add(st)
        out.append({"device": st, "label": labels.get(st), "kind": "camera listener",
                    "online": now - last_win * 900 < ONLINE_S, "last_report": last_win * 900, "species": n})
    for d in devs - seen:
        out.append({"device": d, "label": labels.get(d), "kind": "device", "online": False, "last_report": None})
    out.sort(key=lambda x: (not x["online"], -(x["last_report"] or 0)))
    return {"listeners": out, "listening_now": sum(1 for x in out if x["online"])}


def summary(handle):
    u = accounts._user(handle)
    b = birds(handle, {})
    li = listeners(handle)
    last = max([s["last"] for s in b["species"]] or [None]) if b["species"] else None
    accts, _s, _l = _scope(handle)
    places = {_place(c, r) for _x, _w, c, r, *_rest in _sightings(accts)}
    return {"name": u["name"], "verified": u["verified"], "species": b["count"], "places": len(places),
            "listeners": len(li["listeners"]), "listening_now": li["listening_now"], "last_heard": last}


def tokens(handle):
    with _lock:
        rows = _db.execute("SELECT label, created, expires FROM web_sessions WHERE handle=? AND expires>? ORDER BY created DESC",
                           (handle, int(time.time()))).fetchall()
    return [{"label": r[0], "created": r[1], "expires": r[2]} for r in rows]


def get(path, args, headers):
    h = auth(headers)
    if path == "/api/accounts/mine/summary":
        return summary(h)
    if path == "/api/accounts/mine/birds":
        return birds(h, args)
    m = re.match(r"^/api/accounts/mine/birds/(.+)$", path)
    if m:
        return bird(h, urllib.parse.unquote(m.group(1)))
    if path == "/api/accounts/mine/listeners":
        return listeners(h)
    if path == "/api/accounts/web/tokens":
        return tokens(h)
    raise LookupError("not found")
