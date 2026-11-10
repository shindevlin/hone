#!/usr/bin/env python3
"""hone-raw ingest + reader (honemesh.net/api/raw/*). Change-based raw records, see hone_rawfmt.py
and the schema registry (GET /api/raw/schema).

Access model (Josh 2026-11-10): live data is readable by anyone; after HONE_RAW_LIVE_WINDOW_S an
archive blob needs a paid request. The live window is a network parameter and is OFF (0) for now,
so everything is public. Blobs carry analytic content in the clear; personal identifiers are
already tokens (the ingest refuses anything that looks like a raw MAC address).

POST /api/raw/upload     body = gzip JSONL blob; headers X-Hone-Meta (base64url JSON) and X-Hone-Sig
                         (hex Ed25519 over the meta bytes by the pubkey in meta.owner = account@<64 hex>).
                         meta: {v:1, owner, device, window, cell|null, sha256, size, ts, kind}
                         -> {ok, sha256, size, stored}
GET  /api/raw/health     {ok, registry_sha256, live_window_s, s3}
GET  /api/raw/schema     the schema registry (JSON)
GET  /api/raw/index      ?account=&device=&cell=&from=&to=&limit=   (windows or RFC3339)
GET  /api/raw/blob/<sha> the stored blob (application/gzip)
GET  /api/raw/export     ?device=&from=&to=  -> JSONL of every record, in time order
GET  /api/raw/state      ?device=&at=        -> state rebuilt from the last keyframe + events
GET  /api/raw/stats/<account>               per device per UTC day: blobs, bytes

Storage: always a local directory (HONE_RAW_DIR); also an S3 bucket (MEGA S4) once
HONE_RAW_S3_ENV names an env file with HONE_RAW_S3_{ENDPOINT,BUCKET,ACCESS_KEY,SECRET_KEY,REGION}.
A background pass copies local-only blobs to S3. Credentials are never logged.
"""
import base64, datetime as dt, gzip, hashlib, hmac, json, os, re, sqlite3, sys, threading, time
import urllib.parse, urllib.request, urllib.error
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from nacl.signing import VerifyKey
from nacl.exceptions import BadSignatureError

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import hone_rawfmt as F  # noqa: E402

LISTEN = os.environ.get("HONE_RAW_LISTEN", "127.0.0.1:9470")
DATA = os.environ.get("HONE_RAW_DIR", "/var/lib/hone-raw")
DB = os.path.join(DATA, "index.db")
S3_ENV = os.environ.get("HONE_RAW_S3_ENV", "")
PREFIX = os.environ.get("HONE_RAW_PREFIX", "hone-raw").strip("/")
REGISTRY = os.environ.get("HONE_RAW_REGISTRY", os.path.join(os.path.dirname(os.path.abspath(__file__)), "hone-raw-registry.json"))
MAX_BLOB = int(os.environ.get("HONE_RAW_MAX_BLOB", str(8 << 20)))
DAY_QUOTA = int(os.environ.get("HONE_RAW_DAY_QUOTA", str(256 << 20)))      # per account per UTC day
DISK_FLOOR = int(os.environ.get("HONE_RAW_DISK_FLOOR", str(2 << 30)))       # refuse uploads below this free space
LIVE_WINDOW_S = int(os.environ.get("HONE_RAW_LIVE_WINDOW_S", "0"))          # 0 = off: everything public
ACCOUNTS = set(filter(None, os.environ.get("HONE_RAW_ACCOUNTS", "josh,shindevlin,natoshisakamoto").split(",")))
OWNER = re.compile(r"^([a-z0-9][a-z0-9._-]{0,63})@([0-9a-f]{64})$")
CELL = re.compile(r"^8[0-9a-f]{14}$")
SHA = re.compile(r"^[0-9a-f]{64}$")
RAW_MAC = re.compile(rb"(?<![0-9a-fA-F:])[0-9a-fA-F]{2}(:[0-9a-fA-F]{2}){5}(?![0-9a-fA-F:])")
SCHEMAS = (F.SCHEMA_HDR, F.SCHEMA_KF, F.SCHEMA_EV, F.SCHEMA_OBS)

REG_BYTES = open(REGISTRY, "rb").read()
REG_SHA = hashlib.sha256(REG_BYTES).hexdigest()
S3 = None


def load_s3():
    if not S3_ENV or not os.path.exists(S3_ENV):
        return None
    env = {}
    for line in open(S3_ENV):
        line = line.strip()
        if "=" in line and not line.startswith("#"):
            k, v = line.split("=", 1)
            env[k.strip()] = v.strip().strip('"').strip("'")
    g = lambda k, d=None: env.get("HONE_RAW_S3_" + k, d)
    c = {"endpoint": re.sub(r"^https?://", "", g("ENDPOINT", "")).rstrip("/"), "bucket": g("BUCKET"),
         "region": g("REGION", "us-east-1"), "ak": g("ACCESS_KEY"), "sk": g("SECRET_KEY"),
         "ssl": not str(g("ENDPOINT", "")).startswith("http://"), "dns": g("LOOKUP", "path") == "dns"}
    return c if all(c[k] for k in ("endpoint", "bucket", "ak", "sk")) else None


def _h(key, msg):
    return hmac.new(key, msg.encode(), hashlib.sha256).digest()


def s3_request(method, key, body=b""):
    c = S3
    host = f"{c['bucket']}.{c['endpoint']}" if c["dns"] else c["endpoint"]
    path = urllib.parse.quote("/" + (key if c["dns"] else f"{c['bucket']}/{key}"), safe="/-_.~")
    now = dt.datetime.now(dt.timezone.utc)
    amz, day = now.strftime("%Y%m%dT%H%M%SZ"), now.strftime("%Y%m%d")
    ph = hashlib.sha256(body).hexdigest()
    hd = {"host": host, "x-amz-content-sha256": ph, "x-amz-date": amz}
    if method == "PUT":
        hd["content-type"] = "application/gzip"
    sh = ";".join(sorted(hd))
    canon = "\n".join([method, path, "", "".join(f"{k}:{hd[k]}\n" for k in sorted(hd)), sh, ph])
    scope = f"{day}/{c['region']}/s3/aws4_request"
    sts = "\n".join(["AWS4-HMAC-SHA256", amz, scope, hashlib.sha256(canon.encode()).hexdigest()])
    k = _h(_h(_h(_h(("AWS4" + c["sk"]).encode(), day), c["region"]), "s3"), "aws4_request")
    hd["authorization"] = (f"AWS4-HMAC-SHA256 Credential={c['ak']}/{scope}, SignedHeaders={sh}, "
                           f"Signature={hmac.new(k, sts.encode(), hashlib.sha256).hexdigest()}")
    req = urllib.request.Request(("https://" if c["ssl"] else "http://") + host + path,
                                 data=body if method == "PUT" else None, method=method,
                                 headers={a: b for a, b in hd.items() if a != "host"})
    try:
        with urllib.request.urlopen(req, timeout=120) as r:
            return r.status
    except urllib.error.HTTPError as e:
        return e.code
    except OSError:
        return 0


_lock = threading.Lock()


def db():
    os.makedirs(DATA, exist_ok=True)
    con = sqlite3.connect(DB, timeout=30)
    con.execute("""CREATE TABLE IF NOT EXISTS raw(sha256 TEXT PRIMARY KEY, owner TEXT, account TEXT, device TEXT,
        kind TEXT, win INTEGER, cell TEXT, area_src TEXT, size INTEGER, records INTEGER, ts INTEGER, day TEXT,
        path TEXT, s3 INTEGER DEFAULT 0, stored_at INTEGER)""")
    con.execute("CREATE INDEX IF NOT EXISTS raw_dev_win ON raw(device, win)")
    con.execute("CREATE INDEX IF NOT EXISTS raw_acct_day ON raw(account, day)")
    con.execute("CREATE INDEX IF NOT EXISTS raw_cell_win ON raw(cell, win)")
    return con


def verify_meta(meta_b, sig_hex):
    m = json.loads(meta_b)
    o = OWNER.match(str(m.get("owner", "")))
    if not o:
        raise ValueError("owner must be <account>@<64 hex pubkey>")
    if ACCOUNTS and o.group(1) not in ACCOUNTS:
        raise PermissionError("account not accepted here")
    try:
        VerifyKey(bytes.fromhex(o.group(2))).verify(meta_b, bytes.fromhex(sig_hex))
    except (BadSignatureError, ValueError):
        raise PermissionError("signature does not verify under the owner key")
    if m.get("v") != 1 or not SHA.match(str(m.get("sha256", ""))):
        raise ValueError("bad meta")
    if not isinstance(m.get("window"), int) or not isinstance(m.get("size"), int):
        raise ValueError("window and size must be integers")
    if m.get("cell") is not None and not CELL.match(str(m["cell"])):
        raise ValueError("bad cell")
    if abs(int(time.time()) - int(m.get("ts", 0))) > 30 * 86400:
        raise ValueError("ts too far from now")
    return m, o.group(1)


def check_blob(body, m):
    """Public content must be well-formed hone.raw and free of raw identifiers."""
    try:
        raw = gzip.decompress(body)
    except OSError:
        raise ValueError("blob must be gzip")
    if len(raw) > 64 << 20:
        raise ValueError("blob expands too far")
    if RAW_MAC.search(raw):
        raise ValueError("blob contains a raw MAC address; send tokens only")
    recs = F.from_jsonl(raw)
    if not recs or recs[0].get("s") != F.SCHEMA_HDR:
        raise ValueError("first record must be hone.raw.hdr@1")
    if recs[0].get("win") != m["window"]:
        raise ValueError("header window does not match meta")
    for r in recs:
        if r.get("s") not in SCHEMAS or "t" not in r:
            raise ValueError("unknown schema in blob")
        if r.get("s") == F.SCHEMA_EV and r.get("e") not in F.EVENTS:
            raise ValueError("unknown event")
    bad = F.check_prov(recs)
    if bad is not None:
        raise ValueError(f"record without provenance (dev, cls, sen, why, loc): {str(bad)[:120]}")
    return recs


def blob_path(account, day, sha):
    return os.path.join(DATA, "blobs", account, day, sha + ".jsonl.gz")


def s3_sync_loop():
    while True:
        time.sleep(60)
        if not S3:
            continue
        with _lock:
            rows = db().execute("SELECT sha256, account, day, path FROM raw WHERE s3=0 LIMIT 200").fetchall()
        for sha, acct, day, p in rows:
            try:
                st = s3_request("PUT", f"{PREFIX}/{acct}/{day}/{sha}.jsonl.gz", open(p, "rb").read())
            except OSError:
                continue
            if st in (200, 201, 204):
                with _lock:
                    con = db(); con.execute("UPDATE raw SET s3=1 WHERE sha256=?", (sha,)); con.commit()


def win_arg(v):
    if v is None or v == "":
        return None
    if re.fullmatch(r"\d+", v):
        n = int(v)
        return n if n < 10_000_000 else n // 900          # window, or unix seconds
    return int(F.parse_t(v)) // 900


class H(BaseHTTPRequestHandler):
    server_version = "hone-raw/2"

    def log_message(self, fmt, *a):
        sys.stderr.write("%s\n" % (fmt % a))       # no client addresses, headers or bodies

    def send(self, code, body, ctype="application/json", extra=None):
        if isinstance(body, (dict, list)):
            body = json.dumps(body).encode()
        self.send_response(code)
        self.send_header("content-type", ctype)
        self.send_header("content-length", str(len(body)))
        self.send_header("access-control-allow-origin", "*")
        for k, v in (extra or {}).items():
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(body)

    def archived(self, win):
        return LIVE_WINDOW_S > 0 and time.time() - (win + 1) * 900 > LIVE_WINDOW_S

    def rows(self, q, limit=2000):
        sql, args = "SELECT sha256, owner, account, device, kind, win, cell, area_src, size, records, path FROM raw WHERE 1=1", []
        for k, col in (("account", "account"), ("device", "device"), ("cell", "cell")):
            if q.get(k):
                sql += f" AND {col}=?"; args.append(q[k][0])
        f, t = win_arg(q.get("from", [None])[0]), win_arg(q.get("to", [None])[0])
        if f is not None:
            sql += " AND win>=?"; args.append(f)
        if t is not None:
            sql += " AND win<=?"; args.append(t)
        sql += " ORDER BY win, stored_at LIMIT ?"; args.append(min(int(q.get("limit", [limit])[0]), limit))
        with _lock:
            return db().execute(sql, args).fetchall()

    def do_GET(self):
        u = urllib.parse.urlparse(self.path); q = urllib.parse.parse_qs(u.query); p = u.path
        if p == "/api/raw/health":
            return self.send(200, {"ok": True, "registry_sha256": REG_SHA, "live_window_s": LIVE_WINDOW_S, "s3": bool(S3)})
        if p == "/api/raw/schema":
            return self.send(200, REG_BYTES, extra={"x-registry-sha256": REG_SHA})
        if p == "/api/raw/index":
            return self.send(200, [{"sha256": r[0], "owner": r[1], "account": r[2], "device": r[3], "kind": r[4], "window": r[5],
                                    "cell": r[6], "area_src": r[7], "size": r[8], "records": r[9]} for r in self.rows(q)])
        m = re.fullmatch(r"/api/raw/blob/([0-9a-f]{64})", p)
        if m:
            with _lock:
                r = db().execute("SELECT win, path FROM raw WHERE sha256=?", (m.group(1),)).fetchone()
            if not r:
                return self.send(404, {"error": "unknown blob"})
            if self.archived(r[0]):
                return self.send(402, {"error": "archive window: paid access required", "live_window_s": LIVE_WINDOW_S})
            return self.send(200, open(r[1], "rb").read(), "application/gzip")
        if p in ("/api/raw/export", "/api/raw/state"):
            if not q.get("device"):
                return self.send(400, {"error": "device required"})
            if p == "/api/raw/state":
                at = q.get("at", [None])[0]
                at_t = F.parse_t(at) if at and not at.isdigit() else (int(at) if at else time.time())
                q = dict(q); q["to"] = [str(int(at_t) // 900)]; q.pop("from", None)
            rows = self.rows(q, limit=5000)
            if any(self.archived(r[5]) for r in rows):
                return self.send(402, {"error": "range reaches the archive window: paid access required"})
            recs = []
            for r in rows:
                try:
                    recs += F.from_jsonl(gzip.decompress(open(r[10], "rb").read()))
                except OSError:
                    pass
            recs.sort(key=lambda r: r["t"])
            if p == "/api/raw/export":
                return self.send(200, F.to_jsonl(recs), "application/x-ndjson")
            return self.send(200, {"device": q["device"][0], "at": F.rfc3339(at_t), "state": F.rebuild(recs, until=at_t)})
        m = re.fullmatch(r"/api/raw/stats/([a-z0-9][a-z0-9._-]{0,63})", p)
        if m:
            with _lock:
                rows = db().execute("SELECT device, kind, day, COUNT(*), SUM(size), SUM(records) FROM raw WHERE account=? "
                                    "GROUP BY device, kind, day ORDER BY day DESC LIMIT 400", (m.group(1),)).fetchall()
            return self.send(200, {"account": m.group(1), "days": [
                {"device": d, "kind": k, "day": day, "blobs": n, "bytes": b, "records": rc} for d, k, day, n, b, rc in rows]})
        self.send(404, {"error": "not found"})

    def do_POST(self):
        if self.path != "/api/raw/upload":
            return self.send(404, {"error": "not found"})
        try:
            n = int(self.headers.get("content-length", "0"))
            if n <= 0 or n > MAX_BLOB:
                return self.send(413, {"error": f"body must be 1..{MAX_BLOB} bytes"})
            meta_b = base64.urlsafe_b64decode(self.headers.get("x-hone-meta", "") + "==")
            m, account = verify_meta(meta_b, self.headers.get("x-hone-sig", ""))
            body = self.rfile.read(n)
            sha = hashlib.sha256(body).hexdigest()
            if sha != m["sha256"] or len(body) != m["size"]:
                return self.send(400, {"error": "body does not match meta sha256/size"})
            day = dt.datetime.fromtimestamp(m["window"] * 900, dt.timezone.utc).strftime("%Y-%m-%d")
            with _lock:
                con = db()
                if con.execute("SELECT 1 FROM raw WHERE sha256=?", (sha,)).fetchone():
                    return self.send(200, {"ok": True, "sha256": sha, "size": len(body), "dup": True})
                used = con.execute("SELECT COALESCE(SUM(size),0) FROM raw WHERE account=? AND day=?", (account, day)).fetchone()[0]
            if used + len(body) > DAY_QUOTA:
                return self.send(429, {"error": "daily quota reached"})
            st = os.statvfs(DATA)
            if st.f_bavail * st.f_frsize < DISK_FLOOR:
                return self.send(507, {"error": "ingest storage full; retry later"})
            recs = check_blob(body, m)
            area = recs[0].get("area") or {}
            path = blob_path(account, day, sha)
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path + ".tmp", "wb") as f:
                f.write(body)
            os.replace(path + ".tmp", path)
            with _lock:
                con = db()
                con.execute("INSERT OR IGNORE INTO raw VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                            (sha, m["owner"], account, str(m.get("device", ""))[:64], str(m.get("kind", ""))[:16], m["window"],
                             m.get("cell"), str(area.get("src", ""))[:16], len(body), len(recs), int(m.get("ts", 0)), day,
                             path, 0, int(time.time())))
                con.commit()
            self.send(200, {"ok": True, "sha256": sha, "size": len(body), "records": len(recs), "stored": "local" + ("+s3" if S3 else "")})
        except PermissionError as e:
            self.send(403, {"error": str(e)})
        except (ValueError, KeyError, json.JSONDecodeError, base64.binascii.Error) as e:
            self.send(400, {"error": str(e)[:200]})


def main():
    global S3
    S3 = load_s3()
    db().close()
    threading.Thread(target=s3_sync_loop, daemon=True).start()
    host, port = LISTEN.rsplit(":", 1)
    sys.stderr.write(f"hone-raw on {LISTEN}; data {DATA}; s3 {'on' if S3 else 'off'}; live window {LIVE_WINDOW_S}s; registry {REG_SHA[:12]}\n")
    ThreadingHTTPServer((host, int(port)), H).serve_forever()


if __name__ == "__main__":
    main()
