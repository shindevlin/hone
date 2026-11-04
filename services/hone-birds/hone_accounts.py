"""HONE app accounts (email + password), served by the birds indexer process.

App profile only: name, life list, settings, linked devices. Nothing here goes on chain; the
email never leaves this database. Passwords: argon2id. Email verification by a 6-digit code,
sent from an outbox an n8n workflow drains (or a webhook, when HONE_MAIL_WEBHOOK is set).
Unverified accounts work; sign-in on another phone is the same either way.

API (JSON):
  POST /api/accounts/signup   {name,email,password,ref,consent:true,consent_text,seat:<seat envelope>}
  POST /api/accounts/login    {email,password,seat}
  POST /api/accounts/verify   {code}                      (Bearer)
  POST /api/accounts/resend   {}                          (Bearer)
  GET  /api/accounts/me                                   (Bearer) profile + devices
  PUT  /api/accounts/me       {name?,life_list?,settings?} (Bearer)
  POST /api/accounts/logout                               (Bearer)
  GET  /api/accounts/admin/outbox        (X-Hone-Admin)   unsent mail, for n8n
  POST /api/accounts/admin/outbox/ack    {ids:[...]}      (X-Hone-Admin)
  GET  /api/accounts/ref/<code>          is this invite code valid? {ok, ref}
The seat envelope: {"payload": canonical {"kind":"seat_link","seat_pub","email_sha","label","ts"},
"key":"seat","pub","sig"} signed by the phone's device key; it links that device to the account.
"""
import hashlib, hmac, json, os, re, secrets, threading, time, urllib.request

from argon2 import PasswordHasher
from argon2.exceptions import VerifyMismatchError, InvalidHashError
from nacl.signing import VerifyKey

ph = PasswordHasher()   # argon2id, library defaults (t=3, m=64 MiB, p=4)
EMAIL = re.compile(r"^[^@\s]{1,64}@[^@\s]{1,189}\.[^@\s]{2,}$")
REF = re.compile(r"^[a-z0-9][a-z0-9._-]{0,31}$")
LATIN = re.compile(r"^[A-Z][a-z]+ [a-z][a-z-]+$")
NODE = os.environ.get("HONE_NODE_URL", "http://127.0.0.1:4242")
SESSION_S = 180 * 86400
CODE_S = 30 * 60
OUTBOX_KEEP_S = 3600

_db = _lock = _bind = None
_ref_cache = {}
_login_rate = {}


class AuthError(Exception):
    """Missing or expired session (401)."""


class Refused(ValueError):
    """A message safe to show the user (never contains the email or password)."""


def init(db, lock, bind):
    global _db, _lock, _bind
    _db, _lock, _bind = db, lock, bind
    db.executescript("""
    CREATE TABLE IF NOT EXISTS users(handle TEXT PRIMARY KEY, email TEXT UNIQUE, pw TEXT, name TEXT, ref TEXT,
      verified INTEGER DEFAULT 0, consent_text TEXT, consent_ms INTEGER, created INTEGER,
      life_list TEXT DEFAULT '[]', settings TEXT DEFAULT '{}');
    CREATE TABLE IF NOT EXISTS user_codes(handle TEXT PRIMARY KEY, code_hash TEXT, expires INTEGER, tries INTEGER DEFAULT 0);
    CREATE TABLE IF NOT EXISTS user_sessions(token_hash TEXT PRIMARY KEY, handle TEXT, created INTEGER, expires INTEGER);
    CREATE TABLE IF NOT EXISTS user_seats(handle TEXT, seat_pub TEXT, label TEXT, linked INTEGER, PRIMARY KEY(handle, seat_pub));
    CREATE TABLE IF NOT EXISTS mail_outbox(id INTEGER PRIMARY KEY AUTOINCREMENT, to_email TEXT, subject TEXT, body TEXT,
      kind TEXT, created INTEGER, sent INTEGER DEFAULT 0);
    """)
    db.commit()


def _h(s):
    return hashlib.sha256(s.encode()).hexdigest()


def _norm_email(e):
    e = str(e or "").strip().lower()
    if len(e) > 254 or not EMAIL.match(e):
        raise Refused("Please enter a valid email address.")
    return e


def ref_ok(ref):
    """'none' (community pool) or an account that exists on chain (cached 1 h)."""
    if ref == "none":
        return True
    if not REF.match(ref):
        return False
    c = _ref_cache.get(ref)
    if c and time.time() - c[0] < 3600:
        return c[1]
    try:
        with urllib.request.urlopen(f"{NODE}/api/account/{ref}", timeout=5) as r:
            ok = r.status == 200 and json.loads(r.read()).get("account") == ref
    except Exception:
        ok = False
    _ref_cache[ref] = (time.time(), ok)
    return ok


def _seat(env, email):
    """Verify the device-key proof; returns (seat_pub, label)."""
    if not isinstance(env, dict) or env.get("key") != "seat":
        raise Refused("This app version can't link the phone. Please update HONE.")
    ps, pub, sig = env.get("payload"), str(env.get("pub", "")).lower(), str(env.get("sig", ""))
    try:
        VerifyKey(bytes.fromhex(pub)).verify(ps.encode(), bytes.fromhex(sig))
        p = json.loads(ps)
    except Exception:
        raise Refused("Couldn't link this phone. Please try again.")
    if p.get("kind") != "seat_link" or p.get("seat_pub") != pub or p.get("email_sha") != _h(email) \
            or abs(int(p.get("ts", 0)) - time.time()) > 900:
        raise Refused("Couldn't link this phone. Check the phone's date and time, then try again.")
    return pub, str(p.get("label", ""))[:40]


def _link(handle, pub, label):
    with _lock:
        _db.execute("INSERT OR REPLACE INTO user_seats VALUES(?,?,?,?)", (handle, pub, label, int(time.time())))
        _db.commit()
    _bind(handle, pub, "app-account")   # this device's uploads now count as this account's


def _session(handle):
    tok = secrets.token_urlsafe(32)
    now = int(time.time())
    with _lock:
        _db.execute("INSERT INTO user_sessions VALUES(?,?,?,?)", (_h(tok), handle, now, now + SESSION_S))
        _db.execute("DELETE FROM user_sessions WHERE expires < ?", (now,))
        _db.commit()
    return tok


def _user(handle):
    with _lock:
        r = _db.execute("SELECT handle,email,name,ref,verified,life_list,settings,created FROM users WHERE handle=?", (handle,)).fetchone()
    if not r:
        return None
    return {"handle": r[0], "email": r[1], "name": r[2], "ref": r[3], "verified": bool(r[4]),
            "life_list": json.loads(r[5] or "[]"), "settings": json.loads(r[6] or "{}"), "created": r[7]}


def auth(headers):
    a = headers.get("Authorization", "")
    if not a.startswith("Bearer "):
        raise AuthError("Please sign in again.")
    with _lock:
        r = _db.execute("SELECT handle FROM user_sessions WHERE token_hash=? AND expires>?", (_h(a[7:].strip()), int(time.time()))).fetchone()
    if not r:
        raise AuthError("Please sign in again.")
    return r[0]


def _queue_code(handle, email, name):
    code = f"{secrets.randbelow(1_000_000):06d}"
    now = int(time.time())
    body = (f"Hi {name or 'there'},\n\nYour HONE code is {code}\n\nType it in the app to confirm your email. "
            "It works for 30 minutes.\nIf you didn't sign up for HONE, you can ignore this email.\n\nHONE · https://honemesh.net")
    with _lock:
        _db.execute("INSERT OR REPLACE INTO user_codes VALUES(?,?,?,0)", (handle, _h(handle + code), now + CODE_S))
        _db.execute("DELETE FROM mail_outbox WHERE to_email=? AND kind='verify' AND sent=0", (email,))
        _db.execute("INSERT INTO mail_outbox(to_email,subject,body,kind,created) VALUES(?,?,?,?,?)",
                    (email, f"Your HONE code: {code}", body, "verify", now))
        _db.commit()
    hook = os.environ.get("HONE_MAIL_WEBHOOK", "")
    if hook:
        threading.Thread(target=_push_outbox, args=(hook,), daemon=True).start()


def _push_outbox(hook):
    for m in outbox():
        try:
            req = urllib.request.Request(hook, data=json.dumps({"to": m["to"], "subject": m["subject"], "text": m["text"]}).encode(),
                                         headers={"Content-Type": "application/json",
                                                  "X-Hone-Mail": os.environ.get("HONE_MAIL_WEBHOOK_TOKEN", "")})
            with urllib.request.urlopen(req, timeout=10) as r:
                if r.status < 300:
                    ack([m["id"]])
        except Exception:
            pass


def outbox():
    now = int(time.time())
    with _lock:
        _db.execute("DELETE FROM mail_outbox WHERE sent=1 OR created < ?", (now - OUTBOX_KEEP_S,))
        _db.commit()
        rows = _db.execute("SELECT id,to_email,subject,body FROM mail_outbox WHERE sent=0 ORDER BY id LIMIT 50").fetchall()
    return [{"id": r[0], "to": r[1], "subject": r[2], "text": r[3]} for r in rows]


def ack(ids):
    ids = [int(i) for i in ids][:100]
    with _lock:
        _db.executemany("DELETE FROM mail_outbox WHERE id=?", [(i,) for i in ids])
        _db.commit()
    return {"ok": True, "acked": len(ids)}


def signup(b):
    email = _norm_email(b.get("email"))
    pw = str(b.get("password", ""))
    if len(pw) < 10 or len(pw) > 200:
        raise Refused("Please use a password of at least 10 characters.")
    if b.get("consent") is not True:
        raise Refused("Please tick the box to continue.")
    ref = str(b.get("ref") or "none").strip().lower()
    if not ref_ok(ref):
        raise Refused("That invite code wasn't found. Check it, or leave it empty.")
    name = re.sub(r"[\x00-\x1f<>]", "", str(b.get("name", "")))[:40].strip()
    pub, label = _seat(b.get("seat"), email)
    with _lock:
        if _db.execute("SELECT 1 FROM users WHERE email=?", (email,)).fetchone():
            raise Refused("That email already has an account. Tap \"I already have an account\" to sign in.")
        handle = "u-" + secrets.token_hex(5)
        _db.execute("INSERT INTO users(handle,email,pw,name,ref,verified,consent_text,consent_ms,created) VALUES(?,?,?,?,?,0,?,?,?)",
                    (handle, email, ph.hash(pw), name, ref, str(b.get("consent_text", ""))[:300], int(time.time() * 1000), int(time.time())))
        _db.commit()
    _link(handle, pub, label)
    _queue_code(handle, email, name)
    return {"token": _session(handle), "user": _public(_user(handle))}


def login(b):
    email = _norm_email(b.get("email"))
    w = _login_rate.get(email)
    now = time.time()
    if w and now - w[0] < 3600 and w[1] >= 10:
        raise Refused("Too many tries. Please wait an hour and try again.")
    with _lock:
        r = _db.execute("SELECT handle, pw FROM users WHERE email=?", (email,)).fetchone()
    ok = False
    if r:
        try:
            ok = ph.verify(r[1], str(b.get("password", "")))
        except (VerifyMismatchError, InvalidHashError):
            ok = False
    if not ok:
        _login_rate[email] = [w[0], w[1] + 1] if w and now - w[0] < 3600 else [now, 1]
        raise Refused("Wrong email or password.")
    _login_rate.pop(email, None)
    if ph.check_needs_rehash(r[1]):
        with _lock:
            _db.execute("UPDATE users SET pw=? WHERE handle=?", (ph.hash(str(b.get("password", ""))), r[0])); _db.commit()
    pub, label = _seat(b.get("seat"), email)
    _link(r[0], pub, label)
    return {"token": _session(r[0]), "user": _public(_user(r[0]))}


def verify(handle, b):
    code = re.sub(r"\D", "", str(b.get("code", "")))
    with _lock:
        r = _db.execute("SELECT code_hash, expires, tries FROM user_codes WHERE handle=?", (handle,)).fetchone()
        if not r or r[1] < time.time() or r[2] >= 5:
            raise Refused("That code has expired. Tap \"Send a new code\".")
        if not hmac.compare_digest(r[0], _h(handle + code)):
            _db.execute("UPDATE user_codes SET tries=tries+1 WHERE handle=?", (handle,)); _db.commit()
            raise Refused("That code is wrong. Check the email and try again.")
        _db.execute("UPDATE users SET verified=1 WHERE handle=?", (handle,))
        _db.execute("DELETE FROM user_codes WHERE handle=?", (handle,))
        _db.commit()
    return _public(_user(handle))


def resend(handle):
    u = _user(handle)
    if u["verified"]:
        return {"ok": True, "verified": True}
    with _lock:
        r = _db.execute("SELECT expires FROM user_codes WHERE handle=?", (handle,)).fetchone()
    if r and r[0] - CODE_S > time.time() - 60:
        raise Refused("A code was just sent. Please wait a minute.")
    _queue_code(handle, u["email"], u["name"])
    return {"ok": True}


def _public(u):
    return {k: u[k] for k in ("handle", "email", "name", "ref", "verified")}


def me(handle, device_latest):
    u = _user(handle)
    with _lock:
        seats = _db.execute("SELECT seat_pub, label, linked FROM user_seats WHERE handle=? ORDER BY linked", (handle,)).fetchall()
    devs = []
    for pub, label, linked in seats:
        online = device_latest(handle, pub[:16])
        devs.append({"device": pub[:16], "label": label, "linked": linked, "online": online})
    out = _public(u)
    out.update({"life_list": u["life_list"], "settings": u["settings"], "devices": devs})
    return out


def put_me(handle, b):
    sets, args = [], []
    if "name" in b:
        sets.append("name=?"); args.append(re.sub(r"[\x00-\x1f<>]", "", str(b["name"]))[:40].strip())
    if "life_list" in b:
        ll = [{"latin": x["latin"], "first_ms": int(x.get("first_ms", 0))} for x in (b["life_list"] or [])[:5000]
              if isinstance(x, dict) and LATIN.match(str(x.get("latin", "")))]
        sets.append("life_list=?"); args.append(json.dumps(ll))
    if "settings" in b:
        s = json.dumps(b["settings"] if isinstance(b["settings"], dict) else {})
        if len(s) > 4096:
            raise Refused("Settings too large.")
        sets.append("settings=?"); args.append(s)
    if sets:
        with _lock:
            _db.execute(f"UPDATE users SET {','.join(sets)} WHERE handle=?", (*args, handle)); _db.commit()
    return {"ok": True}


def logout(headers):
    a = headers.get("Authorization", "")
    with _lock:
        _db.execute("DELETE FROM user_sessions WHERE token_hash=?", (_h(a[7:].strip()),)); _db.commit()
    return {"ok": True}
