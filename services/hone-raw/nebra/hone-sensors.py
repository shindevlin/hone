#!/usr/bin/env python3
"""HONE nebra sensor collector: LoRa (Semtech UDP from lora_pkt_fwd on 127.0.0.1:1700)
and Wi-Fi (passive `iw` scan of wlan0, never associates). One summary per family per
15-minute window, queued as JSON in /srv/hone/sensors/queue for SENSOR_SUMMARY.
Raw DevAddrs, BSSIDs and SSIDs never leave this process (only counts, stats and
hashed tokens are written). Health goes to /srv/hone/sensors/status.json (device_status).

2026-11-10 (send everything): every scan and LoRa uplink also feeds a change-based raw record
(hone_rawfmt, schema registry hone.raw@1): a keyframe at boot / each UTC day / after a gap,
then appeared / disappeared / changed events, explicit unchanged markers and gap reasons.
One gzip JSONL blob per window is queued in /srv/hone/sensors/raw-queue and uploaded, signed with
nebra's device key, to honemesh.net/api/raw/upload; its sha256 goes into the window's summary
batch_hash. Sizes of the change-based blob vs. a full-scan log go to state/raw_sizes.jsonl."""
import base64, datetime, gzip, hashlib, json, os, re, socket, subprocess, sys, threading, time
sys.path.insert(0, "/srv/hone/sensors")
import hone_rawfmt as RF, hone_raw_client as RC

BASE = "/srv/hone/sensors"
CONF = json.load(open(f"{BASE}/conf.json"))
QUEUE = f"{BASE}/queue"
STATE = f"{BASE}/state"
RAWQ = f"{BASE}/raw-queue"
FW = "nebra-sensors 2026-11-10"
WIN = 900
os.makedirs(QUEUE, exist_ok=True); os.makedirs(STATE, exist_ok=True)
lock = threading.Lock()

def window_of(t): return int(t) // WIN

def p50(xs):
    s = sorted(xs); n = len(s)
    return s[n // 2] if n % 2 else (s[n // 2 - 1] + s[n // 2]) / 2

def write_json(path, obj):
    tmp = path + ".tmp"
    with open(tmp, "w") as f: json.dump(obj, f, separators=(",", ":"), sort_keys=True)
    os.replace(tmp, path)

# -- daily key for DevAddr hashing: tmpfs only, regenerated each UTC day --
_dk = {"day": None, "key": None}
def day_key():
    day = time.strftime("%Y-%m-%d", time.gmtime())
    if _dk["day"] != day:
        p = f"/run/hone-sensors/daykey-{day}"
        try:
            k = open(p, "rb").read()
        except OSError:
            k = os.urandom(32)
            os.makedirs("/run/hone-sensors", exist_ok=True)
            for old in os.listdir("/run/hone-sensors"):
                if old.startswith("daykey-"): os.remove(f"/run/hone-sensors/{old}")
            fd = os.open(p, os.O_WRONLY | os.O_CREAT, 0o600); os.write(fd, k); os.close(fd)
        _dk.update(day=day, key=k)
    return _dk["key"]

# -- LoRa --
class Lora:
    def __init__(self): self.reset(window_of(time.time())); self.last_stat = None; self.pkts_total = 0
    def reset(self, w):
        self.w = w; self.ok = 0; self.bad = 0; self.rssi = []; self.snr = []
        self.ch = [0] * 8; self.dev = set(); self.heard = {}   # token -> (rssi, attrs)
    def add(self, pk):
        st = pk.get("stat")
        if st == -1: self.bad += 1; return
        if st != 1: return
        self.ok += 1; self.pkts_total += 1
        if "rssi" in pk: self.rssi.append(pk["rssi"])
        if "lsnr" in pk: self.snr.append(pk["lsnr"])
        c = pk.get("chan")
        if isinstance(c, int) and 0 <= c < 8: self.ch[c] += 1
        try:
            b = base64.b64decode(pk.get("data", ""))
            if len(b) >= 5 and (b[0] >> 5) in (2, 4):  # unconfirmed / confirmed data up
                d8 = hashlib.sha256(day_key() + b[1:5]).digest()[:8]
                self.dev.add(d8)
                k = "lora:" + d8.hex()
                r = pk.get("rssi", -200)
                if k not in self.heard or r > self.heard[k][0]:
                    self.heard[k] = (r, {"ch": c if isinstance(c, int) else None, "snr": pk.get("lsnr"), "dr": pk.get("datr")})
        except Exception:
            pass
    def metrics(self):
        m = {"lora.packets": self.ok, "lora.crc_bad": self.bad, "lora.devaddr_unique": len(self.dev)}
        for i, n in enumerate(self.ch): m[f"lora.ch{i}"] = n
        if self.rssi:
            m["lora.rssi_p50"] = round(p50(self.rssi)); m["lora.rssi_max"] = round(max(self.rssi))
        if self.snr: m["lora.snr_p50"] = round(p50(self.snr))
        return m

def lora_server(lo):
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.bind(("127.0.0.1", CONF.get("lora_port", 1700)))
    while True:
        d, a = s.recvfrom(65535)
        if len(d) < 4 or d[0] not in (1, 2): continue
        ident = d[3]
        if ident == 0x00:  # PUSH_DATA
            s.sendto(bytes([d[0], d[1], d[2], 0x01]), a)
            try: j = json.loads(d[12:].decode("utf-8", "replace"))
            except ValueError: continue
            with lock:
                for pk in j.get("rxpk", []): lo.add(pk)
                if "stat" in j: lo.last_stat = {"t": int(time.time()), **{k: j["stat"].get(k) for k in ("rxnb", "rxok", "rxfw")}}
        elif ident == 0x02:  # PULL_DATA keepalive
            s.sendto(bytes([d[0], d[1], d[2], 0x04]), a)

# -- Wi-Fi --
MOBILE_SSID = re.compile(r"iphone|android|galaxy|pixel|hotspot|direct-|redmi|xiaomi|huawei|moto|oppo|vivo|"
                         r"oneplus|realme|tecno|infinix|honor|mifi|jetpack|nokia|samsung|\bmi \d|tether", re.I)
class Wifi:
    def __init__(self): self.reset(window_of(time.time())); self.scans_total = 0; self.err = None
    def reset(self, w): self.w = w; self.scans = 0; self.aps = {}  # bssid -> {"rssi": [], "freq": f, "mobile": b}
    def add_scan(self, bss):
        self.scans += 1; self.scans_total += 1
        for b, v in bss.items():
            a = self.aps.setdefault(b, {"rssi": [], "freq": v["freq"], "mobile": False})
            a["rssi"].append(v["signal"])
            if v.get("mobile"): a["mobile"] = True
    def metrics(self, cell):
        if not self.scans: return None
        m = {"wifi.ap_count": len(self.aps), "wifi.scans": self.scans}
        best = [max(a["rssi"]) for a in self.aps.values()]
        if best: m["wifi.rssi_p50"] = round(p50(best)); m["wifi.rssi_max"] = round(max(best))
        for ch, f in ((1, 2412), (6, 2437), (11, 2462)):
            m[f"wifi.ch{ch}"] = sum(1 for a in self.aps.values() if a["freq"] == f)
        # fp tokens: strongest fixed APs (seen in >= 60% of scans, globally administered, not a
        # phone hotspot, never seen from another cell by this device)
        cells = load_ap_cells(); week = "%04d-W%02d" % datetime.date.today().isocalendar()[:2]
        fixed = []
        for b, a in self.aps.items():
            hb = hashlib.sha256(b.encode()).hexdigest()[:16]
            seen = cells.setdefault(hb, [])
            if cell not in seen: seen.append(cell)
            if a["mobile"] or int(b[:2], 16) & 0x02 or len(seen) > 1: continue
            if len(a["rssi"]) < 0.6 * self.scans: continue
            fixed.append((p50(a["rssi"]), b))
        save_ap_cells(cells)
        for r, b in sorted(fixed, reverse=True)[: CONF.get("fp_top", 5)]:
            tok = hashlib.sha256((cell + week + b).encode()).hexdigest()[:8]
            m[f"wifi.fp:{tok}"] = round(r)
        return m

def load_ap_cells():
    try: return json.load(open(f"{STATE}/ap_cells.json"))
    except (OSError, ValueError): return {}
def save_ap_cells(c): write_json(f"{STATE}/ap_cells.json", c)

def iw_scan(iface):
    out = subprocess.run(["iw", "dev", iface, "scan"], capture_output=True, text=True, timeout=30)
    if out.returncode != 0: raise RuntimeError(out.stderr.strip()[:120])
    bss = {}; cur = None
    for line in out.stdout.splitlines():
        mm = re.match(r"^BSS ([0-9a-f:]{17})", line)
        if mm: cur = mm.group(1).lower(); bss[cur] = {"freq": 0, "signal": -100, "mobile": False}; continue
        if cur is None: continue
        t = line.strip()
        if t.startswith("freq:"): bss[cur]["freq"] = int(float(t.split()[1]))
        elif t.startswith("signal:"): bss[cur]["signal"] = float(t.split()[1])
        elif t.startswith("SSID:"):
            bss[cur]["ssid"] = t[5:].strip()
            if MOBILE_SSID.search(t[5:]): bss[cur]["mobile"] = True
    return {b: v for b, v in bss.items() if 2400 <= v["freq"] <= 2500}

def wifi_loop(wf):
    iface = CONF.get("wifi_iface", "wlan0")
    subprocess.run(["ip", "link", "set", iface, "up"], capture_output=True)
    while True:
        try:
            bss = iw_scan(iface)
            with lock: wf.add_scan(bss); wf.err = None; raw_wifi(bss)
        except Exception as e:
            wf.err = str(e)[:120]
            with lock: RAW["wifi_err"] = wf.err
            subprocess.run(["ip", "link", "set", iface, "up"], capture_output=True)
        time.sleep(CONF.get("wifi_scan_s", 60))

# -- health (device_status, not rewards) --
def health(lo, wf):
    def rd(p, d=None):
        try: return open(p).read().strip()
        except OSError: return d
    mem = dict(l.split(":", 1) for l in open("/proc/meminfo"))
    kb = lambda k: int(mem[k].split()[0]) // 1024
    st = os.statvfs("/srv/hone")
    thr = subprocess.run(["vcgencmd", "get_throttled"], capture_output=True, text=True).stdout.strip().split("=")[-1]
    volt = subprocess.run(["vcgencmd", "measure_volts", "core"], capture_output=True, text=True).stdout.strip().split("=")[-1]
    return {"t": int(time.time()), "temp_c": int(rd("/sys/class/thermal/thermal_zone0/temp", "0")) / 1000,
            "core_v": volt, "throttled": thr, "load1": float(rd("/proc/loadavg").split()[0]),
            "mem_avail_mb": kb("MemAvailable"), "mem_total_mb": kb("MemTotal"),
            "swap_used_mb": kb("SwapTotal") - kb("SwapFree"),
            "disk_free_gb": round(st.f_bavail * st.f_frsize / 1e9, 2),
            "lora": {"packets_since_start": lo.pkts_total, "last_stat": lo.last_stat},
            "wifi": {"scans_since_start": wf.scans_total, "error": wf.err}}

# -- device_status to the HONE devices indexer (health only, never rewards) --
DEV_KEY = "/home/pi/.hone-nebra-device/device.key"   # nebra's own ed25519 seed (usbroot), not Josh's keys
def device_key():
    from nacl.signing import SigningKey
    try:
        return SigningKey(open(DEV_KEY, "rb").read())
    except OSError:
        os.makedirs(os.path.dirname(DEV_KEY), mode=0o700, exist_ok=True)
        sk = SigningKey.generate()
        fd = os.open(DEV_KEY, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600); os.write(fd, bytes(sk)); os.close(fd)
        return sk

def post_status(h):
    import urllib.request
    sk = device_key(); pub = sk.verify_key.encode().hex()
    p = {"v": 1, "kind": "device_status", "account": CONF["owner"], "device": pub[:16], "seat_pub": pub,
         "ts": int(time.time()), "cell7": CONF["cell"], "model": "nebra-indoor-cm3", "health": h}
    ps = json.dumps(p, separators=(",", ":"), sort_keys=True)
    body = json.dumps({"payload": ps, "key": "seat", "pub": pub, "sig": sk.sign(ps.encode()).signature.hex()}).encode()
    req = urllib.request.Request(CONF.get("status_url", "https://honemesh.net/api/devices/status"), data=body,
                                 headers={"Content-Type": "application/json"}, method="POST")
    with urllib.request.urlopen(req, timeout=15) as r:
        return json.loads(r.read() or b"{}")

def emit(family, sensor_id, w, metrics, batch_hash=""):
    if not metrics: return
    write_json(f"{QUEUE}/{w}-{family}.json", {"sensor_id": sensor_id, "owner": CONF["owner"],
               "cell": CONF["cell"], "window": w, "metrics": metrics, "batch_hash": batch_hash})

# -- raw records (change-based, public, tokens only) --
RAW = {"enc": RF.ChangeEncoder(), "full": [], "wifi_err": None, "last_t": None}
def week(): return "%04d-W%02d" % datetime.date.today().isocalendar()[:2]
def raw_wifi(bss):
    t = time.time(); cell = CONF["cell"]; wk = week(); seen = {}; full = []
    for b, v in bss.items():
        a = {"f": v["freq"], "b": "2g"}
        if v.get("ssid"): a["ss"] = RF.token(cell, wk, "ssid", v["ssid"], n=8)
        else: a["hid"] = 1
        if v.get("mobile"): a["mob"] = 1
        k = "wifi:" + RF.token(cell, wk, b)
        seen[k] = (round(v["signal"]), a); full.append([k, round(v["signal"]), a])
    RAW["full"].append({"t": RF.rfc3339(t), "scan": full})
    if not bss: RAW["enc"].gap(t - CONF.get("wifi_scan_s", 60), t, "wifi", "no_signal")
    RAW["enc"].observe_set(t, "wifi", seen); RAW["wifi_err"] = None

def raw_tick(t):
    """Once a minute: device temperature (not air), day keyframe."""
    try: RAW["enc"].observe(t, "dev.temp_c", round(int(open("/sys/class/thermal/thermal_zone0/temp").read()) / 1000, 1))
    except (OSError, ValueError): pass

def raw_close(w, lo):
    """Window w ended: LoRa heard set, markers, gaps; queue the blob; returns its sha256."""
    enc = RAW["enc"]; t = (w + 1) * WIN - 0.001
    enc.observe_set(t, "lora", lo.heard)
    if RAW["wifi_err"]: enc.gap(w * WIN, t, "wifi", "offline")
    enc.gap(w * WIN, t, "env", "sensor_absent")      # nebra has no ambient climate sensor attached
    enc.end_window(t, ["wifi", "lora"])
    sk = device_key(); pub = sk.verify_key.encode().hex()
    hdr = {"s": RF.SCHEMA_HDR, "t": RF.rfc3339(w * WIN), "dev": pub[:16], "owner": CONF["owner"], "cls": "nebra",
           "fw": FW, "model": "nebra-indoor-cm3", "win": w,
           "sensors": {"wifi": "Raspberry Pi CM3 + USB Wi-Fi, passive iw scan, 2.4 GHz", "lora": "SX1301 US915 FSB2, receive-only",
                       "dev.temp_c": "BCM2837 SoC thermal zone (device temperature, not air)", "env": "absent"},
           "area": {"cell": CONF["cell"], "res": int(CONF["cell"][1], 16), "src": "fixed", "acc_m": None, "inferred": False, "age_s": 0},
           "deadbands": RF.DEADBANDS}
    recs = [hdr] + enc.take()
    body = RC.blob(recs)
    full = gzip.compress(json.dumps({"hdr": hdr, "scans": RAW["full"], "lora": {k: v for k, v in lo.heard.items()}},
                                    separators=(",", ":")).encode(), compresslevel=9, mtime=0)
    full_raw = len(json.dumps({"scans": RAW["full"]}, separators=(",", ":")))
    sizes = {"win": w, "records": len(recs), "delta_gz": len(body), "delta_raw": len(RF.to_jsonl(recs)),
             "full_gz": len(full), "full_raw": full_raw, "scans": len(RAW["full"])}
    with open(f"{STATE}/raw_sizes.jsonl", "a") as f: f.write(json.dumps(sizes) + "\n")
    RAW["full"] = []
    return RC.enqueue(RAWQ, w, body, {"cell": CONF["cell"]}), sizes

def raw_upload_loop():
    while True:
        time.sleep(CONF.get("raw_upload_s", 300))
        try:
            sk = device_key()
            r = RC.flush(RAWQ, sk, CONF["owner"], sk.verify_key.encode().hex()[:16], "nebra", url=CONF.get("raw_url", RC.URL))
            RAW["upload"] = {"t": int(time.time()), "sent": r[0], "left": r[1], "error": r[2]}
        except Exception as e:
            RAW["upload"] = {"t": int(time.time()), "error": str(e)[:120]}

def main():
    lo, wf = Lora(), Wifi(); last_h = 0; last_post = 0; last_raw = 0
    t0 = time.time(); RAW["enc"].keyframe(t0, "boot"); RAW["day"] = time.strftime("%Y-%m-%d", time.gmtime(t0))
    threading.Thread(target=raw_upload_loop, daemon=True).start()
    threading.Thread(target=lora_server, args=(lo,), daemon=True).start()
    if CONF.get("wifi", True): threading.Thread(target=wifi_loop, args=(wf,), daemon=True).start()
    while True:
        time.sleep(5)
        w = window_of(time.time())
        with lock:
            if time.time() - last_raw >= 60: last_raw = time.time(); raw_tick(last_raw)
            if w != lo.w:
                try: sha, RAW["sizes"] = raw_close(lo.w, lo)
                except Exception as e: sha = ""; RAW["error"] = str(e)[:160]
                emit("lora", CONF["lora_sensor_id"], lo.w, lo.metrics(), sha); lo.reset(w)
                emit("wifi", CONF["wifi_sensor_id"], wf.w, wf.metrics(CONF["cell"]), sha); wf.reset(w)
                day = time.strftime("%Y-%m-%d", time.gmtime())
                if day != RAW.get("day"): RAW["day"] = day; RAW["enc"].keyframe(time.time(), "day")
        if time.time() - last_h >= 60:
            last_h = time.time()
            try:
                h = health(lo, wf); h["raw"] = {k: RAW.get(k) for k in ("sizes", "upload", "error")}
                write_json(f"{BASE}/status.json", h)
                if time.time() - last_post >= CONF.get("status_post_s", 300):
                    last_post = time.time()
                    try: r = post_status(h); h["posted"] = {"t": int(last_post), "ok": bool(r.get("ok")), "verified": r.get("verified")}
                    except Exception as e: h["posted"] = {"t": int(last_post), "error": str(e)[:120]}
                    write_json(f"{BASE}/status.json", h)
            except Exception: pass

if __name__ == "__main__":
    main()
