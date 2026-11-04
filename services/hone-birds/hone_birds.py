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
  Envelopes may also carry "key": "seat" (device seat key): such rows are held back
  (verified=0) until the seat is bound to the account (see hone_devices.py).
  Device status and Flipper frames: hone_devices.py (/api/devices/*, /api/flipper/*).
  Occupancy scores (hone_occupancy.py):
  POST /api/birds/windows     signed listening window: effort at res 7, detections at res <= 9
       payload = {"v":1,"sid","window","cell","frames","human","silenced","tz_min","source",
                  "station"?,"species":[{"name","cell","events","conf_q8"}],"account"}
  POST /api/birds/labels      signed, opt-in yes/no answers: {"v":1,"opt_in":true,"labels":[...],"account"}
  GET  /api/birds/occupancy?cell=<res<=7 cell>   local view (status words; exact cells for map tiers)
  GET  /api/birds/occupancy/map?res=5|7          likely/confirmed/rare species per cell
  GET  /api/birds/occupancy/changes?since=<t>    arrivals/departures (indexer only, no chain)
"""
import base64, hashlib, hmac, io, json, os, re, sqlite3, sys, threading, time, urllib.error, urllib.parse, urllib.request
from PIL import Image
from concurrent.futures import ThreadPoolExecutor
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, unquote, urlparse

import h3
from nacl.signing import VerifyKey

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import hone_devices as devices  # noqa: E402  device status + Flipper frames
import hone_accounts as accounts  # noqa: E402  app accounts (email sign-in)
import hone_occupancy as occupancy  # noqa: E402  per-cell occupancy scores

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
db.executescript("""
CREATE TABLE IF NOT EXISTS ranges(cell TEXT PRIMARY KEY, species TEXT, updated INTEGER);
CREATE TABLE IF NOT EXISTS gbif_names(key INTEGER PRIMARY KEY, name TEXT);
CREATE TABLE IF NOT EXISTS wiki_hash(latin TEXT PRIMARY KEY, dhash TEXT);
""")
for col in ("dhash TEXT", "evidence TEXT", "evidence_ok INTEGER DEFAULT 0"):
    try:
        db.execute("ALTER TABLE photos ADD COLUMN " + col)
    except sqlite3.OperationalError:
        pass  # already there
lock = threading.RLock()  # re-entrant: occupancy scoring reads ranges under the same lock
rate = {}  # ip -> [window_start, count]

# ── species range (GBIF occurrences within RANGE_KM of an H3 res-3 cell) ────────────────
# Perch picks from ~15,000 species worldwide; restricting to species actually recorded near
# the phone removes most false detections. Data: GBIF.org occurrence facets (counts only).
RANGE_KM = 250
RARE_MIN_Q8 = 217  # 85 %
RANGE_TTL = 30 * 86400
GBIF_GROUPS = {"classKey": [212, 131, 359], "orderKey": [1458]}  # birds, amphibians, mammals, Orthoptera
range_locks = {}


def gbif(url):
    req = urllib.request.Request(url, headers={"User-Agent": "hone-birds/1 (https://honemesh.net)"})
    return json.load(urllib.request.urlopen(req, timeout=60))


def gbif_name(key):
    with lock:
        r = db.execute("SELECT name FROM gbif_names WHERE key=?", (key,)).fetchone()
    if r:
        return r[0]
    try:
        n = gbif(f"https://api.gbif.org/v1/species/{key}").get("canonicalName")
    except Exception:
        return None
    if n:
        with lock:
            db.execute("INSERT OR REPLACE INTO gbif_names VALUES(?,?)", (key, n)); db.commit()
    return n


def species_range(cell):
    """Species names recorded within RANGE_KM of the res-3 cell's centre (cached 30 days)."""
    if h3.get_resolution(cell) != 3:
        cell = area(cell)
    with lock:
        r = db.execute("SELECT species, updated FROM ranges WHERE cell=?", (cell,)).fetchone()
    if r and time.time() - r[1] < RANGE_TTL:
        return cell, json.loads(r[0])
    lk = range_locks.setdefault(cell, threading.Lock())
    with lk:
        with lock:
            r = db.execute("SELECT species, updated FROM ranges WHERE cell=?", (cell,)).fetchone()
        if r and time.time() - r[1] < RANGE_TTL:
            return cell, json.loads(r[0])
        lat, lng = h3.cell_to_latlng(cell)
        keys = set()
        for field, vals in GBIF_GROUPS.items():
            for v in vals:
                d = gbif(f"https://api.gbif.org/v1/occurrence/search?geo_distance={lat:.4f},{lng:.4f},{RANGE_KM}km"
                         f"&{field}={v}&facet=speciesKey&facetLimit=5000&limit=0")
                for c in d["facets"][0]["counts"] if d.get("facets") else []:
                    if c["count"] >= 2:
                        keys.add(int(c["name"]))
        with ThreadPoolExecutor(8) as ex:
            names = sorted({n for n in ex.map(gbif_name, keys) if n})
        with lock:
            db.execute("INSERT OR REPLACE INTO ranges VALUES(?,?,?)", (cell, json.dumps(names), int(time.time())))
            db.commit()
        return cell, names


def area(cell):
    """Res-3 area of a cell by its centre point (same rule the phone uses: lat/lon -> res 3).
    Not cell_to_parent: H3 children aren't strictly inside their parent near edges."""
    return h3.latlng_to_cell(*h3.cell_to_latlng(cell), 3)


def in_range(cell, latin):
    """True/False if the range for this cell is cached; None if not known yet."""
    c3 = area(cell)
    with lock:
        r = db.execute("SELECT species FROM ranges WHERE cell=?", (c3,)).fetchone()
    return None if not r else latin in set(json.loads(r[0]))


# ── reference calls (xeno-canto API v3, metadata only) ─────────────────────────────────
# The app streams audio from xeno-canto's own file URL; nothing is rehosted here. These
# recordings are reference listening only: never used for rewards, scoring, training,
# anti-gaming or marketing. Key: XC_API_KEY in /etc/hone-birds.env (never logged).
db.execute("CREATE TABLE IF NOT EXISTS xc_calls(latin TEXT PRIMARY KEY, body TEXT, updated INTEGER)")
XC_TTL, XC_NEG_TTL, XC_MAX = 30 * 86400, 86400, 3
xc_locks = {}


def xc_license(url):
    """(rank, short name, https url): CC0, BY, BY-SA, BY-ND, then NC variants."""
    u = (url or "").lower()
    if "zero" in u or "publicdomain" in u:
        name = "CC0"
    else:
        m = re.search(r"licenses/([a-z-]+)/([0-9.]+)", u)
        name = ("CC " + m.group(1).upper() + " " + m.group(2)) if m else (url or "unknown")
    order = ["CC0", "CC BY ", "CC BY-SA", "CC BY-ND", "CC BY-NC ", "CC BY-NC-SA", "CC BY-NC-ND"]
    rank = next((i for i, o in enumerate(order) if (name + " ").startswith(o)), len(order))
    full = url if (url or "").startswith("http") else ("https:" + url if (url or "").startswith("//") else url)
    return rank, name, full


def xc_fetch(latin):
    key = os.environ.get("XC_API_KEY", "").strip()
    if not key:
        raise LookupError("xeno-canto API key not configured")
    gen, sp = latin.split(" ", 1)
    recs = []
    for query in (f'gen:{gen} sp:{sp}', f'sp:"{latin}"'):
        url = "https://xeno-canto.org/api/3/recordings?" + urllib.parse.urlencode(
            {"query": query, "per_page": 100, "key": key})
        req = urllib.request.Request(url, headers={"User-Agent": "hone-birds/1 (https://honemesh.net)"})
        try:
            data = json.load(urllib.request.urlopen(req, timeout=30))
        except urllib.error.HTTPError as e:
            raise RuntimeError(f"xeno-canto HTTP {e.code}") from None  # no URL: it carries the key
        recs = [r for r in data.get("recordings", [])
                if f"{r.get('gen', '')} {r.get('sp', '')}".lower() == latin.lower() and r.get("file")]
        if recs:
            break
    out = []
    for r in recs:
        rank, lic, lic_url = xc_license(r.get("lic"))
        page = r.get("url") or f"https://xeno-canto.org/{r.get('id')}"
        out.append({"xc_id": str(r.get("id")), "recordist": r.get("rec") or "unknown", "license": lic,
                    "license_url": lic_url, "quality": r.get("q") or "", "file_url": r["file"],
                    "page_url": "https:" + page if page.startswith("//") else page,
                    "length": r.get("length") or "", "type": r.get("type") or "", "country": r.get("cnt") or "",
                    "_rank": (rank, (r.get("q") or "z").upper() if (r.get("q") or "") in "ABCDE" else "Z")})
    out.sort(key=lambda x: x["_rank"])
    for x in out:
        x.pop("_rank")
    return out[:XC_MAX]


def xc_calls(latin):
    if not LATIN.match(latin):
        raise ValueError("bad species name")
    now = int(time.time())
    with lock:
        r = db.execute("SELECT body, updated FROM xc_calls WHERE latin=?", (latin,)).fetchone()
    if r:
        body = json.loads(r[0])
        if now - r[1] < (XC_TTL if body else XC_NEG_TTL):
            return body
    with xc_locks.setdefault(latin, threading.Lock()):
        body = xc_fetch(latin)
        with lock:
            db.execute("INSERT OR REPLACE INTO xc_calls VALUES(?,?,?)", (latin, json.dumps(body), now)); db.commit()
    return body


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
    if body.get("key") == "seat":
        p, _ps, _k, pub, verified = devices.verify_env(body)
        return p, pub, verified
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


devices.init(db, lock, posting_key)
accounts.init(db, lock, devices.bind)
for col in ("source TEXT", "tier TEXT", "station TEXT"):
    try:
        db.execute("ALTER TABLE sightings ADD COLUMN " + col)
    except sqlite3.OperationalError:
        pass  # already there
if os.environ.get("HONE_SEALS", "1") == "1":
    devices.start_seals()

# ── occupancy: season months per (res-3 area, species) from GBIF, cache only on the hot path ──
db.execute("CREATE TABLE IF NOT EXISTS gbif_months(cell TEXT, latin TEXT, months TEXT, updated INTEGER, PRIMARY KEY(cell, latin))")
db.commit()
_months_q = set()
_months_pool = ThreadPoolExecutor(2)


def _fetch_months(c3, latin):
    try:
        m = gbif("https://api.gbif.org/v1/species/match?strict=true&name=" + urllib.parse.quote(latin))
        key = m.get("usageKey")
        months = None
        if key:
            lat, lng = h3.cell_to_latlng(c3)
            d = gbif(f"https://api.gbif.org/v1/occurrence/search?geo_distance={lat:.4f},{lng:.4f},{RANGE_KM}km"
                     f"&speciesKey={key}&facet=month&facetLimit=12&limit=0")
            counts = {int(c["name"]): c["count"] for c in (d["facets"][0]["counts"] if d.get("facets") else [])}
            if sum(counts.values()) >= 20:
                top = max(counts.values())
                months = sorted(k for k, v in counts.items() if v >= 0.1 * top)
        with lock:
            db.execute("INSERT OR REPLACE INTO gbif_months VALUES(?,?,?,?)", (c3, latin, json.dumps(months), int(time.time())))
            db.commit()
    except Exception as e:
        sys.stderr.write("months %s: %s\n" % (latin, str(e)[:80]))
    finally:
        _months_q.discard((c3, latin))


def season_months(cell, latin):
    c3 = area(cell)
    with lock:
        r = db.execute("SELECT months, updated FROM gbif_months WHERE cell=? AND latin=?", (c3, latin)).fetchone()
    if r and time.time() - r[1] < RANGE_TTL:
        v = json.loads(r[0])
        return set(v) if v else None
    if (c3, latin) not in _months_q:
        _months_q.add((c3, latin)); _months_pool.submit(_fetch_months, c3, latin)
    return None


def photo_near(cell, latin):
    """A verified photo of the species by an account that listened in this res-7 area."""
    c7 = h3.cell_to_parent(cell, 7) if h3.get_resolution(cell) > 7 else cell
    with lock:
        return db.execute("SELECT 1 FROM photos p WHERE p.latin=? AND p.verified=1 AND p.hidden=0 AND EXISTS("
                          "SELECT 1 FROM occ_windows w WHERE w.account=p.account AND w.cell7=?) LIMIT 1",
                          (latin, c7)).fetchone() is not None


occupancy.init(db, lock, in_range, months=season_months, photo_near=photo_near)
if not db.execute("SELECT 1 FROM occ_state LIMIT 1").fetchone():
    # First start with occupancy: score the sightings already stored (positive evidence only).
    for _r in db.execute("SELECT cell, latin, conf_q8, frames, window, account, station, verified FROM sightings"
                         " WHERE verified IN (1,3,4) ORDER BY window").fetchall():
        occupancy.ingest_sighting(_r[0], _r[1], _r[2], _r[3], _r[4], occupancy.listener_of(_r[5]), _r[7])


def add_window(body):
    p, pub, verified = verify(body)
    return occupancy.ingest_window(p, str(p["account"]), verified)


def add_labels(body):
    p, pub, verified = verify(body)
    if p.get("opt_in") is not True:
        raise ValueError("labels are shared only after the user opts in")
    if verified != 1:
        return {"ok": True, "stored": 0, "held": True}
    return occupancy.add_labels(str(p["account"]), p.get("labels"))


def add_sighting(body):
    p, pub, verified = verify(body)
    cell = p["cell"]
    if not h3.is_valid_cell(cell):
        raise ValueError("bad cell")
    res = h3.get_resolution(cell)
    sid = str(p["sid"])[:64]
    source = p.get("source") if p.get("source") in ("phone", "station") else "phone"
    tier = p.get("tier") if p.get("tier") in ("A", "B", "C") else None
    station = str(p.get("station", ""))[:16] if re.fullmatch(r"[0-9a-f]{16}", str(p.get("station", ""))) else None
    rows = []
    for i, s in enumerate(p["species"][:20]):
        name = s["name"]
        if not LATIN.match(name):
            continue
        v = verified
        if v and in_range(cell, name) is False:
            # Not recorded in this region (GBIF). Vagrants happen: very confident reports are
            # published flagged rare (3 = unconfirmed); weaker ones are held back (2).
            v = 3 if int(s["conf_q8"]) >= RARE_MIN_Q8 else 2
        rows.append((f"{sid}:{i}", int(p["window"]), cell, res, name, int(s["conf_q8"]),
                     int(s.get("frames", 1)), str(p.get("model", ""))[:64],
                     p["account"], pub, int(time.time()), v, 1 if p.get("show_name") else 0, source, tier, station))
    first = []
    a5 = h3.cell_to_parent(cell, 5) if res > 5 else cell
    with lock:
        for r in rows:
            if r[11] not in (1, 3, 4):
                continue
            prior = db.execute("SELECT cell, res FROM sightings WHERE latin=? AND verified IN (1,3,4) AND account<>? LIMIT 5000",
                               (r[4], r[8])).fetchall()
            mine = db.execute("SELECT 1 FROM sightings WHERE latin=? AND account=? LIMIT 1", (r[4], r[8])).fetchone()
            if not mine and not any((h3.cell_to_parent(c, 5) if rs > 5 else c) == a5 for c, rs in prior):
                first.append(r[4])
        db.executemany("INSERT OR IGNORE INTO sightings(sid,window,cell,res,latin,conf_q8,frames,model,account,pub,received,"
                       "verified,show_name,source,tier,station) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)", rows)
        # Occupancy: phones from v8 send their evidence as listening windows ("occ": 1); older
        # phones and camera stations feed it from sightings (positive evidence only).
        if not p.get("occ"):
            for r in rows:
                occupancy.ingest_sighting(r[2], r[4], r[5], r[6], r[1], occupancy.listener_of(r[8]), r[11])
        # A rare report is confirmed (4) when a different account reports the same species
        # in the same res-3 area within 72 h (288 windows).
        for r in rows:
            if r[11] != 3:
                continue
            c3 = area(r[2])
            others = [o for o in db.execute(
                "SELECT sid, cell, res FROM sightings WHERE latin=? AND verified IN (3,4) AND account<>? AND ABS(window-?)<=288",
                (r[4], r[8], r[1])).fetchall()
                if area(o[1]) == c3]
            if others:
                db.executemany("UPDATE sightings SET verified=4 WHERE sid=?", [(o[0],) for o in others] + [(r[0],)])
        db.commit()
    return {"ok": True, "stored": len(rows), "first_in_area": first}


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
    # Anti-gaming: near-duplicates of any HONE photo or of the species' Wikipedia image are refused.
    dh = dhash(jpeg)
    with lock:
        known = [r[0] for r in db.execute("SELECT dhash FROM photos WHERE dhash IS NOT NULL AND id<>?", (pid,))]
    wiki = wiki_dhash(p["latin"])
    if any(hamming(dh, k) <= DUP_BITS for k in known + ([wiki] if wiki else [])):
        raise ValueError("this photo matches an existing image")
    ev = p.get("evidence") if isinstance(p.get("evidence"), dict) else None
    ev_ok = evidence_ok(ev)
    store_photo(pid, jpeg)
    with lock:
        db.execute("INSERT OR IGNORE INTO photos(id,latin,t,license,account,pub,sha256,received,verified,show_name,ufl_agreed_ms,"
                   "dhash,evidence,evidence_ok) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                   (pid, p["latin"], int(p["t"]), "UFL", p["account"], pub, h, int(time.time()),
                    verified, 1 if p.get("show_name") else 0, int(p["ufl_agreed_ms"]),
                    dh, json.dumps(ev) if ev else None, 1 if ev_ok else 0))
        # An in-app photo with good evidence confirms that account's rare report of the species (±72 h).
        if ev_ok and verified:
            w = int(p["t"]) // 900_000
            db.execute("UPDATE sightings SET verified=4 WHERE verified=3 AND latin=? AND account=? AND ABS(window-?)<=288",
                       (p["latin"], p["account"], w))
        db.commit()
    return {"ok": True, "id": pid, "url": f"/api/birds/photos/{pid}.jpg", "evidence_ok": ev_ok}


# ── photo evidence, duplicates, storage ──────────────────────────────────────────────────
DUP_BITS = 6


def dhash(jpeg, size=8):
    im = Image.open(io.BytesIO(jpeg)).convert("L").resize((size + 1, size), Image.LANCZOS)
    px = list(im.getdata())
    bits = 0
    for r in range(size):
        for c in range(size):
            bits = (bits << 1) | (1 if px[r * (size + 1) + c] > px[r * (size + 1) + c + 1] else 0)
    return f"{bits:016x}"


def hamming(a, b):
    return bin(int(a, 16) ^ int(b, 16)).count("1")


def wiki_dhash(latin):
    with lock:
        r = db.execute("SELECT dhash FROM wiki_hash WHERE latin=?", (latin,)).fetchone()
    if r:
        return r[0]
    h = None
    try:
        s = json.load(urllib.request.urlopen(urllib.request.Request(
            "https://en.wikipedia.org/api/rest_v1/page/summary/" + urllib.parse.quote(latin.replace(" ", "_")),
            headers={"User-Agent": "hone-birds/1 (https://honemesh.net)"}), timeout=20))
        src = (s.get("thumbnail") or {}).get("source")
        if src:
            h = dhash(urllib.request.urlopen(urllib.request.Request(src, headers={"User-Agent": "hone-birds/1"}), timeout=20).read())
    except Exception:
        return None
    with lock:
        db.execute("INSERT OR REPLACE INTO wiki_hash VALUES(?,?)", (latin, h)); db.commit()
    return h


def evidence_ok(ev):
    """In-app capture with plausible distance. Parallax/motion are recorded but not yet enforced
    (thresholds to be set from real examples)."""
    if not ev or not ev.get("in_app") or int(ev.get("frames", 0)) < 2:
        return False
    focus_m, zoom = ev.get("focus_m"), float(ev.get("zoom") or 1.0)
    # a bird filling the frame from under ~0.8 m without zoom is almost always a screen or a print
    if focus_m is not None and float(focus_m) < 0.8 and zoom < 2.0:
        return False
    return True


S4 = None
if os.environ.get("S4_BUCKET"):
    import boto3
    from botocore.config import Config
    S4 = boto3.client("s3", endpoint_url=os.environ["S4_ENDPOINT"],
                      aws_access_key_id=os.environ["S4_ACCESS_KEY"], aws_secret_access_key=os.environ["S4_SECRET_KEY"],
                      region_name=os.environ.get("S4_REGION", "eu-central-1"),
                      config=Config(s3={"addressing_style": "path"}, signature_version="s3v4"))
S4_PREFIX = os.environ.get("S4_PREFIX", "birds/photos/")


def store_photo(pid, jpeg):
    """MEGA S4 when configured (source of truth), plus the local cache used for serving."""
    if S4:
        S4.put_object(Bucket=os.environ["S4_BUCKET"], Key=S4_PREFIX + pid + ".jpg", Body=jpeg, ContentType="image/jpeg")
    with open(os.path.join(DATA, "photos", pid + ".jpg"), "wb") as f:
        f.write(jpeg)


def load_photo(pid):
    f = os.path.join(DATA, "photos", pid + ".jpg")
    if os.path.exists(f):
        with open(f, "rb") as fh:
            return fh.read()
    if S4:
        b = S4.get_object(Bucket=os.environ["S4_BUCKET"], Key=S4_PREFIX + pid + ".jpg")["Body"].read()
        with open(f, "wb") as fh:
            fh.write(b)
        return b
    return None


def q(sql, args=()):
    with lock:
        cur = db.execute(sql, args)
        cols = [c[0] for c in cur.description]
        return [dict(zip(cols, r)) for r in cur.fetchall()]


def public_sighting(r):
    # never return the signing key of anonymous rows (there is none) or receive times
    out = {k: r[k] for k in ("window", "cell", "res", "latin", "conf_q8", "frames")}
    out["account"] = r["account"] if r["show_name"] else None   # verified either way; name only on opt-in
    out["rare"] = {3: "unconfirmed", 4: "confirmed"}.get(r["verified"])
    out["source"] = r.get("source") or "phone"
    if r.get("tier"):
        out["tier"] = r["tier"]
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
            if path.startswith("/api/accounts/"):
                return self._send(200, self._accounts_post(path, body))
            if path == "/api/devices/status":
                return self._send(200, devices.post_status(body, n))
            if path == "/api/flipper/frames":
                return self._send(200, devices.post_frames(body))
            if path == "/api/coverage/summaries":
                return self._send(200, devices.post_coverage(body))
            if path == "/api/devices/admin/seat-keys":
                if not devices.admin(self.headers):
                    return self._send(403, {"error": "forbidden"})
                return self._send(200, devices.admin_bind(body))
            if path == "/api/birds/sightings":
                return self._send(200, add_sighting(body))
            if path == "/api/birds/photos":
                return self._send(200, add_photo(body))
            if path == "/api/birds/windows":
                return self._send(200, add_window(body))
            if path == "/api/birds/labels":
                return self._send(200, add_labels(body))
            if path == "/api/birds/admin/posting-keys":
                # Refreshed by the n8n workflow "HONE: birds posting-key refresh" on grouchly.
                token = os.environ.get("HONE_BIRDS_ADMIN_TOKEN", "")
                if not token or not hmac.compare_digest(self.headers.get("X-Hone-Admin", ""), token):
                    return self._send(403, {"error": "forbidden"})
                keys = {str(a): str(k).lower() for a, k in body.items() if re.fullmatch(r"[0-9a-fA-F]{64}", str(k))}
                if len(keys) < 10:
                    raise ValueError("refusing a suspiciously small key table")
                tmp = KEYS_FILE + ".tmp"
                with open(tmp, "w") as f:
                    json.dump(keys, f)
                os.replace(tmp, KEYS_FILE)
                # Rows held back because the account was unknown become public once it is known.
                with lock:
                    for acct, pub in keys.items():
                        db.execute("UPDATE sightings SET verified=1 WHERE verified=0 AND account=? AND lower(pub)=?", (acct, pub))
                        db.execute("UPDATE photos SET verified=1 WHERE verified=0 AND account=? AND lower(pub)=?", (acct, pub))
                    db.commit()
                return self._send(200, {"ok": True, "accounts": len(keys)})
            return self._send(404, {"error": "not found"})
        except accounts.AuthError as e:
            return self._send(401, {"error": str(e)[:200]})
        except PermissionError as e:
            return self._send(429, {"error": str(e)[:200]})
        except Exception as e:
            # Rejects are logged (path + reason only, no IPs) so a silent phone can be told
            # apart from a refused one.
            sys.stderr.write("REJECT %s: %s\n" % (urlparse(self.path).path, str(e)[:160]))
            return self._send(400, {"error": str(e)[:200]})

    def _accounts_post(self, path, body):
        if path == "/api/accounts/signup":
            return accounts.signup(body)
        if path == "/api/accounts/login":
            return accounts.login(body)
        if path == "/api/accounts/admin/outbox/ack":
            if not devices.admin(self.headers):
                raise accounts.AuthError("forbidden")
            return accounts.ack(body.get("ids") or [])
        h = accounts.auth(self.headers)
        if path == "/api/accounts/verify":
            return accounts.verify(h, body)
        if path == "/api/accounts/resend":
            return accounts.resend(h)
        if path == "/api/accounts/logout":
            return accounts.logout(self.headers)
        raise ValueError("not found")

    def do_PUT(self):
        n = int(self.headers.get("Content-Length", 0))
        if n <= 0 or n > 512 * 1024:
            return self._send(413, {"error": "body size"})
        try:
            if urlparse(self.path).path != "/api/accounts/me":
                return self._send(404, {"error": "not found"})
            return self._send(200, accounts.put_me(accounts.auth(self.headers), json.loads(self.rfile.read(n))))
        except accounts.AuthError as e:
            return self._send(401, {"error": str(e)})
        except Exception as e:
            sys.stderr.write("REJECT %s: %s\n" % (urlparse(self.path).path, str(e)[:160]))
            return self._send(400, {"error": str(e)[:200]})

    def do_GET(self):
        u = urlparse(self.path)
        a = {k: v[0] for k, v in parse_qs(u.query).items()}
        path = u.path
        try:
            if path == "/api/birds/occupancy":
                return self._send(200, {"cell": a["cell"], "species": occupancy.area_view(a["cell"])})
            if path == "/api/birds/occupancy/map":
                res = int(a.get("res", 7))
                if res not in (5, 7):
                    raise ValueError("res must be 5 or 7")
                return self._send(200, occupancy.map_cells(res))
            if path == "/api/birds/occupancy/changes":
                return self._send(200, occupancy.changes(int(a.get("since", 0)), int(a.get("limit", 500))))
            if path == "/api/birds/sightings":
                rows = q("SELECT * FROM sightings WHERE verified IN (1,3,4) AND window>=? ORDER BY window DESC LIMIT ?",
                         (int(a.get("since", 0)), min(int(a.get("limit", 500)), 2000)))
                return self._send(200, [public_sighting(r) for r in rows])
            if path == "/api/birds/species":
                rows = q("""SELECT latin, COUNT(*) sightings, COUNT(DISTINCT cell) cells,
                            MAX(window) last_window, MIN(window) first_window,
                            SUM(verified IN (3,4)) rare
                            FROM sightings WHERE verified IN (1,3,4) GROUP BY latin ORDER BY sightings DESC""")
                photos = {r["latin"]: r["n"] for r in q("SELECT latin, COUNT(*) n FROM photos WHERE hidden=0 AND verified=1 GROUP BY latin")}
                for r in rows:
                    r["photos"] = photos.get(r["latin"], 0)
                return self._send(200, rows)
            m = re.match(r"^/api/birds/species/(.+)$", path)
            if m:
                latin = unquote(m.group(1))
                s = q("SELECT * FROM sightings WHERE verified IN (1,3,4) AND latin=? ORDER BY window DESC LIMIT 500", (latin,))
                p = q("SELECT id, t, license, evidence_ok, CASE WHEN show_name=1 THEN account END account FROM photos"
                      " WHERE latin=? AND hidden=0 AND verified=1 ORDER BY t DESC LIMIT 100", (latin,))
                for r in p:
                    r["url"] = f"/api/birds/photos/{r['id']}.jpg"
                return self._send(200, {"latin": latin, "sightings": [public_sighting(r) for r in s], "photos": p})
            if path == "/api/birds/map":
                res = int(a.get("res", 5))
                if res not in (3, 5, 7, 9):
                    raise ValueError("res must be 3, 5, 7 or 9")
                sql = "SELECT cell, res, latin FROM sightings WHERE verified IN (1,3,4) AND window>=?"
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
                hidden = q("SELECT hidden, verified FROM photos WHERE id=?", (m.group(1),))
                if not hidden or hidden[0]["hidden"] or not hidden[0]["verified"]:
                    return self._send(404, {"error": "not found"})
                b = load_photo(m.group(1))
                if b is None:
                    return self._send(404, {"error": "not found"})
                return self._send(200, raw=b, ctype="image/jpeg")
            m = re.match(r"^/api/birds/call/(.+)$", path)
            if m:
                latin = unquote(m.group(1))
                ip = self.headers.get("X-Forwarded-For", self.client_address[0]).split(",")[0]
                if limited("xc:" + ip, 60, 3600):
                    return self._send(429, {"error": "slow down"})
                try:
                    recs = xc_calls(latin)
                except LookupError as e:
                    return self._send(503, {"error": str(e)})
                return self._send(200, {"latin": latin, "source": "xeno-canto.org",
                                        "use": "reference listening only; played unmodified", "recordings": recs})
            if path == "/api/birds/range":
                c3, names = species_range(a["cell"])
                return self._send(200, {"cell": c3, "radius_km": RANGE_KM, "source": "GBIF.org occurrences",
                                        "species": names})
            if path == "/api/accounts/me":
                try:
                    return self._send(200, accounts.me(accounts.auth(self.headers), devices.online))
                except accounts.AuthError as e:
                    return self._send(401, {"error": str(e)})
            if path == "/api/accounts/admin/outbox":
                if not devices.admin(self.headers):
                    return self._send(403, {"error": "forbidden"})
                return self._send(200, accounts.outbox())
            m = re.match(r"^/api/accounts/ref/([a-z0-9._-]{1,32})$", path)
            if m:
                return self._send(200, {"ok": accounts.ref_ok(m.group(1)), "ref": m.group(1)})
            if path == "/api/devices/status" or path.startswith("/api/devices/status/"):
                out = devices.get_status(path, a, self.headers)
                return self._send(200, out) if out is not None else self._send(404, {"error": "not found"})
            m = re.match(r"^/api/devices/seals/([^/]+)$", path)
            if m:
                return self._send(200, devices.get_seals(unquote(m.group(1))))
            if path == "/api/devices/admin/unbound":
                if not devices.admin(self.headers):
                    return self._send(403, {"error": "forbidden"})
                return self._send(200, devices.unbound())
            if path == "/api/flipper/frames":
                return self._send(200, devices.get_frames(a))
            if path == "/api/coverage/map":
                return self._send(200, devices.get_coverage_map(a))
            if path == "/api/flipper/map":
                return self._send(200, devices.get_frames_map(a))
            if path == "/api/birds/health":
                return self._send(200, {"ok": True, "sightings": q("SELECT COUNT(*) n FROM sightings")[0]["n"],
                                        "photos": q("SELECT COUNT(*) n FROM photos")[0]["n"]})
            return self._send(404, {"error": "not found"})
        except Exception as e:
            return self._send(400, {"error": str(e)[:200]})


if __name__ == "__main__":
    ThreadingHTTPServer(("127.0.0.1", PORT), H).serve_forever()
