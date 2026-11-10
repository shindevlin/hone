"""Client side of hone-raw: build a window blob, sign its meta, upload, keep a queue on disk."""
import base64, glob, gzip, hashlib, json, os, time, urllib.error, urllib.request
import hone_rawfmt as F

URL = "https://honemesh.net/api/raw/upload"


def blob(records):
    """gzip JSONL with a fixed mtime so the same records always give the same sha256."""
    return gzip.compress(F.to_jsonl(records), compresslevel=9, mtime=0)


def meta(sk, account, device, window, cell, body, kind):
    m = {"v": 1, "owner": f"{account}@{sk.verify_key.encode().hex()}", "device": device, "window": window,
         "cell": cell, "sha256": hashlib.sha256(body).hexdigest(), "size": len(body), "ts": int(time.time()), "kind": kind}
    mb = json.dumps(m, separators=(",", ":"), sort_keys=True).encode()
    return mb, sk.sign(mb).signature.hex()


def upload(url, meta_b, sig, body, timeout=60):
    req = urllib.request.Request(url, data=body, method="POST", headers={
        "content-type": "application/gzip", "x-hone-sig": sig,
        "x-hone-meta": base64.urlsafe_b64encode(meta_b).decode().rstrip("=")})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.status, json.loads(r.read() or b"{}")
    except urllib.error.HTTPError as e:
        try:
            return e.code, json.loads(e.read() or b"{}")
        except ValueError:
            return e.code, {}
    except OSError as e:
        return 0, {"error": str(e)[:120]}


def enqueue(qdir, window, body, info):
    """Queue a blob (never dropped: it stays until the ingest has it). Returns its sha256."""
    os.makedirs(qdir, exist_ok=True)
    sha = hashlib.sha256(body).hexdigest()
    base = os.path.join(qdir, f"{window}-{sha[:16]}")
    with open(base + ".gz.tmp", "wb") as f:
        f.write(body)
    os.replace(base + ".gz.tmp", base + ".gz")
    with open(base + ".json", "w") as f:
        json.dump({**info, "window": window, "sha256": sha}, f)
    return sha


def keep(path, keep_dir, keep_free_bytes):
    """Owner self-storage: after upload the owner's own blob stays here (served on request) while the
    disk keeps keep_free_bytes free; the oldest kept blobs go first when space is short."""
    os.makedirs(keep_dir, exist_ok=True)
    os.replace(path, os.path.join(keep_dir, os.path.basename(path)))
    st = os.statvfs(keep_dir)
    if st.f_bavail * st.f_frsize >= keep_free_bytes:
        return
    for old in sorted(glob.glob(os.path.join(keep_dir, "*.gz")), key=os.path.getmtime):
        os.remove(old)
        st = os.statvfs(keep_dir)
        if st.f_bavail * st.f_frsize >= keep_free_bytes:
            break


def flush(qdir, sk, account, device, kind, url=URL, max_n=50, keep_dir=None, keep_free_bytes=2 << 30):
    """Upload queued blobs oldest first; stop at the first network/server error. Returns (sent, left, last_error).
    With keep_dir, uploaded blobs are kept there (owner self-storage) instead of deleted."""
    files = sorted(glob.glob(os.path.join(qdir, "*.json")))
    sent, err = 0, None
    for jf in files[:max_n]:
        info = json.load(open(jf)); gz = jf[:-5] + ".gz"
        body = open(gz, "rb").read()
        mb, sig = meta(sk, account, device, info["window"], info.get("cell"), body, kind)
        st, r = upload(url, mb, sig, body)
        if st == 200 and r.get("ok"):
            if keep_dir:
                keep(gz, keep_dir, keep_free_bytes)
            else:
                os.remove(gz)
            os.remove(jf); sent += 1
        elif st in (400, 403, 413):
            # The ingest refuses this blob for good: keep it aside (never deleted) and go on.
            os.makedirs(os.path.join(qdir, "refused"), exist_ok=True)
            for f in (gz, jf):
                os.replace(f, os.path.join(qdir, "refused", os.path.basename(f)))
            err = f"refused {st}: {r.get('error')}"
        else:
            err = f"HTTP {st}: {r.get('error')}"
            break
    return sent, len(files) - sent, err
