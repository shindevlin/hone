"""hone.raw v1: change-based, self-describing sensor records (JSONL). Josh 2026-11-10 10:43.

Every line is one record with "s" = schema_id@version and "t" = RFC3339 UTC time.
  hone.raw.hdr@1  blob header: device, owner, class, firmware, model, window, area, registry hash, deadbands
  hone.raw.kf@1   keyframe: the full state {key: {"v": value, "a": {attrs}}}; why = boot | day | gap
  hone.raw.ev@1   event: e = appeared | disappeared | changed | unchanged | gap
                  appeared/changed carry v (and a); disappeared carries prev; unchanged carries
                  since (and v for forced climate samples); gap carries reason, from, to.
Keys: "<family>:<id>" for things that come and go (wifi:<token>, ble:<token>, lora:<token>,
flipper:<id>) and "<family>.<metric>" for measured quantities (env.air_temp_c, env.rh_pct,
env.pressure_pa, env.light_lux, dev.temp_c ...). Units are in the schema registry.
Rebuild: state at T = last keyframe at or before T, then appeared/changed set, disappeared
deletes, unchanged and gap change nothing. rebuild() and ChangeEncoder.reported always agree.
"""
import datetime as dt, hashlib, json

SCHEMA_HDR, SCHEMA_KF, SCHEMA_EV = "hone.raw.hdr@1", "hone.raw.kf@1", "hone.raw.ev@1"
EVENTS = ("appeared", "disappeared", "changed", "unchanged", "gap")
GAP_REASONS = ("offline", "permission_denied", "speech_deleted", "no_signal", "sensor_absent", "app_stopped")

# Deadbands (Josh 10:43). abs = absolute units of the key, rel = fraction of the last reported value.
DEADBANDS = {
    "wifi:": {"abs": 6.0}, "ble:": {"abs": 6.0}, "lora:": {"abs": 6.0}, "flipper:": {"abs": 6.0},
    "env.air_temp_c": {"abs": 0.15}, "env.rh_pct": {"abs": 1.0}, "env.pressure_pa": {"abs": 10.0},
    "env.light_lux": {"rel": 0.10}, "dev.temp_c": {"abs": 1.0},
}
FORCED = ("env.", "dev.temp_c")       # sampled every window even when unchanged (complete daily curves)


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


class ChangeEncoder:
    """Turns observations into keyframes and change events. `reported` is what a reader rebuilds."""

    def __init__(self, miss=2):
        self.miss = miss                      # consecutive scans an id may be missing before "disappeared"
        self.reported = {}                    # key -> {"v": value, "a": attrs}
        self.changed_at = {}                  # key -> t of its last event
        self.missing = {}                     # key -> consecutive misses
        self.out = []

    def _ev(self, t, e, k, **kw):
        r = {"s": SCHEMA_EV, "t": rfc3339(t), "e": e, "k": k}
        r.update({a: b for a, b in kw.items() if b is not None})
        self.out.append(r)

    def keyframe(self, t, why):
        self.out.append({"s": SCHEMA_KF, "t": rfc3339(t), "why": why,
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
                self._ev(t, "appeared", k, v=v, a=a or None)
            elif moved(k, cur["v"], v):
                self._ev(t, "changed", k, v=v, prev=cur["v"])
                cur["v"] = v
                self.changed_at[k] = t
        for k in [k for k in self.reported if k.startswith(family + ":") and k not in seen]:
            self.missing[k] = self.missing.get(k, 0) + 1
            if self.missing[k] >= self.miss:
                prev = self.reported.pop(k)["v"]
                self.missing.pop(k, None)
                self.changed_at[k] = t
                self._ev(t, "disappeared", k, prev=prev)

    def observe(self, t, k, v, a=None):
        """One measured quantity (climate, device temperature)."""
        cur = self.reported.get(k)
        if cur is None:
            self.reported[k] = {"v": v, "a": a or {}}
            self.changed_at[k] = t
            self._ev(t, "appeared", k, v=v, a=a or None)
        elif moved(k, cur["v"], v):
            self._ev(t, "changed", k, v=v, prev=cur["v"])
            cur["v"] = v
            self.changed_at[k] = t

    def gap(self, t0, t1, family, reason):
        assert reason in GAP_REASONS
        self._ev(t1, "gap", family + ":*", reason=reason, **{"from": rfc3339(t0), "to": rfc3339(t1)})

    def end_window(self, t, families):
        """Explicit markers: everything in each family not mentioned is unchanged since its last event;
        forced climate samples carry their current reported value."""
        for f in families:
            ks = [k for k in self.reported if k.startswith(f + ":")]
            if ks:
                self._ev(t, "unchanged", f + ":*", since=rfc3339(min(self.changed_at.get(k, t) for k in ks)), n=len(ks))
        for k, cur in sorted(self.reported.items()):
            if k.startswith(FORCED):
                self._ev(t, "unchanged", k, v=cur["v"], since=rfc3339(self.changed_at.get(k, t)), forced=True)

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
        if s == SCHEMA_KF:
            state = {k: dict(v) for k, v in r["state"].items()}
        elif s == SCHEMA_EV and state is not None:
            e, k = r["e"], r["k"]
            if e in ("appeared", "changed"):
                cur = state.setdefault(k, {"v": None, "a": {}})
                cur["v"] = r["v"]
                if "a" in r:
                    cur["a"] = r["a"]
            elif e == "disappeared":
                state.pop(k, None)
    return state


def to_jsonl(records):
    return "".join(json.dumps(r, separators=(",", ":"), sort_keys=True) + "\n" for r in records).encode()


def from_jsonl(b):
    return [json.loads(l) for l in b.decode().splitlines() if l.strip()]
