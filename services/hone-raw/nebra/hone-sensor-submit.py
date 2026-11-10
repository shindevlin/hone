#!/usr/bin/env python3
"""Send queued nebra SENSOR_SUMMARYs (/srv/hone/sensors/queue) to the local node.
Holds everything until conf.json "submit" names an activation epoch that the chain has
reached and a signer. Signer "seat": nebra's own device key signs as seat josh@<pubkey>
(the seat must be registered/backed by josh on chain first). Josh's own keys are never used."""
import glob, json, os, sys, time, urllib.request
BASE = "/srv/hone/sensors"; NODE = "http://127.0.0.1:4242"
CONF = json.load(open(f"{BASE}/conf.json")); SUB = CONF.get("submit") or {}
MAX_PER_RUN = SUB.get("max_per_run", 24)

def get(path):
    with urllib.request.urlopen(NODE + path, timeout=10) as r: return json.loads(r.read())
def post(path, obj):
    req = urllib.request.Request(NODE + path, data=json.dumps(obj).encode(), headers={"Content-Type": "application/json"}, method="POST")
    with urllib.request.urlopen(req, timeout=20) as r: return json.loads(r.read())

def reconcile():
    """Sent entries wait in state/sent until the chain commits them; rejected ones go to state/rejected."""
    for f in sorted(glob.glob(f"{BASE}/state/sent/*.json")):
        q = json.load(open(f))
        st = get(f"/api/entry/{q['hash']}/status").get("status", "unknown")
        if st == "committed": os.remove(f)
        elif st == "rejected":
            os.makedirs(f"{BASE}/state/rejected", exist_ok=True); os.replace(f, f"{BASE}/state/rejected/{os.path.basename(f)}")

def main():
    files = sorted(glob.glob(f"{BASE}/queue/*.json"))
    act = SUB.get("active_from_epoch")
    if not act or SUB.get("signer") != "seat":
        print(f"hold: {len(files)} queued, submit not configured"); return
    info = get("/api/node/info"); epoch = info["epoch"]; chain_id = info.get("chain_id", "hone")
    if epoch < act:
        print(f"hold: {len(files)} queued, epoch {epoch} < {act}"); return
    from nacl.signing import SigningKey
    sk = SigningKey(open("/home/pi/.hone-nebra-device/device.key", "rb").read())
    seat = f"{CONF['owner']}@{sk.verify_key.encode().hex()}"
    reconcile(); os.makedirs(f"{BASE}/state/sent", exist_ok=True)
    sent = 0
    for f in files[:MAX_PER_RUN]:
        q = json.load(open(f))
        e = {"sensor_id": q["sensor_id"], "owner": seat, "batch_hash": q.get("batch_hash", ""), "cell": q["cell"],
             "window": q["window"], "metrics": q["metrics"]}
        msg = json.dumps({"chain_id": chain_id, "type": "SENSOR_SUMMARY", **e}, separators=(",", ":"), sort_keys=True)
        r = post("/api/sensor/summary", {**e, "signature": sk.sign(msg.encode()).signature.hex()})
        if r.get("accepted"):
            q["hash"] = r.get("hash"); json.dump(q, open(f"{BASE}/state/sent/{os.path.basename(f)}", "w")); os.remove(f); sent += 1
            print(f"sent {os.path.basename(f)} {r.get('hash')}")
        else:
            print(f"refused {os.path.basename(f)}: {r.get('error')}"); break
        time.sleep(1)
    print(f"done: sent {sent}, left {len(files) - sent}")

if __name__ == "__main__":
    main()
