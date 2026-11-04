"""Indexer test for device status + Flipper frames, with test accounts and throwaway keys.
Run: python3 test_hone_devices.py  (starts hone_birds.py on a temp port with a temp data dir)."""
import json, os, subprocess, sys, tempfile, time, urllib.error, urllib.request

import h3
from nacl.signing import SigningKey

HERE = os.path.dirname(os.path.abspath(__file__))
PORT = 19460 + os.getpid() % 500
BASE = f"http://127.0.0.1:{PORT}"
ADMIN = "test-admin-token"


def canon(o):
    return json.dumps(o, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def env(payload, sk, key):
    ps = canon(payload)
    return {"payload": ps, "key": key, "pub": sk.verify_key.encode().hex(), "sig": sk.sign(ps.encode()).signature.hex()}


def call(method, path, body=None, headers=None):
    data = json.dumps(body).encode() if body is not None else None
    r = urllib.request.Request(BASE + path, data=data, method=method, headers={"Content-Type": "application/json", **(headers or {})})
    try:
        with urllib.request.urlopen(r, timeout=10) as x:
            return x.status, json.loads(x.read())
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read() or b"{}")


def main():
    data = tempfile.mkdtemp(prefix="hb-test-")
    posting = SigningKey.generate(); seat = SigningKey.generate(); flipper = SigningKey.generate()
    other = SigningKey.generate()
    keys = {"testacct": posting.verify_key.encode().hex()}
    keys.update({f"filler{i}": SigningKey.generate().verify_key.encode().hex() for i in range(10)})
    json.dump(keys, open(os.path.join(data, "posting_keys.json"), "w"))
    e = dict(os.environ, HONE_BIRDS_DATA=data, HONE_BIRDS_PORT=str(PORT), HONE_DEVICES_ADMIN_TOKEN=ADMIN,
             HONE_NODE_URL="http://127.0.0.1:1")
    srv = subprocess.Popen([sys.executable, os.path.join(HERE, "hone_birds.py")], env=e, stderr=subprocess.PIPE)
    try:
        for _ in range(50):
            try:
                urllib.request.urlopen(BASE + "/api/birds/health", timeout=1); break
            except Exception:
                time.sleep(0.2)
        seat_pub = seat.verify_key.encode().hex(); dev = seat_pub[:16]
        now = int(time.time())
        cell7 = h3.latlng_to_cell(13.68, -89.24, 7)
        st = {"v": 1, "kind": "device_status", "account": "testacct", "device": dev, "seat_pub": seat_pub,
              "ts": now, "cell7": cell7, "battery": {"pct": 80}}

        # 1. seat-signed status: accepted, unverified (seat not bound yet)
        c, r = call("POST", "/api/devices/status", env(st, seat, "seat")); assert c == 200 and r["verified"] is False, (c, r)
        # 2. seat-signed bird sighting is held back (not public)
        s = {"v": 1, "sid": "t1", "window": now // 900 - 1, "cell": h3.latlng_to_cell(13.68, -89.24, 9), "model": "t",
             "species": [{"name": "Turdus grayi", "conf_q8": 230, "frames": 2}], "account": "testacct", "show_name": False}
        c, r = call("POST", "/api/birds/sightings", env(s, seat, "seat")); assert c == 200, (c, r)
        c, r = call("GET", "/api/birds/sightings"); assert c == 200 and r == [], r
        # 3. wrong posting key refused; posting key vouches for the seat -> bound, held rows promoted
        c, r = call("POST", "/api/devices/status", env(st, other, "posting")); assert c == 400, (c, r)
        c, r = call("POST", "/api/devices/status", env(dict(st, ts=now + 1), posting, "posting")); assert c == 200 and r["verified"], r
        c, r = call("GET", "/api/birds/sightings"); assert c == 200 and len(r) == 1, r
        c, r = call("POST", "/api/devices/status", env(dict(st, ts=now + 2), seat, "seat")); assert r["verified"] is True, r
        # 4. finer than res 7 refused; seat status must name its own seat
        c, r = call("POST", "/api/devices/status", env(dict(st, cell7=h3.latlng_to_cell(13.68, -89.24, 9)), seat, "seat")); assert c == 400, r
        c, r = call("POST", "/api/devices/status", env(dict(st, seat_pub=other.verify_key.encode().hex()), seat, "seat")); assert c == 400, r
        # 5. public view is online/offline only; admin and owner-signed get full detail + history
        c, r = call("GET", "/api/devices/status/testacct"); assert r["devices"][0] == {"account": "testacct", "device": dev, "online": True}, r
        c, r = call("GET", f"/api/devices/status/testacct/{dev}?history=1", headers={"X-Hone-Admin": ADMIN})
        assert r["status"]["battery"]["pct"] == 80 and len(r["history"]) == 3, r
        path = f"/api/devices/status/testacct/{dev}"; ts = str(int(time.time()))
        sig = posting.sign(f"GET {path} {ts}".encode()).signature.hex()
        c, r = call("GET", path, headers={"X-Hone-Account": "testacct", "X-Hone-Ts": ts, "X-Hone-Sig": sig}); assert "status" in r, r
        c, r = call("GET", path, headers={"X-Hone-Admin": "wrong"}); assert "status" not in r, r
        c, r = call("GET", "/api/devices/status"); assert c == 200 and len(r) == 1, r
        # 6. admin bind + unbound listing
        seat2 = SigningKey.generate(); s2 = seat2.verify_key.encode().hex()
        call("POST", "/api/devices/status", env(dict(st, device=s2[:16], seat_pub=s2), seat2, "seat"))
        c, r = call("GET", "/api/devices/admin/unbound", headers={"X-Hone-Admin": ADMIN}); assert [u["pub"] for u in r] == [s2], r
        c, r = call("POST", "/api/devices/admin/seat-keys", {"account": "testacct", "pub": s2}); assert c == 403
        c, r = call("POST", "/api/devices/admin/seat-keys", {"account": "testacct", "pub": s2}, {"X-Hone-Admin": ADMIN}); assert c == 200, r
        # 7. Flipper frames: Flipper-signed commit verified; forged one rejected; duplicates merged
        fpub = flipper.verify_key.encode().hex(); sid = f"flipper-{fpub[:16]}-subghz"; bh = "ab" * 32
        msg = ('{"chain_id":"hone","type":"SENSOR_DATA_COMMIT","sensor_id":"%s","owner":"testacct","batch_hash":"%s",'
               '"reading_count":12,"sensor_type":"continuous","signed_by":"testacct"}' % (sid, bh))
        good = {"sensor_id": sid, "owner": "testacct", "batch_hash": bh, "reading_count": 12, "sensor_type": "continuous",
                "signature": flipper.sign(msg.encode()).signature.hex(), "device_pubkey": fpub,
                "t_min": now // 60, "cell": h3.latlng_to_cell(13.68, -89.24, 10), "rssi": -60}
        forged = dict(good, batch_hash="cd" * 32)
        fb = {"v": 1, "kind": "flipper_frames", "account": "testacct", "device": dev, "ts": now, "frames": [good, forged]}
        c, r = call("POST", "/api/flipper/frames", env(fb, seat, "seat")); assert r["stored"] == 1 and r["rejected"] == 1, r
        c, r = call("POST", "/api/flipper/frames", env(dict(fb, device=s2[:16], frames=[good]), seat2, "seat")); assert r["duplicates"] == 1, r
        c, r = call("GET", "/api/flipper/frames"); assert len(r) == 1 and r[0]["heard_by"] == 2 and r[0]["cells"] == [cell7], r
        c, r = call("GET", "/api/flipper/map?res=5"); assert r[0]["frames"] == 1, r
        # 8. sealed-epoch counts endpoint (node unreachable in the test: zero, but well-formed)
        c, r = call("GET", "/api/devices/seals/testacct@" + seat_pub); assert c == 200 and r["sealed_today"] == 0, r
        c, r = call("GET", "/api/devices/seals/Bad%20Id"); assert c == 400, r
        print("ALL INDEXER TESTS PASSED")
    finally:
        srv.terminate()
        err = srv.stderr.read().decode()
        print("server log (rejects):", "\n".join(l for l in err.splitlines() if "REJECT" in l))


if __name__ == "__main__":
    main()
