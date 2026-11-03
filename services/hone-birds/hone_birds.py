#!/usr/bin/env python3
"""HONE birds indexer (v1, off-chain). Public bird sightings and user photos.

Privacy model (decided by Josh, 2026-11-03):
  - Sightings arrive already coarsened by the phone: ~5 km (H3 res 7) inside the user's home
    area, otherwise as precise as the GPS fix allows.
  - Every sighting and photo is signed by the account's posting key and checked against the
    chain's key table (posting_keys.json, pushed from a node). No anonymous submissions.
    The account is shown publicly only if the user opted in ("show my name").
  - Photos: re-encoded on the phone (no EXIF), published under HONE's license (UFL).

API (all JSON):
  POST /api/birds/sightings   {"payload": "<json>", "pub"?: hex, "sig"?: hex}
       payload = {"v":1,"sid","window","cell","species":[{"name","conf_q8","frames"}],
                  "model","account"?}
  GET  /api/birds/sightings?since=<window>&limit=
  GET  /api/birds/species                 -> per-species totals
  GET  /api/birds/species/<latin>         -> recent sightings + photos
  GET  /api/birds/map?res=3|5|7[&species=] -> counts per coarse cell
  POST /api/birds/photos      {"payload": "<json>", "jpeg_b64", "pub"?, "sig"?}
       payload = {"v":1,"latin","t","license":"UFL","account"?}
  GET  /api/birds/photos/<id>.jpg
"""
import base64, hashlib, json, os, re, sqlite3, sys, threading, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, unquote, urlparse

import h3
from nacl.signing import VerifyKey

DATA = os.environ.get("HONE_BIRDS_DATA", "/var/lib/hone-birds")
PORT = int(os.environ.get("HONE_BIRDS_PORT", "9460"))
MAX_BODY = 4 * 1024 * 1024
LATIN = re.compile(r"^[A-Z][a-z]+ [a-z][a-z-]+$")
os.makedirs(os.path.join(DATA, "photos"), exist_ok=True)

db = sqlite3.connect(os.path.join(DATA, "birds.db"), check_same_thread=False)
db.executescript("""
PRAGMA journal_mode=WAL;
CREATE TABLE IF NOT EXISTS sightings(
  sid TEXT PRIMARY KEY, window INTEGER, cell TEXT, res INTEGER, latin TEXT, conf_q8 INTEGER,
  frames INTEGER, model TEXT, account TEXT NOT NULL, pub TEXT NOT NULL, received INTEGER,
  verified INTEGER, show_name INTEGER);
CREATE INDEX IF NOT EXISTS s_win ON sightings(window);
CREATE INDEX IF NOT EXISTS s_latin ON sightings(latin);
CREATE TABLE IF NOT EXISTS photos(
  id TEXT PRIMARY KEY, latin TEXT, t INTEGER, license TEXT, account TEXT NOT NULL, pub TEXT NOT NULL,
  sha256 TEXT, received INTEGER, hidden INTEGER DEFAULT 0, verified INTEGER, show_name INTEGER,
  ufl_agreed_ms INTEGER);
CREATE INDEX IF NOT EXISTS p_latin ON photos(latin);
""")
lock = threading.Lock()
rate = {}  # ip -> [window_start, count]


def limited(ip, n=120, per=3600):
    now = time.time()
    w = rate.get(ip)
    if not w or now - w[0] > per:
        rate[ip] = [now, 1]; return False
    w[1] += 1
    return w[1] > n


KEYS_FILE = os.path.join(DATA, "posting_keys.json")  # {account: posting_pub_hex}, pushed from a node
_keys = {"mtime": 0, "map": {}}


def posting_key(account):
    try:
        m = os.path.getmtime(KEYS_FILE)
        if m != _keys["mtime"]:
            _keys["map"] = json.load(open(KEYS_FILE)); _keys["mtime"] = m
    except OSError:
        pass
    return _keys["map"].get(account)


def verify(body):
    """Every submission must be signed by the account's posting key (Josh, 2026-11-03).
    Returns (payload, pub, verified): verified=1 when pub matches the account's posting key
    on chain; otherwise the row is held back (not public) until the key table catches up."""
    payload_s = body["payload"]
    payload = json.loads(payload_s)
    pub, sig, acct = body.get("pub"), body.get("sig"), payload.get("account")
    if not (acct and pub and sig):
        raise ValueError("account, posting-key pub and signature required")
    VerifyKey(bytes.fromhex(pub)).verify(payload_s.encode(), bytes.fromhex(sig))
    known = posting_key(acct)
    if known is not None and known.lower() != pub.lower():
        raise ValueError("pub is not this account's posting key")
    return payload, pub, 1 if known else 0


def add_sighting(body):
    p, pub, verified = verify(body)
    cell = p["cell"]
    if not h3.is_valid_cell(cell):
        raise ValueError("bad cell")
    res = h3.get_resolution(cell)
    sid = str(p["sid"])[:64]
    rows = []
    for i, s in enumerate(p["species"][:20]):
        name = s["name"]
        if not LATIN.match(name):
            continue
        rows.append((f"{sid}:{i}", int(p["window"]), cell, res, name, int(s["conf_q8"]),
                     int(s.get("frames", 1)), str(p.get("model", ""))[:64],
                     p["account"], pub, int(time.time()), verified, 1 if p.get("show_name") else 0))
    with lock:
        db.executemany("INSERT OR IGNORE INTO sightings VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)", rows)
        db.commit()
    return {"ok": True, "stored": len(rows)}


def add_photo(body):
    p, pub, verified = verify(body)
    if not LATIN.match(p["latin"]):
        raise ValueError("bad species")
    if p.get("license") != "UFL" or not p.get("ufl_agreed_ms"):
        raise ValueError("photos are published under UFL; the user must agree first")
    jpeg = base64.b64decode(body["jpeg_b64"])
    if jpeg[:3] != b"\xff\xd8\xff" or len(jpeg) > 3_500_000:
        raise ValueError("jpeg only, max 3.5 MB")
    if b"Exif" in jpeg[:64]:
        raise ValueError("strip EXIF before upload")
    h = hashlib.sha256(jpeg).hexdigest()
    pid = h[:24]
    with open(os.path.join(DATA, "photos", pid + ".jpg"), "wb") as f:
        f.write(jpeg)
    with lock:
        db.execute("INSERT OR IGNORE INTO photos(id,latin,t,license,account,pub,sha256,received,verified,show_name,ufl_agreed_ms)"
                   " VALUES(?,?,?,?,?,?,?,?,?,?,?)",
                   (pid, p["latin"], int(p["t"]), "UFL", p["account"], pub, h, int(time.time()),
                    verified, 1 if p.get("show_name") else 0, int(p["ufl_agreed_ms"])))
        db.commit()
    return {"ok": True, "id": pid, "url": f"/api/birds/photos/{pid}.jpg"}


def q(sql, args=()):
    with lock:
        cur = db.execute(sql, args)
        cols = [c[0] for c in cur.description]
        return [dict(zip(cols, r)) for r in cur.fetchall()]


def public_sighting(r):
    # never return the signing key of anonymous rows (there is none) or receive times
    out = {k: r[k] for k in ("window", "cell", "res", "latin", "conf_q8", "frames")}
    out["account"] = r["account"] if r["show_name"] else None   # verified either way; name only on opt-in
    return out


class H(BaseHTTPRequestHandler):
    server_version = "hone-birds/1"

    def _send(self, code, obj=None, raw=None, ctype="application/json"):
        data = raw if raw is not None else json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Access-Control-Allow-Origin", "*")
        if raw is not None:
            self.send_header("Cache-Control", "public, max-age=86400")
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, fmt, *a):  # no IPs in logs
        sys.stderr.write("%s %s\n" % (self.command, self.path.split("?")[0]))

    def do_POST(self):
        ip = self.headers.get("X-Forwarded-For", self.client_address[0]).split(",")[0]
        if limited(ip):
            return self._send(429, {"error": "slow down"})
        n = int(self.headers.get("Content-Length", 0))
        if n <= 0 or n > MAX_BODY:
            return self._send(413, {"error": "body size"})
        try:
            body = json.loads(self.rfile.read(n))
            path = urlparse(self.path).path
            if path == "/api/birds/sightings":
                return self._send(200, add_sighting(body))
            if path == "/api/birds/photos":
                return self._send(200, add_photo(body))
            return self._send(404, {"error": "not found"})
        except Exception as e:
            return self._send(400, {"error": str(e)[:200]})

    def do_GET(self):
        u = urlparse(self.path)
        a = {k: v[0] for k, v in parse_qs(u.query).items()}
        path = u.path
        try:
            if path == "/api/birds/sightings":
                rows = q("SELECT * FROM sightings WHERE verified=1 AND window>=? ORDER BY window DESC LIMIT ?",
                         (int(a.get("since", 0)), min(int(a.get("limit", 500)), 2000)))
                return self._send(200, [public_sighting(r) for r in rows])
            if path == "/api/birds/species":
                rows = q("""SELECT latin, COUNT(*) sightings, COUNT(DISTINCT cell) cells,
                            MAX(window) last_window, MIN(window) first_window
                            FROM sightings WHERE verified=1 GROUP BY latin ORDER BY sightings DESC""")
                photos = {r["latin"]: r["n"] for r in q("SELECT latin, COUNT(*) n FROM photos WHERE hidden=0 AND verified=1 GROUP BY latin")}
                for r in rows:
                    r["photos"] = photos.get(r["latin"], 0)
                return self._send(200, rows)
            m = re.match(r"^/api/birds/species/(.+)$", path)
            if m:
                latin = unquote(m.group(1))
                s = q("SELECT * FROM sightings WHERE verified=1 AND latin=? ORDER BY window DESC LIMIT 500", (latin,))
                p = q("SELECT id, t, license, CASE WHEN show_name=1 THEN account END account FROM photos"
                      " WHERE latin=? AND hidden=0 AND verified=1 ORDER BY t DESC LIMIT 100", (latin,))
                for r in p:
                    r["url"] = f"/api/birds/photos/{r['id']}.jpg"
                return self._send(200, {"latin": latin, "sightings": [public_sighting(r) for r in s], "photos": p})
            if path == "/api/birds/map":
                res = int(a.get("res", 5))
                if res not in (3, 5, 7, 9):
                    raise ValueError("res must be 3, 5, 7 or 9")
                sql = "SELECT cell, res, latin FROM sightings WHERE verified=1 AND window>=?"
                args = [int(a.get("since", 0))]
                if a.get("species"):
                    sql += " AND latin=?"; args.append(a["species"])
                agg = {}
                for r in q(sql, args):
                    c = h3.cell_to_parent(r["cell"], res) if r["res"] > res else r["cell"]
                    e = agg.setdefault(c, {"cell": c, "sightings": 0, "species": set()})
                    e["sightings"] += 1; e["species"].add(r["latin"])
                out = [{"cell": e["cell"], "sightings": e["sightings"], "species": len(e["species"])} for e in agg.values()]
                return self._send(200, out)
            m = re.match(r"^/api/birds/photos/([0-9a-f]{24})\.jpg$", path)
            if m:
                f = os.path.join(DATA, "photos", m.group(1) + ".jpg")
                hidden = q("SELECT hidden, verified FROM photos WHERE id=?", (m.group(1),))
                if not os.path.exists(f) or not hidden or hidden[0]["hidden"] or not hidden[0]["verified"]:
                    return self._send(404, {"error": "not found"})
                with open(f, "rb") as fh:
                    return self._send(200, raw=fh.read(), ctype="image/jpeg")
            if path == "/api/birds/health":
                return self._send(200, {"ok": True, "sightings": q("SELECT COUNT(*) n FROM sightings")[0]["n"],
                                        "photos": q("SELECT COUNT(*) n FROM photos")[0]["n"]})
            return self._send(404, {"error": "not found"})
        except Exception as e:
            return self._send(400, {"error": str(e)[:200]})


if __name__ == "__main__":
    ThreadingHTTPServer(("127.0.0.1", PORT), H).serve_forever()
