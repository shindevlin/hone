"""HONE device status + Flipper frame index (served by the birds indexer process).

Signed envelopes (all uploads): {"payload": "<canonical json string>", "key": "posting"|"seat",
"pub": hex, "sig": hex}. The signature is Ed25519 over the exact payload string.
  - key=posting: pub must be the account's on-chain posting key (posting_keys.json table).
  - key=seat: the device's seat key. Accepted, but "verified" only once the seat is bound to
    the account: by a posting-key-signed status that names the seat (seat_pub), or by an admin.

API:
  POST /api/devices/status                       signed status (latest + 24 h history kept)
  GET  /api/devices/status                       public: [{account, device, online}]
  GET  /api/devices/status/<account>             public: online/offline per device; full with auth
  GET  /api/devices/status/<account>/<device>    public: online; full (+ ?history=1) with auth
       auth = X-Hone-Admin: <HONE_DEVICES_ADMIN_TOKEN>, or the account owner:
              X-Hone-Account, X-Hone-Ts (unix s, +-300), X-Hone-Sig = ed25519(posting key,
              "GET <path> <ts>")
  POST /api/devices/admin/seat-keys              {"account","pub"} admin: bind a seat key
  GET  /api/devices/admin/unbound                admin: seat signers not yet bound
  POST /api/flipper/frames                       signed batch of Flipper-signed commits
  GET  /api/flipper/frames?since=<t_min>&limit=&owner=
  GET  /api/flipper/map?res=5|7
  POST /api/coverage/summaries                  signed coverage rows (map; any age)
  GET  /api/coverage/map?res=5|7|9&since=&metric=
  GET  /api/devices/seals/<node id>              sealed epochs today (from finalized sealed_by)
"""
import hmac, json, os, re, time, urllib.error, urllib.request

import h3
from nacl.signing import VerifyKey

ONLINE_S = 35 * 60
HISTORY_S = 7 * 86400  # 7 days of status history
MAX_STATUS = 32 * 1024
MAX_FRAMES = 500
NODE = os.environ.get("HONE_NODE_URL", "http://127.0.0.1:4242")
ACCT = re.compile(r"^[a-z0-9][a-z0-9._-]{0,31}$")
DEV = re.compile(r"^[0-9a-f]{16}$")
HEX64 = re.compile(r"^[0-9a-f]{64}$")
SENSOR = re.compile(r"^flipper-([0-9a-f]{16})-(subghz|wifi|tags|ir)$")
TYPES = {"continuous", "event"}

_db = _lock = _posting_key = None
_dev_rate = {}
_chain_cache = {}


def init(db, lock, posting_key):
    global _db, _lock, _posting_key
    _db, _lock, _posting_key = db, lock, posting_key
    db.executescript("""
    CREATE TABLE IF NOT EXISTS seat_keys(account TEXT, pub TEXT, source TEXT, added INTEGER,
      PRIMARY KEY(account, pub));
    CREATE TABLE IF NOT EXISTS device_status(account TEXT, device TEXT, received INTEGER, ts INTEGER,
      keytype TEXT, pub TEXT, verified INTEGER, payload TEXT);
    CREATE INDEX IF NOT EXISTS ds_dev ON device_status(account, device, received);
    CREATE INDEX IF NOT EXISTS ds_recv ON device_status(received);
    CREATE TABLE IF NOT EXISTS device_latest(account TEXT, device TEXT, received INTEGER, ts INTEGER,
      keytype TEXT, pub TEXT, verified INTEGER, payload TEXT, PRIMARY KEY(account, device));
    CREATE TABLE IF NOT EXISTS seal_counts(day TEXT, node TEXT, n INTEGER, PRIMARY KEY(day, node));
    CREATE TABLE IF NOT EXISTS seal_cursor(k TEXT PRIMARY KEY, epoch INTEGER);
    CREATE TABLE IF NOT EXISTS seat_seen(account TEXT, pub TEXT, device TEXT, last INTEGER,
      PRIMARY KEY(account, pub));
    CREATE TABLE IF NOT EXISTS flipper_frames(batch_hash TEXT PRIMARY KEY, sensor_id TEXT, owner TEXT,
      reading_count INTEGER, sensor_type TEXT, signature TEXT, device_pubkey TEXT, chain_ok INTEGER,
      first_t_min INTEGER, received INTEGER);
    CREATE INDEX IF NOT EXISTS ff_t ON flipper_frames(first_t_min);
    CREATE TABLE IF NOT EXISTS flipper_heard(batch_hash TEXT, gw_account TEXT, gw_device TEXT,
      gw_verified INTEGER, t_min INTEGER, cell TEXT, rssi INTEGER, received INTEGER,
      PRIMARY KEY(batch_hash, gw_device));
    """)
    db.commit()


# ── signatures ─────────────────────────────────────────────────────────────────────────
def bound(account, pub):
    with _lock:
        return _db.execute("SELECT 1 FROM seat_keys WHERE account=? AND pub=?", (account, pub)).fetchone() is not None


def bind(account, pub, source):
    """Bind a seat key to an account and publish the bird rows it signed while held back."""
    with _lock:
        _db.execute("INSERT OR IGNORE INTO seat_keys VALUES(?,?,?,?)", (account, pub, source, int(time.time())))
        n = _db.execute("UPDATE sightings SET verified=1 WHERE verified=0 AND account=? AND lower(pub)=?", (account, pub)).rowcount
        n += _db.execute("UPDATE photos SET verified=1 WHERE verified=0 AND account=? AND lower(pub)=?", (account, pub)).rowcount
        _db.execute("UPDATE device_latest SET verified=1 WHERE account=? AND pub=?", (account, pub))
        _db.commit()
    return n


def verify_env(body):
    """Returns (payload, payload_str, keytype, pub, verified). Raises ValueError when refused."""
    ps = body.get("payload")
    if not isinstance(ps, str):
        raise ValueError("payload must be the signed JSON string")
    payload = json.loads(ps)
    keytype = body.get("key", "posting")
    pub, sig, acct = str(body.get("pub", "")).lower(), str(body.get("sig", "")), payload.get("account")
    if not (acct and pub and sig) or not ACCT.match(str(acct)) or not HEX64.match(pub):
        raise ValueError("account, pub and signature required")
    try:
        VerifyKey(bytes.fromhex(pub)).verify(ps.encode(), bytes.fromhex(sig))
    except Exception:
        raise ValueError("bad signature")
    if keytype == "posting":
        known = _posting_key(acct)
        if known is not None and known.lower() != pub:
            raise ValueError("pub is not this account's posting key")
        verified = 1 if known else 0
    elif keytype == "seat":
        verified = 1 if bound(acct, pub) else 0
        with _lock:
            _db.execute("INSERT OR REPLACE INTO seat_seen VALUES(?,?,?,?)",
                        (acct, pub, str(payload.get("device", ""))[:16], int(time.time())))
            _db.commit()
    else:
        raise ValueError("key must be posting or seat")
    return payload, ps, keytype, pub, verified


def _limited(device, per_hour):
    now = time.time()
    w = _dev_rate.get(device)
    if not w or now - w[0] > 3600:
        _dev_rate[device] = [now, 1]; return False
    w[1] += 1
    return w[1] > per_hour


# ── device status ──────────────────────────────────────────────────────────────────────
def post_status(body, raw_len):
    if raw_len > MAX_STATUS:
        raise ValueError("status too large")
    p, ps, keytype, pub, verified = verify_env(body)
    if p.get("kind") != "device_status":
        raise ValueError("kind must be device_status")
    acct, dev = p["account"], str(p.get("device", ""))
    if not DEV.match(dev):
        raise ValueError("device must be 16 hex")
    seat = str(p.get("seat_pub", "")).lower()
    if keytype == "seat" and seat != pub:
        raise ValueError("seat-signed status must name its seat_pub")
    if not seat.startswith(dev):
        raise ValueError("device id must be the seat pubkey prefix")
    if _limited("st:" + acct + "/" + dev, 40):
        raise PermissionError("slow down")
    ts = int(p.get("ts", 0)); now = int(time.time())
    if abs(ts - now) > 6 * 3600:
        raise ValueError("ts too far from now")
    if keytype == "posting" and verified and HEX64.match(seat) and not bound(acct, seat):
        bind(acct, seat, "posting-attest")  # the account's posting key vouches for this seat
    cell = p.get("cell7")
    if cell is not None and (not h3.is_valid_cell(cell) or h3.get_resolution(cell) > 7):
        raise ValueError("cell7 must be an H3 cell of res <= 7")
    row = (acct, dev, now, ts, keytype, pub, verified, ps)
    with _lock:
        _db.execute("INSERT INTO device_status VALUES(?,?,?,?,?,?,?,?)", row)
        _db.execute("INSERT OR REPLACE INTO device_latest VALUES(?,?,?,?,?,?,?,?)", row)
        _db.execute("DELETE FROM device_status WHERE received < ?", (now - HISTORY_S,))
        _db.commit()
    return {"ok": True, "verified": bool(verified), "signer": keytype}


def _row(r, full):
    acct, dev, received, ts, keytype, pub, verified, payload = r
    out = {"account": acct, "device": dev, "online": int(time.time()) - received < ONLINE_S}
    if full:
        out.update({"received": received, "ts": ts, "signer": keytype, "pub": pub,
                    "verified": bool(verified), "status": json.loads(payload)})
    return out


def online(account, device):
    with _lock:
        r = _db.execute("SELECT received FROM device_latest WHERE account=? AND device=?", (account, device)).fetchone()
    return bool(r) and int(time.time()) - r[0] < ONLINE_S


def authorized(headers, account, path):
    tok = os.environ.get("HONE_DEVICES_ADMIN_TOKEN", "")
    if tok and hmac.compare_digest(headers.get("X-Hone-Admin", ""), tok):
        return True
    a, ts, sig = headers.get("X-Hone-Account", ""), headers.get("X-Hone-Ts", ""), headers.get("X-Hone-Sig", "")
    if not (account and a == account and ts.isdigit() and sig) or abs(int(ts) - time.time()) > 300:
        return False
    key = _posting_key(a)
    if not key:
        return False
    try:
        VerifyKey(bytes.fromhex(key)).verify(f"GET {path} {ts}".encode(), bytes.fromhex(sig))
        return True
    except Exception:
        return False


def admin(headers):
    tok = os.environ.get("HONE_DEVICES_ADMIN_TOKEN", "")
    return bool(tok) and hmac.compare_digest(headers.get("X-Hone-Admin", ""), tok)


def get_status(path, args, headers):
    parts = [x for x in path[len("/api/devices/status"):].split("/") if x]
    with _lock:
        if not parts:
            rows = _db.execute("SELECT * FROM device_latest ORDER BY account, device").fetchall()
            return [_row(r, False) for r in rows]
        acct = parts[0]
        full = authorized(headers, acct, path)
        if len(parts) == 1:
            rows = _db.execute("SELECT * FROM device_latest WHERE account=? ORDER BY device", (acct,)).fetchall()
            return {"account": acct, "devices": [_row(r, full) for r in rows]}
        dev = parts[1]
        r = _db.execute("SELECT * FROM device_latest WHERE account=? AND device=?", (acct, dev)).fetchone()
        if not r:
            return None
        out = _row(r, full)
        if full and args.get("history"):
            hist = _db.execute("SELECT received, payload FROM device_status WHERE account=? AND device=? AND received>=?"
                               " ORDER BY received", (acct, dev, int(time.time()) - HISTORY_S)).fetchall()
            out["history"] = [{"received": h[0], "status": json.loads(h[1])} for h in hist]
        return out


def unbound():
    with _lock:
        return [{"account": a, "pub": p, "device": d, "last": t} for a, p, d, t in _db.execute(
            "SELECT s.account, s.pub, s.device, s.last FROM seat_seen s LEFT JOIN seat_keys k"
            " ON k.account=s.account AND k.pub=s.pub WHERE k.pub IS NULL ORDER BY s.last DESC")]


def admin_bind(body):
    acct, pub = str(body.get("account", "")), str(body.get("pub", "")).lower()
    if not ACCT.match(acct) or not HEX64.match(pub):
        raise ValueError("account and 64-hex pub required")
    return {"ok": True, "promoted_bird_rows": bind(acct, pub, "admin")}


# ── Flipper frames ─────────────────────────────────────────────────────────────────────
def commit_message(sensor_id, owner, batch_hash, count, sensor_type):
    # Byte-exact SENSOR_DATA_COMMIT message the Flipper signs (same as the phone's relay check).
    return ('{"chain_id":"hone","type":"SENSOR_DATA_COMMIT","sensor_id":"%s","owner":"%s",'
            '"batch_hash":"%s","reading_count":%d,"sensor_type":"%s","signed_by":"%s"}'
            % (sensor_id, owner, batch_hash, count, sensor_type, owner))


def chain_device(dev_id):
    """(device_pubkey, owner) the chain enrolled for this Flipper, cached 1 h; None if unknown."""
    c = _chain_cache.get(dev_id)
    if c and time.time() - c[0] < 3600:
        return c[1]
    val = None
    try:
        with urllib.request.urlopen(f"{NODE}/api/sensor/flipper-{dev_id}-subghz", timeout=5) as r:
            j = json.loads(r.read())
            val = (str(j.get("device_pubkey", "")).lower(), str(j.get("owner", "")))
    except Exception:
        val = None
    _chain_cache[dev_id] = (time.time(), val)
    return val


def post_frames(body):
    p, ps, keytype, pub, verified = verify_env(body)
    if p.get("kind") != "flipper_frames":
        raise ValueError("kind must be flipper_frames")
    acct, gw = p["account"], str(p.get("device", ""))
    if not DEV.match(gw):
        raise ValueError("device must be 16 hex")
    if _limited("ff:" + gw, 12):
        raise PermissionError("slow down")
    frames = p.get("frames") or []
    if not isinstance(frames, list) or len(frames) > MAX_FRAMES:
        raise ValueError(f"frames: list of at most {MAX_FRAMES}")
    now = int(time.time()); now_min = now // 60
    stored = dup = bad = 0
    rows_f, rows_h = [], []
    for f in frames:
        try:
            sid, owner, bh = str(f["sensor_id"]), str(f["owner"]), str(f["batch_hash"]).lower()
            count, st = int(f["reading_count"]), str(f["sensor_type"])
            sig, dpk = str(f["signature"]).lower(), str(f["device_pubkey"]).lower()
            m = SENSOR.match(sid)
            if not (m and ACCT.match(owner) and HEX64.match(bh) and re.fullmatch(r"[0-9a-f]{128}", sig)
                    and HEX64.match(dpk) and dpk.startswith(m.group(1)) and st in TYPES and 0 <= count < 65536):
                raise ValueError
            VerifyKey(bytes.fromhex(dpk)).verify(commit_message(sid, owner, bh, count, st).encode(), bytes.fromhex(sig))
            t = int(f.get("t_min", now_min))
            if not (now_min - 30 * 1440 <= t <= now_min + 10):
                raise ValueError
            cell = f.get("cell")
            if cell is not None:
                if not h3.is_valid_cell(cell):
                    cell = None
                elif h3.get_resolution(cell) > 7:
                    cell = h3.cell_to_parent(cell, 7)
            rssi = f.get("rssi"); rssi = int(rssi) if isinstance(rssi, int) and -130 < rssi < 20 else None
        except Exception:
            bad += 1
            continue
        known = chain_device(m.group(1))
        chain_ok = None if known is None else int(known == (dpk, owner))
        rows_f.append((bh, sid, owner, count, st, sig, dpk, chain_ok, t, now))
        rows_h.append((bh, acct, gw, verified, t, cell, rssi, now))
    with _lock:
        for rf, rh in zip(rows_f, rows_h):
            if _db.execute("INSERT OR IGNORE INTO flipper_frames VALUES(?,?,?,?,?,?,?,?,?,?)", rf).rowcount:
                stored += 1
            else:
                dup += 1
                _db.execute("UPDATE flipper_frames SET first_t_min=MIN(first_t_min, ?) WHERE batch_hash=?", (rf[8], rf[0]))
            _db.execute("INSERT OR IGNORE INTO flipper_heard VALUES(?,?,?,?,?,?,?,?)", rh)
        _db.commit()
    return {"ok": True, "stored": stored, "duplicates": dup, "rejected": bad, "gateway_verified": bool(verified)}


def get_frames(args):
    since, lim = int(args.get("since", 0)), min(int(args.get("limit", 500)), 2000)
    sql = ("SELECT f.batch_hash, f.sensor_id, f.owner, f.reading_count, f.sensor_type, f.first_t_min, f.chain_ok,"
           " f.signature, f.device_pubkey, COUNT(h.gw_device), GROUP_CONCAT(DISTINCT h.cell)"
           " FROM flipper_frames f LEFT JOIN flipper_heard h ON h.batch_hash=f.batch_hash WHERE f.first_t_min>=?")
    a = [since]
    if args.get("owner"):
        sql += " AND f.owner=?"; a.append(args["owner"])
    sql += " GROUP BY f.batch_hash ORDER BY f.first_t_min DESC LIMIT ?"; a.append(lim)
    with _lock:
        rows = _db.execute(sql, a).fetchall()
    return [{"batch_hash": r[0], "sensor_id": r[1], "owner": r[2], "reading_count": r[3], "sensor_type": r[4],
             "t_min": r[5], "chain_ok": None if r[6] is None else bool(r[6]), "signature": r[7],
             "device_pubkey": r[8], "heard_by": r[9],
             "cells": sorted({h3.cell_to_parent(c, 7) if h3.get_resolution(c) > 7 else c for c in (r[10] or "").split(",") if c})}
            for r in rows]


def get_frames_map(args):
    res = int(args.get("res", 7))
    if res not in (3, 5, 7):
        raise ValueError("res must be 3, 5 or 7")
    since = int(args.get("since", 0))
    with _lock:
        rows = _db.execute("SELECT h.cell, COUNT(DISTINCT h.batch_hash) FROM flipper_heard h JOIN flipper_frames f"
                           " ON f.batch_hash=h.batch_hash WHERE h.cell IS NOT NULL AND f.first_t_min>=? GROUP BY h.cell",
                           (since,)).fetchall()
    agg = {}
    for cell, n in rows:
        c = h3.cell_to_parent(cell, res) if h3.get_resolution(cell) > res else cell
        agg[c] = agg.get(c, 0) + n
    return [{"cell": c, "frames": n} for c, n in agg.items()]


# ── sealed-epoch counts per seat (from finalized epochs' sealed_by on the local node) ──
# GET /api/devices/seals/<node id>  e.g. josh@3f5a20e4...  -> sealed epochs today (UTC).
# A background thread walks finalized epochs on the ferryvps node (127.0.0.1:4242) and counts
# each sealer per UTC day, so phones show a chain-derived number, never a local counter.
GENESIS_TS_MS = 1_783_191_600_000
EPOCH_MS = 30_000
NODE_ID = re.compile(r"^[a-z0-9][a-z0-9._-]{0,31}(@[0-9a-f]{64})?$")
_seal_state = {"cursor": None, "tip": None, "err": None}


def _node_json(path):
    with urllib.request.urlopen(NODE + path, timeout=5) as r:
        return json.loads(r.read())


def _day(ms):
    return time.strftime("%Y-%m-%d", time.gmtime(ms / 1000))


def _seals_loop():
    import threading  # noqa: F401  (thread started by start_seals)
    with _lock:
        _db.execute("CREATE TABLE IF NOT EXISTS seal_counts(day TEXT, node TEXT, n INTEGER, PRIMARY KEY(day, node))")
        _db.execute("CREATE TABLE IF NOT EXISTS seal_cursor(k TEXT PRIMARY KEY, epoch INTEGER)")
        r = _db.execute("SELECT epoch FROM seal_cursor WHERE k='done'").fetchone()
        _db.commit()
    now_ms = int(time.time() * 1000)
    day0 = (now_ms // 86_400_000) * 86_400_000
    first_today = (day0 - GENESIS_TS_MS) // EPOCH_MS
    done = r[0] if r else first_today - 1
    while True:
        try:
            tip = int(_node_json("/api/node/info")["epoch"])
            _seal_state["tip"] = tip
            hi = min(tip - 2, done + 400)       # finalized lag; catch up in chunks
            for e in range(done + 1, hi + 1):
                try:
                    j = _node_json(f"/api/epoch/{e}")
                except urllib.error.HTTPError:
                    j = {}
                if j.get("finalized"):
                    day = _day(int(j.get("finalized_at") or (GENESIS_TS_MS + e * EPOCH_MS)))
                    with _lock:
                        for s in set(j.get("sealed_by") or []):
                            _db.execute("INSERT INTO seal_counts VALUES(?,?,1) ON CONFLICT(day,node) DO UPDATE SET n=n+1",
                                        (day, str(s)[:128]))
                        _db.commit()
                done = e
                with _lock:
                    _db.execute("INSERT OR REPLACE INTO seal_cursor VALUES('done', ?)", (done,))
                    _db.execute("DELETE FROM seal_counts WHERE day < ?", (_day(time.time() * 1000 - 3 * 86_400_000),))
                    _db.commit()
            _seal_state.update(cursor=done, err=None)
        except Exception as ex:
            _seal_state["err"] = str(ex)[:120]
        time.sleep(5 if _seal_state["tip"] and done < _seal_state["tip"] - 10 else 30)


def start_seals():
    import threading
    threading.Thread(target=_seals_loop, name="seals", daemon=True).start()


def get_seals(node_id):
    if not NODE_ID.match(node_id):
        raise ValueError("node id: account or account@<64 hex>")
    today = _day(time.time() * 1000)
    with _lock:
        r = _db.execute("SELECT n FROM seal_counts WHERE day=? AND node=?", (today, node_id)).fetchone()
    return {"node": node_id, "day": today, "sealed_today": r[0] if r else 0,
            "counted_to_epoch": _seal_state["cursor"], "chain_tip": _seal_state["tip"],
            "source": "sealed_by of finalized epochs"}


# ── coverage map (phone public_summary rows, any age; the chain only gets <= 7-day rows) ──
# POST /api/coverage/summaries  signed {"kind":"coverage","account","device","items":[{cell,window,metrics}]}
# GET  /api/coverage/map?res=5|7|9[&since=<window>][&metric=<name>]
METRIC = re.compile(r"^[a-z0-9._:]{1,40}$")


def _coverage_table():
    with _lock:
        _db.execute("""CREATE TABLE IF NOT EXISTS coverage(cell TEXT, window INTEGER, account TEXT, device TEXT,
            verified INTEGER, metrics TEXT, received INTEGER, PRIMARY KEY(cell, window, device))""")
        _db.execute("CREATE INDEX IF NOT EXISTS cov_w ON coverage(window)")
        _db.commit()


def post_coverage(body):
    _coverage_table()
    p, ps, keytype, pub, verified = verify_env(body)
    if p.get("kind") != "coverage":
        raise ValueError("kind must be coverage")
    acct, dev = p["account"], str(p.get("device", ""))
    if not DEV.match(dev):
        raise ValueError("device must be 16 hex")
    if _limited("cov:" + dev, 12):
        raise PermissionError("slow down")
    items = p.get("items") or []
    if not isinstance(items, list) or len(items) > 500:
        raise ValueError("items: list of at most 500")
    now = int(time.time()); now_w = now // 900
    rows, bad = [], 0
    for it in items:
        try:
            cell, w, m = it["cell"], int(it["window"]), it["metrics"]
            if not h3.is_valid_cell(cell) or h3.get_resolution(cell) > 9 or not (now_w - 60 * 96 <= w <= now_w):
                raise ValueError
            if not isinstance(m, dict) or len(m) > 32 or not all(METRIC.match(k) and isinstance(v, int) for k, v in m.items()):
                raise ValueError
            rows.append((cell, w, acct, dev, verified, json.dumps(m, sort_keys=True), now))
        except Exception:
            bad += 1
    with _lock:
        _db.executemany("INSERT OR REPLACE INTO coverage VALUES(?,?,?,?,?,?,?)", rows)
        _db.commit()
    return {"ok": True, "stored": len(rows), "rejected": bad, "verified": bool(verified)}


public_res_of = lambda account: 7  # set by the indexer (res 9 once the account saved a home)


def get_coverage_map(args):
    _coverage_table()
    res = int(args.get("res", 7))
    if res not in (5, 7, 9):
        raise ValueError("res must be 5, 7 or 9")
    metric = args.get("metric")
    if metric and not METRIC.match(metric):
        raise ValueError("bad metric")
    with _lock:
        rows = _db.execute("SELECT cell, device, verified, metrics, account FROM coverage WHERE window>=?",
                           (int(args.get("since", 0)),)).fetchall()
    agg = {}
    for cell, dev, ver, mj, acct in rows:
        cell = h3.cell_to_parent(cell, public_res_of(acct)) if h3.get_resolution(cell) > public_res_of(acct) else cell
        c = h3.cell_to_parent(cell, res) if h3.get_resolution(cell) > res else cell
        e = agg.setdefault(c, {"cell": c, "reports": 0, "verified_reports": 0, "devices": set(), "metric_max": None})
        e["reports"] += 1; e["verified_reports"] += 1 if ver else 0; e["devices"].add(dev)
        if metric:
            v = json.loads(mj).get(metric)
            if isinstance(v, int) and (e["metric_max"] is None or v > e["metric_max"]):
                e["metric_max"] = v
    return [dict(e, devices=len(e["devices"])) for e in agg.values()]
