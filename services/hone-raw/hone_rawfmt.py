"""hone.raw v1: change-based, self-describing sensor records (JSONL). Josh 2026-11-10 10:43.

Every line is one record with "s" = schema_id@version and "t" = RFC3339 UTC time.
  hone.raw.hdr@1  blob header: device, owner, class, firmware, model, window, area, registry hash, deadbands
  hone.raw.kf@2   keyframe: the full state {key: {"v": value, "a": {attrs}}}; why = boot | day | gap
  hone.raw.ev@2   event: e = appeared | disappeared | changed | unchanged | gap
                  appeared/changed carry v (and a); disappeared carries prev; unchanged carries
                  since (and v for forced climate samples); gap carries reason, from, to.
Keys: "<family>:<id>" for things that come and go (wifi:<token>, ble:<token>, lora:<token>,
flipper:<id>) and "<family>.<metric>" for measured quantities (env.air_temp_c, env.rh_pct,
env.pressure_pa, env.light_lux, dev.temp_c ...). Units are in the schema registry.
  hone.raw.obs@1  observation payload that is not state (a LoRa packet as received, a Flipper's
                  signed frame relayed unaltered, a phone+Flipper pairing); rebuild ignores it.
Provenance (Josh 11:22): EVERY record carries dev (device id), cls (device class), sen (sensor),
why (purpose / reason) and loc (where the area came from, or why there is none). The encoder
refuses to emit without them and the ingest refuses blobs that lack them.
Rebuild: state at T = last keyframe at or before T, then appeared/changed set, disappeared
deletes, unchanged and gap change nothing. rebuild() and ChangeEncoder.reported always agree.
"""
import datetime as dt, hashlib, json

SCHEMA_HDR, SCHEMA_KF, SCHEMA_EV, SCHEMA_OBS = "hone.raw.hdr@1", "hone.raw.kf@3", "hone.raw.ev@3", "hone.raw.obs@2"
# Accepted on read (older devices still in the field). v2 has provenance, v3 adds obs_type.
SCHEMAS_KF = ("hone.raw.kf@2", "hone.raw.kf@3"); SCHEMAS_EV = ("hone.raw.ev@2", "hone.raw.ev@3")
SCHEMAS_OBS = ("hone.raw.obs@1", "hone.raw.obs@2")
PROV = ("dev", "cls", "sen", "why", "loc", "obs_type")

# Observation types (Josh 2026-11-14): every reading is normalized into one of these.
OBS_TYPES = {
    "pressure": "Barometric air pressure (Pa).",
    "temp_humidity": "Ambient air temperature (degC) and relative humidity (%RH).",
    "light": "Ambient light (lx).",
    "cell_coverage": "Mobile network coverage: carrier (MCC-MNC), RAT (LTE/NR/...), RSRP/RSRQ/SINR or the RAT's equivalent; cell id hashed.",
    "wifi_env": "Wi-Fi environment: access points (tokens) and their signal.",
    "ble_density": "Bluetooth LE devices around (tokens rotate daily) and their signal.",
    "subghz": "Sub-GHz radio activity (Flipper and other sub-GHz receivers).",
    "lora": "LoRa / LoRaWAN uplinks heard (packets as received plus PHY metadata).",
    "aircraft": "Aircraft transponders heard (ADS-B / Mode S).",
    "birds_noise": "Bird detections and ambient sound level (speech is never kept).",
    "outage_signal": "Signs of an outage: coverage, Wi-Fi or power disappearing across many devices in a cell.",
    "foot_traffic": "People passing: BLE crowd counts and their change over time.",
    "device": "The recording device itself (device temperature, battery). Never air temperature.",
    "presence": "Another HONE device heard (Flipper relay, pairing, travel group).",
    "location": "Position fixes (kept on the owner's device; only coarse cells leave it).",
}


def obs_type_of(key):
    """Observation type for a record key."""
    if key in ("env.pressure_pa",): return "pressure"
    if key in ("env.air_temp_c", "env.rh_pct"): return "temp_humidity"
    if key == "env.light_lux": return "light"
    fam = key.split(":", 1)[0] if ":" in key else key.split(".", 1)[0]
    return {"cell": "cell_coverage", "wifi": "wifi_env", "ble": "ble_density", "subghz": "subghz", "lora": "lora",
            "adsb": "aircraft", "aircraft": "aircraft", "bird": "birds_noise", "audio": "birds_noise", "noise": "birds_noise",
            "outage": "outage_signal", "crowd": "foot_traffic", "dev": "device", "flipper": "presence",
            "travel": "presence", "track": "location", "env": "temp_humidity"}.get(fam, "device" if fam == "*" else fam)
EVENTS = ("appeared", "disappeared", "changed", "unchanged", "gap")
GAP_REASONS = ("offline", "permission_denied", "speech_deleted", "no_signal", "sensor_absent", "app_stopped")

# Deadbands (Josh 10:43). abs = absolute units of the key, rel = fraction of the last reported value.
DEADBANDS = {
    "wifi:": {"abs": 6.0}, "ble:": {"abs": 6.0}, "lora:": {"abs": 6.0}, "flipper:": {"abs": 6.0},
    "env.air_temp_c": {"abs": 0.15}, "env.rh_pct": {"abs": 1.0}, "env.pressure_pa": {"abs": 10.0},
    "env.light_lux": {"rel": 0.10}, "dev.temp_c": {"abs": 1.0}, "cell:": {"abs": 6.0},
}
FORCED = ("env.", "dev.temp_c", "cell:")       # sampled every window even when unchanged (complete daily curves)


def rfc3339(ts):
    return dt.datetime.fromtimestamp(ts, dt.timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z")


def parse_t(s):
    return dt.datetime.fromisoformat(s.replace("Z", "+00:00")).timestamp()


def deadband(key):
    if key in DEADBANDS:
        return DEADBANDS[key]
    for p, d in DEADBANDS.items():
        if p.endswith(":") and key.startswith(p):
            return d
    return {"abs": 0.0}


def moved(key, prev, v):
    d = deadband(key)
    if "rel" in d:
        return abs(v - prev) > d["rel"] * max(abs(prev), 1e-9)
    return abs(v - prev) > d["abs"]


def token(*parts, n=12):
    """Public token for a personal identifier (BSSID, SSID, MAC, DevAddr): never the identifier itself."""
    return hashlib.sha256("|".join(parts).encode()).hexdigest()[:n]


def sensor_of(key, sensors):
    """Sensor id for a key: exact key, then 'family.' / 'family:' prefix, from the device's sensor table."""
    if key in sensors:
        return sensors[key]
    fam = key.split(":", 1)[0] if ":" in key else key.split(".", 1)[0]
    return sensors.get(fam, fam)


class ChangeEncoder:
    """Turns observations into keyframes and change events. `reported` is what a reader rebuilds.
    prov = {"dev", "cls", "sensors": {key or family: sensor id}}; set_loc() keeps "loc" current."""

    def __init__(self, miss=2, prov=None):
        prov = prov or {}
        if not prov.get("dev") or not prov.get("cls"):
            raise ValueError("provenance: dev and cls are required")
        self.dev, self.cls, self.sensors = prov["dev"], prov["cls"], dict(prov.get("sensors", {}))
        self.loc = prov.get("loc", "none")
        self.miss = miss                      # consecutive scans an id may be missing before "disappeared"
        self.reported = {}                    # key -> {"v": value, "a": attrs}
        self.changed_at = {}                  # key -> t of its last event
        self.missing = {}                     # key -> consecutive misses
        self.out = []

    def set_loc(self, src):
        self.loc = src or "none"

    def _prov(self, k, why):
        return {"dev": self.dev, "cls": self.cls, "sen": sensor_of(k, self.sensors), "why": why, "loc": self.loc,
                "obs_type": obs_type_of(k)}

    def _ev(self, t, e, k, why, **kw):
        r = {"s": SCHEMA_EV, "t": rfc3339(t), "e": e, "k": k, **self._prov(k, why)}
        r.update({a: b for a, b in kw.items() if b is not None})
        self.out.append(r)

    def obs(self, t, k, why, **kw):
        """A payload record (not state): e.g. a LoRa packet as received, a relayed signed frame."""
        r = {"s": SCHEMA_OBS, "t": rfc3339(t), "k": k, **self._prov(k, why)}
        r.update({a: b for a, b in kw.items() if b is not None})
        self.out.append(r)

    def keyframe(self, t, why):
        self.out.append({"s": SCHEMA_KF, "t": rfc3339(t), "dev": self.dev, "cls": self.cls, "sen": "*",
                         "why": "keyframe:" + why, "loc": self.loc, "obs_type": "*",
                         "state": {k: dict(v) for k, v in sorted(self.reported.items())}})
        for k in self.reported:
            self.changed_at[k] = t

    def observe_set(self, t, family, seen):
        """A full scan of one family: seen = {key: (value, attrs)}. Keys must start with family + ':'."""
        for k, (v, a) in seen.items():
            self.missing.pop(k, None)
            cur = self.reported.get(k)
            if cur is None:
                self.reported[k] = {"v": v, "a": a or {}}
                self.changed_at[k] = t
                self._ev(t, "appeared", k, "scan", v=v, a=a or None)
            elif moved(k, cur["v"], v):
                self._ev(t, "changed", k, "scan:deadband", v=v, prev=cur["v"])
                cur["v"] = v
                self.changed_at[k] = t
        for k in [k for k in self.reported if k.startswith(family + ":") and k not in seen]:
            self.missing[k] = self.missing.get(k, 0) + 1
            if self.missing[k] >= self.miss:
                prev = self.reported.pop(k)["v"]
                self.missing.pop(k, None)
                self.changed_at[k] = t
                self._ev(t, "disappeared", k, f"scan:missing_{self.miss}", prev=prev)

    def observe(self, t, k, v, a=None):
        """One measured quantity (climate, device temperature)."""
        cur = self.reported.get(k)
        if cur is None:
            self.reported[k] = {"v": v, "a": a or {}}
            self.changed_at[k] = t
            self._ev(t, "appeared", k, "sample", v=v, a=a or None)
        elif moved(k, cur["v"], v):
            self._ev(t, "changed", k, "sample:deadband", v=v, prev=cur["v"])
            cur["v"] = v
            self.changed_at[k] = t

    def gap(self, t0, t1, family, reason):
        assert reason in GAP_REASONS
        self._ev(t1, "gap", family + ":*", "gap:" + reason, reason=reason, **{"from": rfc3339(t0), "to": rfc3339(t1)})

    def end_window(self, t, families):
        """Explicit markers: everything in each family not mentioned is unchanged since its last event;
        forced climate samples carry their current reported value."""
        for f in families:
            ks = [k for k in self.reported if k.startswith(f + ":")]
            if ks:
                self._ev(t, "unchanged", f + ":*", "marker:window_end", since=rfc3339(min(self.changed_at.get(k, t) for k in ks)), n=len(ks))
        for k, cur in sorted(self.reported.items()):
            if k.startswith(FORCED):
                self._ev(t, "unchanged", k, "forced_sample:15min", v=cur["v"], since=rfc3339(self.changed_at.get(k, t)), forced=True)

    def take(self):
        out, self.out = self.out, []
        return out


def rebuild(records, until=None):
    """State at `until` (RFC3339 or unix seconds; None = end) from keyframes and events."""
    if isinstance(until, str):
        until = parse_t(until)
    state = None
    for r in records:
        if until is not None and parse_t(r["t"]) > until:
            break
        s = r.get("s")
        if s in SCHEMAS_KF:
            state = {k: dict(v) for k, v in r["state"].items()}
        elif s in SCHEMAS_EV and state is not None:
            e, k = r["e"], r["k"]
            if e in ("appeared", "changed"):
                cur = state.setdefault(k, {"v": None, "a": {}})
                cur["v"] = r["v"]
                if "a" in r:
                    cur["a"] = r["a"]
            elif e == "disappeared":
                state.pop(k, None)
    return state


def check_prov(records):
    """Every record must carry provenance; returns the first offending record or None."""
    for r in records:
        if r.get("s") == SCHEMA_HDR:
            if not all(r.get(f) for f in ("dev", "cls", "win")) or "area" not in r:
                return r
        else:
            need = PROV if str(r.get("s", "")).endswith(("@3", "obs@2")) else PROV[:5]
            if not all(r.get(f) for f in need):
                return r
    return None


def to_jsonl(records):
    return "".join(json.dumps(r, separators=(",", ":"), sort_keys=True) + "\n" for r in records).encode()


def from_jsonl(b):
    return [json.loads(l) for l in b.decode().splitlines() if l.strip()]


# ── co-location (travel groups, Josh 2026-11-14) ─────────────────────────────────────────
def radio_env(records, t0=None, t1=None):
    """What a device's radios saw between t0 and t1 (unix s): Wi-Fi tokens, BLE device count,
    sub-GHz activity times. Tokens are salted with the res-7 area and ISO week, so two devices
    in the same area compare like for like."""
    wifi, ble, sub = set(), set(), []
    for r in records:
        if "t" not in r or r.get("s") == SCHEMA_HDR:
            continue
        t = parse_t(r["t"])
        if (t0 is not None and t < t0) or (t1 is not None and t > t1):
            continue
        k = str(r.get("k", ""))
        if r.get("s") in SCHEMAS_KF:
            for kk in r.get("state", {}):
                if kk.startswith("wifi:"): wifi.add(kk)
                elif kk.startswith("ble:"): ble.add(kk)
        elif k.startswith("wifi:") and k != "wifi:*" and r.get("e") in ("appeared", "changed"):
            wifi.add(k)
        elif k.startswith("ble:") and k != "ble:*" and r.get("e") in ("appeared", "changed"):
            ble.add(k)
        elif k.startswith("subghz:") or "subghz" in str(r.get("why", "")):
            sub.append(t)
    return {"wifi": sorted(wifi), "ble": len(ble), "subghz": sorted(sub)}


def colocate(a, b, threshold=0.35):
    """Score how likely two devices were together from their radio environments.
    wifi: Jaccard of AP tokens (weight 0.6); ble: density similarity (0.2); subghz: share of
    activity peaks that line up within 30 s (0.2). Only parts both sides have count.
    Returns {score, match, parts, proof}; proof = sha256 over the canonical evidence."""
    parts, w = {}, {}
    wa, wb = set(a.get("wifi") or []), set(b.get("wifi") or [])
    if wa and wb:
        parts["wifi"] = round(len(wa & wb) / len(wa | wb), 4); w["wifi"] = 0.6
    ba, bb = a.get("ble"), b.get("ble")
    if ba is not None and bb is not None and (ba or bb):
        parts["ble"] = round(1 - abs(ba - bb) / max(ba, bb, 1), 4); w["ble"] = 0.2
    sa, sb = a.get("subghz") or [], b.get("subghz") or []
    if sa and sb:
        hit = sum(1 for x in sa if any(abs(x - y) <= 30 for y in sb))
        parts["subghz"] = round(hit / len(sa), 4); w["subghz"] = 0.2
    if not w:
        return {"score": 0.0, "match": False, "parts": parts, "proof": None, "reason": "no shared radio evidence"}
    score = round(sum(parts[k] * w[k] for k in w) / sum(w.values()), 4)
    match = score >= threshold and ("wifi" not in parts or parts["wifi"] >= 0.2)
    ev = json.dumps({"a": a, "b": b, "parts": parts, "score": score, "threshold": threshold}, sort_keys=True, separators=(",", ":"))
    return {"score": score, "match": match, "parts": parts, "proof": hashlib.sha256(ev.encode()).hexdigest()}


def home_anchor(next_env, home_env, threshold=0.3):
    """The next sighting after a trip matches the owner's home fingerprint (Wi-Fi Jaccard)."""
    wa, wb = set(next_env.get("wifi") or []), set(home_env.get("wifi") or [])
    if not wa or not wb:
        return False
    return len(wa & wb) / len(wa | wb) >= threshold
