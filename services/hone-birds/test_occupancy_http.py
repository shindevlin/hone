"""HTTP test for /api/birds/windows, /labels and /occupancy (temp server, throwaway keys)."""
import json, os, subprocess, sys, tempfile, time, urllib.error, urllib.request

import h3
from nacl.signing import SigningKey

HERE = os.path.dirname(os.path.abspath(__file__))
PORT = 21460 + os.getpid() % 500
BASE = f"http://127.0.0.1:{PORT}"


def call(method, path, body=None):
    data = json.dumps(body).encode() if body is not None else None
    r = urllib.request.Request(BASE + path, data=data, method=method, headers={"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(r, timeout=20) as x:
            return x.status, json.loads(x.read())
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read() or b"{}")


def signed(sk, payload):
    ps = json.dumps(payload, sort_keys=True, separators=(",", ":"))
    return {"payload": ps, "key": "posting", "pub": sk.verify_key.encode().hex(), "sig": sk.sign(ps.encode()).signature.hex()}


def main():
    data = tempfile.mkdtemp(prefix="hb-occ-")
    sk = SigningKey.generate()
    json.dump({"tester": sk.verify_key.encode().hex()}, open(os.path.join(data, "posting_keys.json"), "w"))
    env = dict(os.environ, HONE_BIRDS_DATA=data, HONE_BIRDS_PORT=str(PORT), HONE_DEVICES_ADMIN_TOKEN="x",
               HONE_NODE_URL="http://127.0.0.1:1", HONE_SEALS="0")
    env.pop("AGENTMAIL_HONEHELP", None)
    srv = subprocess.Popen([sys.executable, os.path.join(HERE, "hone_birds.py")], env=env, stderr=subprocess.PIPE)
    try:
        for _ in range(50):
            try: urllib.request.urlopen(BASE + "/api/birds/health", timeout=1); break
            except Exception: time.sleep(0.2)
        c9 = h3.latlng_to_cell(13.68, -89.24, 9); c7 = h3.cell_to_parent(c9, 7)
        w0 = int(time.time()) // 900 - 20
        for i in range(8):
            p = {"v": 1, "sid": f"s{i}", "window": w0 + i, "cell": c7, "frames": 60, "human": 2, "silenced": 0,
                 "tz_min": -360, "source": "phone", "account": "tester",
                 "species": [{"name": "Pitangus sulphuratus", "cell": c9, "events": 2, "conf_q8": 220}]}
            c, r = call("POST", "/api/birds/windows", signed(sk, p)); assert c == 200 and r["ok"], (c, r)
        c, r = call("POST", "/api/birds/windows", signed(sk, dict(p, cell=c9))); assert c == 400, (c, r)   # effort at res 9 refused
        c, r = call("GET", f"/api/birds/occupancy?cell={c7}"); assert c == 200, r
        sp = r["species"]; assert sp and sp[0]["latin"] == "Pitangus sulphuratus" and sp[0]["status"] == "Heard recently", sp
        c, r = call("POST", "/api/birds/labels", signed(sk, {"v": 1, "account": "tester", "labels": []})); assert c == 400, r
        c, r = call("POST", "/api/birds/labels", signed(sk, {"v": 1, "account": "tester", "opt_in": True,
                    "labels": [{"latin": "Pitangus sulphuratus", "conf_q8": 200, "label": True, "t": 1}]})); assert r["stored"] == 1, r
        c, r = call("GET", "/api/birds/occupancy/map?res=7"); assert c == 200, r
        c, r = call("GET", "/api/birds/occupancy/changes"); assert c == 200, r
        print("occupancy view:", sp[0])
        print("ALL OCCUPANCY HTTP TESTS PASSED")
    finally:
        srv.terminate(); srv.wait(5)


if __name__ == "__main__":
    main()
