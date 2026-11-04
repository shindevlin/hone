"""Indexer test for app accounts + station sightings (temp server, throwaway keys and emails)."""
import hashlib, json, os, re, subprocess, sys, tempfile, time, urllib.error, urllib.request

import h3
from nacl.signing import SigningKey

HERE = os.path.dirname(os.path.abspath(__file__))
PORT = 20460 + os.getpid() % 500
BASE = f"http://127.0.0.1:{PORT}"
ADMIN = "test-admin-token"


def canon(o):
    return json.dumps(o, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def call(method, path, body=None, headers=None):
    data = json.dumps(body).encode() if body is not None else None
    r = urllib.request.Request(BASE + path, data=data, method=method, headers={"Content-Type": "application/json", **(headers or {})})
    try:
        with urllib.request.urlopen(r, timeout=20) as x:
            return x.status, json.loads(x.read())
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read() or b"{}")


def seat_proof(sk, email, ts=None):
    pub = sk.verify_key.encode().hex()
    ps = canon({"kind": "seat_link", "seat_pub": pub, "email_sha": hashlib.sha256(email.encode()).hexdigest(),
                "label": "test phone", "ts": ts or int(time.time())})
    return {"payload": ps, "key": "seat", "pub": pub, "sig": sk.sign(ps.encode()).signature.hex()}


def main():
    data = tempfile.mkdtemp(prefix="hb-acct-")
    keys = {f"filler{i}": SigningKey.generate().verify_key.encode().hex() for i in range(11)}
    json.dump(keys, open(os.path.join(data, "posting_keys.json"), "w"))
    env = dict(os.environ, HONE_BIRDS_DATA=data, HONE_BIRDS_PORT=str(PORT), HONE_DEVICES_ADMIN_TOKEN=ADMIN,
               HONE_NODE_URL="http://127.0.0.1:1", HONE_SEALS="0")
    env.pop("HONE_MAIL_WEBHOOK", None)
    srv = subprocess.Popen([sys.executable, os.path.join(HERE, "hone_birds.py")], env=env, stderr=subprocess.PIPE)
    try:
        for _ in range(50):
            try: urllib.request.urlopen(BASE + "/api/birds/health", timeout=1); break
            except Exception: time.sleep(0.2)
        seat = SigningKey.generate(); email = "joe@example.org"; pw = "correct horse battery"
        base = {"name": "Joe", "email": email, "password": pw, "ref": "none", "consent": True, "consent_text": "pool"}
        # refusals
        c, r = call("POST", "/api/accounts/signup", dict(base, consent=False, seat=seat_proof(seat, email))); assert c == 400 and "tick" in r["error"], r
        c, r = call("POST", "/api/accounts/signup", dict(base, password="short", seat=seat_proof(seat, email))); assert c == 400, r
        c, r = call("POST", "/api/accounts/signup", dict(base, ref="nosuchuser", seat=seat_proof(seat, email))); assert "invite" in r["error"], r
        c, r = call("POST", "/api/accounts/signup", dict(base, seat=seat_proof(SigningKey.generate(), "other@x.org"))); assert c == 400, r
        # signup
        c, r = call("POST", "/api/accounts/signup", dict(base, seat=seat_proof(seat, email))); assert c == 200, r
        tok, handle = r["token"], r["user"]["handle"]
        assert re.fullmatch(r"u-[0-9a-f]{10}", handle) and r["user"]["verified"] is False and r["user"]["ref"] == "none"
        c, r = call("POST", "/api/accounts/signup", dict(base, seat=seat_proof(seat, email))); assert "already" in r["error"], r
        A = {"Authorization": "Bearer " + tok}
        # password stored as argon2id, never plain
        import sqlite3
        row = sqlite3.connect(os.path.join(data, "birds.db")).execute("SELECT pw FROM users WHERE handle=?", (handle,)).fetchone()
        assert row[0].startswith("$argon2id$") and pw not in row[0]
        # verification code through the outbox (what n8n reads)
        c, r = call("GET", "/api/accounts/admin/outbox"); assert c == 403
        c, ob = call("GET", "/api/accounts/admin/outbox", headers={"X-Hone-Admin": ADMIN}); assert len(ob) == 1 and ob[0]["to"] == email, ob
        code = re.search(r"\b(\d{6})\b", ob[0]["text"]).group(1)
        c, r = call("POST", "/api/accounts/verify", {"code": "000000" if code != "000000" else "111111"}, A); assert c == 400, r
        c, r = call("POST", "/api/accounts/verify", {"code": code}, A); assert c == 200 and r["verified"] is True, r
        c, r = call("POST", "/api/accounts/admin/outbox/ack", {"ids": [ob[0]["id"]]}, {"X-Hone-Admin": ADMIN}); assert r["acked"] == 1
        # login: wrong password, right password links a second phone
        c, r = call("POST", "/api/accounts/login", {"email": email, "password": "nope", "seat": seat_proof(seat, email)}); assert c == 400 and "Wrong" in r["error"]
        seat2 = SigningKey.generate()
        c, r = call("POST", "/api/accounts/login", {"email": "JOE@example.org ", "password": pw, "seat": seat_proof(seat2, email)}); assert c == 200, r
        c, r = call("GET", "/api/accounts/me", headers=A); assert c == 200 and len(r["devices"]) == 2, r
        c, r = call("GET", "/api/accounts/me"); assert c == 401
        c, r = call("PUT", "/api/accounts/me", {"life_list": [{"latin": "Turdus grayi", "first_ms": 1}, {"latin": "bad"}], "settings": {"digest": False}}, A)
        c, r = call("GET", "/api/accounts/me", headers=A); assert r["life_list"] == [{"latin": "Turdus grayi", "first_ms": 1}] and r["settings"]["digest"] is False
        # the linked device's seat-signed station sighting is public at once, tagged, and first in its area
        cell = h3.latlng_to_cell(13.68, -89.24, 7)
        p = canon({"v": 1, "sid": "st1", "window": int(time.time()) // 900 - 1, "cell": cell, "model": "t", "source": "station",
                   "tier": "C", "station": "0123456789abcdef", "species": [{"name": "Megascops cooperi", "conf_q8": 230, "frames": 3}],
                   "account": handle, "show_name": False})
        env_s = {"payload": p, "key": "seat", "pub": seat.verify_key.encode().hex(), "sig": seat.sign(p.encode()).signature.hex()}
        c, r = call("POST", "/api/birds/sightings", env_s); assert c == 200 and r["first_in_area"] == ["Megascops cooperi"], r
        c, r = call("GET", "/api/birds/sightings"); assert len(r) == 1 and r[0]["source"] == "station" and r[0]["tier"] == "C" and r[0]["account"] is None, r
        assert "station" not in r[0]
        c, r = call("GET", "/api/accounts/ref/none"); assert r["ok"] is True
        c, r = call("POST", "/api/accounts/logout", {}, A); c, r = call("GET", "/api/accounts/me", headers=A); assert c == 401
        print("ALL ACCOUNT TESTS PASSED")
    finally:
        srv.terminate()
        err = srv.stderr.read().decode()
        assert pw not in err and "joe@example.org" not in err.lower(), "secrets in server log"
        print("server log clean of password/email; rejects:", sum("REJECT" in l for l in err.splitlines()))


if __name__ == "__main__":
    main()
