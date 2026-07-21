#!/usr/bin/env python3
"""
Verasens sensor pipeline monitor.

Watches for SensorDataCommit -> SensorReward (and GatewayRewardSplit) events
on the HONE node at NODE_URL.  Polls /api/explorer/activity every epoch
and checks sealed blocks for raw sensor entries.

Usage:
    python3 monitor-sensor-pipeline.py [--node http://localhost:4242] [--interval 30]

To submit a test SensorDataCommit (requires PyNaCl: pip install PyNaCl):
    python3 monitor-sensor-pipeline.py --submit-test --posting-key <64-char-hex-seed>

How to sign a SensorDataCommit manually (all other tools):
    See sign_sensor_commit() below — canonical message is compact JSON with keys
    sorted ALPHABETICALLY (serde_json BTreeMap default, no preserve_order feature):
      {"batch_hash":"...","chain_id":"hone-2","owner":"...",
       "reading_count":N,"sensor_id":"...","sensor_type":"...",
       "signed_by":"...","type":"SENSOR_DATA_COMMIT"}
    Sign with ed25519 (posting key seed), hex-encode the 64-byte signature.

Curl commands (node at http://localhost:4242):

  # Node info
  curl -s http://localhost:4242/api/node/info | python3 -m json.tool

  # Register a sensor (signature required if posting key is set on account):
  curl -s -X POST http://localhost:4242/api/sensor/register \
    -H 'Content-Type: application/json' \
    -d '{"sensor_id":"<account>/mydevice","owner":"<account>",
         "sensor_type":"sampled","location":null,"metadata":null,
         "signature":"<128-char-hex>"}'

  # Submit a SensorDataCommit:
  curl -s -X POST http://localhost:4242/api/sensor/commit \
    -H 'Content-Type: application/json' \
    -d '{"sensor_id":"<account>/mydevice","owner":"<account>",
         "batch_hash":"<64-char-sha256-hex>","reading_count":1,
         "sensor_type":"sampled","value":42.0,
         "signature":"<128-char-hex>"}'

  # Poll activity feed for recent sensor events:
  curl -s http://localhost:4242/api/explorer/activity | \
    python3 -c "import sys,json; [print(e) for e in json.load(sys.stdin).get('entries',[]) \
    if any(x in e.get('type','') for x in ('SENSOR','Sensor','GATEWAY'))]"

  # Check a sealed block for sensor entries:
  curl -s http://localhost:4242/api/block/<epoch> | python3 -m json.tool
"""

import sys
import time
import json
import hashlib
import argparse
import urllib.request
import urllib.error
from datetime import datetime, timezone


# ── Signing (optional: requires PyNaCl) ────────────────────────────────────

def _try_import_nacl():
    try:
        import nacl.signing
        return nacl.signing
    except ImportError:
        return None


def sign_sensor_commit(
    chain_id: str,
    sensor_id: str,
    owner: str,
    batch_hash: str,
    reading_count: int,
    sensor_type: str,
    posting_key_hex: str,
) -> str:
    """
    Sign a SensorDataCommit canonical message with an ed25519 posting key seed.
    Returns 128-char hex signature.

    Canonical message (insertion order matching tx.rs json! literal, compact JSON no spaces):
      {"chain_id":"...","type":"SENSOR_DATA_COMMIT","sensor_id":"...",
       "owner":"...","batch_hash":"...","reading_count":N,
       "sensor_type":"...","signed_by":"..."}

    Requires: pip install PyNaCl
    """
    nacl_signing = _try_import_nacl()
    if nacl_signing is None:
        raise ImportError("PyNaCl not installed — run: pip install PyNaCl")

    seed = bytes.fromhex(posting_key_hex)
    sk = nacl_signing.SigningKey(seed)

    # Build canonical message — serde_json with preserve_order (indexmap) uses INSERTION order.
    # Key order must match the json!{} literal in tx.rs canonical_signing_message exactly.
    # Do NOT use sort_keys=True — that would produce alphabetical order which does not match.
    msg = json.dumps({
        "chain_id": chain_id,
        "type": "SENSOR_DATA_COMMIT",
        "sensor_id": sensor_id,
        "owner": owner,
        "batch_hash": batch_hash,
        "reading_count": reading_count,
        "sensor_type": sensor_type,
        "signed_by": owner,
    }, separators=(",", ":"))

    signed = sk.sign(msg.encode())
    # nacl returns signature + message concatenated; first 64 bytes are the sig
    return signed.signature.hex()


def make_batch_hash(sensor_id: str, epoch: int, readings: list) -> str:
    """SHA-256 of the batch JSON (same as what the device stores off-chain)."""
    batch = json.dumps({
        "sensor_id": sensor_id,
        "epoch": epoch,
        "readings": readings,
    }, separators=(",", ":"), sort_keys=True)
    return hashlib.sha256(batch.encode()).hexdigest()


# ── HTTP helpers ────────────────────────────────────────────────────────────

def ts() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def fetch(url: str, timeout: int = 10):
    try:
        with urllib.request.urlopen(url, timeout=timeout) as r:
            return json.loads(r.read())
    except urllib.error.HTTPError as e:
        if e.code != 404:
            print(f"[{ts()}] HTTP {e.code} fetching {url}", file=sys.stderr)
        return None
    except Exception as e:
        print(f"[{ts()}] Error fetching {url}: {e}", file=sys.stderr)
        return None


def post(url: str, payload: dict, timeout: int = 10):
    body = json.dumps(payload).encode()
    req = urllib.request.Request(url, data=body,
                                  headers={"Content-Type": "application/json"},
                                  method="POST")
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return json.loads(r.read())
    except urllib.error.HTTPError as e:
        body = e.read()
        if e.code == 429:
            # Rate limited — back off 2s to let the window clear
            print(f"[{ts()}] 429 rate limit — sleeping 2s", file=sys.stderr)
            time.sleep(2.0)
        else:
            print(f"[{ts()}] HTTP {e.code} posting to {url}: {body.decode()[:200]}", file=sys.stderr)
        return None
    except Exception as e:
        print(f"[{ts()}] Error posting to {url}: {e}", file=sys.stderr)
        return None


# ── Live API helpers ────────────────────────────────────────────────────────

def get_node_info(node: str) -> dict:
    return fetch(f"{node}/api/node/info") or {}


def get_balance(node: str, account: str) -> dict:
    return fetch(f"{node}/api/balance/{account}") or {}


def get_mempool_status(node: str) -> dict:
    return fetch(f"{node}/api/mempool/status") or {}


def scan_activity_for_sensor(node: str) -> list:
    """
    Poll /api/explorer/activity and return sensor-related entries.
    This endpoint returns the ~20 most recent system entries (rewards, seals).
    """
    data = fetch(f"{node}/api/explorer/activity")
    if not data:
        return []
    entries = data.get("entries", data) if isinstance(data, dict) else data
    if not isinstance(entries, list):
        return []
    return [
        e for e in entries
        if isinstance(e, dict) and any(
            x in e.get("type", "")
            for x in ("SENSOR", "Sensor", "GATEWAY_REWARD", "GatewayReward")
        )
    ]


def scan_block_for_sensor(node: str, epoch: int) -> list:
    """
    Fetch /api/block/:epoch and return sensor-related entries from the payload.
    Falls back to empty list on 404 (epoch not yet written to block store).
    """
    data = fetch(f"{node}/api/block/{epoch}")
    if not data:
        return []
    payload = data.get("payload", {})
    entries = payload.get("entries", []) if isinstance(payload, dict) else []
    return [
        e for e in entries
        if isinstance(e, dict) and any(
            x in e.get("type", "")
            for x in ("SENSOR", "Sensor", "GATEWAY_REWARD", "GatewayReward")
        )
    ]


# ── Adaptive sensor scheduler (Phase 1.2) ───────────────────────────────────
# Port of clients/hone-flipper/hone_scheduler.c
# Three sensor classes:
#   Continuous   — always interesting if not barren (subghz, gnss, coverage, sampled)
#   Event        — interesting when tag/signal found; epoch-capped (nfc, rfid, ibutton)
#   Housekeeping — fixed slow cadence, never backed off (heartbeat)

_SENSOR_TYPES = ["subghz", "gnss", "nfc", "rfid", "ibutton", "coverage", "sampled", "heartbeat"]
_SENSOR_CLASS = {
    "subghz":    "continuous",
    "gnss":      "continuous",
    "coverage":  "continuous",
    "sampled":   "continuous",
    "ibutton":   "continuous",   # always-on: contact sensor, never barren by definition
    "nfc":       "event",
    "rfid":      "event",
    "heartbeat": "housekeeping",
}
# Representative metadata for each type (used when no real sensor data available)
_SENSOR_META = {
    "gnss":      {"lat": 37.7749, "lon": -122.4194, "alt": 10.0, "accuracy": 5.0},
    "subghz":    {"freq_hz": 433920000, "rssi": -72.5, "modulation": "AM270"},
    "heartbeat": {"uptime_s": 3600, "battery_pct": 85, "fw": "1.2.0"},
    "sampled":   {"value": 42.0, "unit": "raw"},
    "rfid":      {"protocol": "EM4100",    "obs_id": "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4"},
    "nfc":       {"protocol": "ISO14443A", "obs_id": "b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5"},
    "ibutton":   {"protocol": "DS1990A",   "obs_id": "c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6"},
    "coverage":  {"mcc": 310, "mnc": 260, "rsrp": -95, "lat": 37.7749, "lon": -122.4194},
}

EPOCH_CAP        = 20    # event sensors stop earning after 20 commits/epoch
MAX_BACKOFF      = 16    # max cycles to skip when barren
HEARTBEAT_EVERY  = 30    # housekeeping fires every N cycles
BASE_DELAY_S     = 1.1   # ~54 cycles/min at full battery — stays under 60 POST/min rate limit


class SensorScheduler:
    """
    Adaptive sensor rotation: yield-weighted priority, exponential barren
    backoff, per-epoch event cap, battery-aware cycle delay.

    Mirrors hone_scheduler.c exactly so device and test submission share
    the same selection logic.
    """

    def __init__(self):
        self.cycle = 0
        self.battery_pct = 100
        self.state = {
            t: {
                "class":          _SENSOR_CLASS[t],
                "backoff":        0,
                "skip_remaining": 0,
                "epoch_yield":    0,
                "total_yield":    0,
                "attempts":       0,
            }
            for t in _SENSOR_TYPES
        }

    def new_epoch(self):
        for t, st in self.state.items():
            st["epoch_yield"] = 0
            if st["class"] == "event":
                st["backoff"] = 0
                st["skip_remaining"] = 0

    def set_battery(self, pct: int):
        self.battery_pct = max(1, min(100, pct))

    def _at_cap(self, t: str) -> bool:
        st = self.state[t]
        return st["class"] == "event" and st["epoch_yield"] >= EPOCH_CAP

    def _eligible(self, t: str) -> bool:
        if t == "heartbeat":
            return (self.cycle % HEARTBEAT_EVERY) == 0
        if self._at_cap(t):
            return False
        return self.state[t]["skip_remaining"] == 0

    def next(self) -> str:
        self.cycle += 1
        for st in self.state.values():
            if st["skip_remaining"] > 0:
                st["skip_remaining"] -= 1

        if self._eligible("heartbeat"):
            return "heartbeat"

        start = self.cycle % len(_SENSOR_TYPES)
        for off in range(len(_SENSOR_TYPES)):
            t = _SENSOR_TYPES[(start + off) % len(_SENSOR_TYPES)]
            if t == "heartbeat":
                continue
            if self._eligible(t):
                return t

        # Everything backed off / capped — pick soonest
        best, best_skip = "subghz", 0xFFFFFFFF
        for t in _SENSOR_TYPES:
            if t == "heartbeat" or self._at_cap(t):
                continue
            if self.state[t]["skip_remaining"] < best_skip:
                best_skip = self.state[t]["skip_remaining"]
                best = t
        return best

    def report(self, t: str, found: bool):
        """Call after each scan with whether interesting data was found."""
        st = self.state[t]
        st["attempts"] += 1
        if found:
            st["epoch_yield"] += 1
            st["total_yield"] += 1
            st["backoff"] = 0
            st["skip_remaining"] = 0
        else:
            if st["backoff"] == 0:
                st["backoff"] = 1
            elif st["backoff"] < MAX_BACKOFF:
                st["backoff"] = min(st["backoff"] * 2, MAX_BACKOFF)
            st["skip_remaining"] = st["backoff"]

    def cycle_delay_s(self) -> float:
        """Battery-aware inter-cycle delay: full=0.5s, low battery=up to ~3x."""
        pct = max(1, self.battery_pct)
        mult = 1.0 + (100 - pct) / 40.0
        return BASE_DELAY_S * mult

    def status(self) -> str:
        parts = []
        for t in _SENSOR_TYPES:
            st = self.state[t]
            cap = "CAP" if self._at_cap(t) else ""
            back = f"b{st['backoff']}" if st["backoff"] > 0 else ""
            tag = "/".join(x for x in [cap, back] if x) or "ok"
            parts.append(f"{t}:{st['total_yield']}y/{st['attempts']}a({tag})")
        return "  ".join(parts)


def _is_interesting(sensor_type: str, metadata: dict, api_resp: dict | None) -> bool:
    """
    Decide whether a scan produced 'interesting' data worth boosting.
    API success = at minimum not-barren for continuous sensors.
    For event sensors: only interesting if an actual tag/signal was present
    (simulated here by non-None obs_id / rssi above noise floor).
    """
    if api_resp is None:
        return False  # submission rejected — treat as barren

    if sensor_type == "heartbeat":
        return True  # always interesting

    if sensor_type in ("nfc", "rfid", "ibutton"):
        # Interesting only if we have a real obs_id (not the synthetic placeholder)
        obs = metadata.get("obs_id", "")
        return bool(obs) and obs != "0" * 32

    if sensor_type == "subghz":
        rssi = metadata.get("rssi", -120)
        return rssi > -100  # above noise floor

    if sensor_type == "gnss":
        accuracy = metadata.get("accuracy", 999)
        return accuracy < 50  # sub-50m fix = real data

    if sensor_type == "coverage":
        rsrp = metadata.get("rsrp", -140)
        return rsrp > -110  # decent cell signal

    return True  # sampled: always interesting


def _register_sensor(node: str, account: str, sensor_type: str, chain_id: str,
                     posting_key_hex: str, sk) -> None:
    """Idempotent sensor registration (re-register is a no-op on the chain)."""
    sensor_id = f"{account}/{sensor_type}-test"
    reg_msg = json.dumps({
        "chain_id": chain_id, "type": "SENSOR_REGISTER",
        "sensor_id": sensor_id, "owner": account,
        "sensor_type": sensor_type, "location": None, "signed_by": account,
    }, separators=(",", ":"))
    reg_sig = sk.sign(reg_msg.encode()).signature.hex() if sk else ""
    post(f"{node}/api/sensor/register", {
        "sensor_id": sensor_id, "owner": account,
        "sensor_type": sensor_type, "location": None,
        "metadata": _SENSOR_META[sensor_type], "signature": reg_sig,
    })


def run_adaptive_scheduler(node: str, account: str, posting_key_hex: str,
                           max_cycles: int = 0) -> None:
    """
    Run the adaptive sensor scheduler until interrupted (or max_cycles if set).

    Cycles at 500ms–1.5s depending on battery. Submits the sensor type the
    scheduler selects — highest-yield, non-barren, not-yet-epoch-capped sensor
    wins each slot.  Prints a status line every 10 cycles.
    """
    nacl_signing = _try_import_nacl()
    if nacl_signing is None:
        raise ImportError("PyNaCl required: pip install PyNaCl")
    seed = bytes.fromhex(posting_key_hex)
    sk = nacl_signing.SigningKey(seed)

    sched = SensorScheduler()
    registered: set = set()
    last_epoch = -1
    cycle_count = 0

    print(f"[{ts()}] Adaptive scheduler starting — node={node} account={account}")
    print(f"[{ts()}] Sensor types: {', '.join(_SENSOR_TYPES)}")

    while True:
        info = get_node_info(node)
        epoch = info.get("epoch", 0)
        chain_id = info.get("chain_id", "hone-2")
        battery = info.get("battery_pct", 100)  # node may not expose this; defaults to 100
        sched.set_battery(battery)

        if epoch != last_epoch:
            sched.new_epoch()
            last_epoch = epoch

        sensor_type = sched.next()
        sensor_id   = f"{account}/{sensor_type}-test"
        metadata    = _SENSOR_META[sensor_type].copy()

        # Register lazily (once per type)
        if sensor_type not in registered:
            _register_sensor(node, account, sensor_type, chain_id, posting_key_hex, sk)
            registered.add(sensor_type)

        # Build and sign commit
        readings   = [{"epoch": epoch, "ts": int(time.time() * 1000), **metadata}]
        batch_hash = make_batch_hash(sensor_id, epoch, readings)
        sig = sign_sensor_commit(
            chain_id=chain_id, sensor_id=sensor_id, owner=account,
            batch_hash=batch_hash, reading_count=1,
            sensor_type=sensor_type, posting_key_hex=posting_key_hex,
        )
        resp = post(f"{node}/api/sensor/commit", {
            "sensor_id": sensor_id, "owner": account,
            "batch_hash": batch_hash, "reading_count": 1,
            "sensor_type": sensor_type, "value": None, "signature": sig,
        })

        found = _is_interesting(sensor_type, metadata, resp)
        sched.report(sensor_type, found)

        st = sched.state[sensor_type]
        marker = "✓" if found else "✗"
        print(f"[{ts()}] [{marker}] epoch={epoch} type={sensor_type:10s} "
              f"yield={st['total_yield']:3d}  backoff={st['backoff']:2d}  "
              f"resp={str(resp)[:60]}")

        cycle_count += 1
        if cycle_count % 10 == 0:
            print(f"[{ts()}] sched: {sched.status()}")

        if max_cycles and cycle_count >= max_cycles:
            break

        time.sleep(sched.cycle_delay_s())


# ── Submit test entry ───────────────────────────────────────────────────────

def submit_test_commit(node: str, account: str, posting_key_hex: str) -> dict | None:
    """
    Register a test sensor (if not already registered) then submit one
    SensorDataCommit for the current epoch.  Returns the API response.

    Sensor ID: "<account>/test"
    Sensor type: "sampled"
    batch_hash: SHA-256 of a minimal batch JSON
    reading_count: 1
    value: 42.0  (representative numeric value for cross-validation)
    """
    info = get_node_info(node)
    epoch = info.get("epoch", 0)
    chain_id = info.get("chain_id", "hone-2")
    sensor_id = f"{account}/test"

    # Step 1: Register sensor (idempotent — node stores in meta, not re-applied)
    reg_sig = sign_sensor_commit(
        chain_id=chain_id,
        sensor_id=sensor_id,
        owner=account,
        batch_hash="0" * 64,  # unused for register, but need a valid call
        reading_count=0,
        sensor_type="sampled",
        posting_key_hex=posting_key_hex,
    ) if posting_key_hex else ""

    # SensorRegister canonical message differs — build separately
    nacl_signing = _try_import_nacl()
    if nacl_signing and posting_key_hex:
        seed = bytes.fromhex(posting_key_hex)
        sk = nacl_signing.SigningKey(seed)
        # canonical_signing_message() for SensorRegister includes:
        # chain_id, type, sensor_id, owner, sensor_type, location, signed_by
        # Note: metadata is NOT part of the signed message (excluded in tx.rs).
        reg_msg = json.dumps({
            "chain_id": chain_id,
            "type": "SENSOR_REGISTER",
            "sensor_id": sensor_id,
            "owner": account,
            "sensor_type": "sampled",
            "location": None,
            "signed_by": account,
        }, separators=(",", ":"))
        reg_sig = sk.sign(reg_msg.encode()).signature.hex()
    else:
        reg_sig = ""

    reg_resp = post(f"{node}/api/sensor/register", {
        "sensor_id": sensor_id,
        "owner": account,
        "sensor_type": "sampled",
        "location": None,
        "metadata": None,
        "signature": reg_sig,
    })
    print(f"[{ts()}] SensorRegister response: {json.dumps(reg_resp)}")

    # Step 2: Build batch hash from a minimal reading set
    readings = [{"epoch": epoch, "value": 42.0, "ts": int(time.time() * 1000)}]
    batch_hash = make_batch_hash(sensor_id, epoch, readings)

    # Step 3: Sign the commit
    sig = sign_sensor_commit(
        chain_id=chain_id,
        sensor_id=sensor_id,
        owner=account,
        batch_hash=batch_hash,
        reading_count=1,
        sensor_type="sampled",
        posting_key_hex=posting_key_hex,
    )

    print(f"[{ts()}] Submitting SensorDataCommit: sensor={sensor_id} epoch={epoch} "
          f"batch_hash={batch_hash[:12]}... sig={sig[:12]}...")

    resp = post(f"{node}/api/sensor/commit", {
        "sensor_id": sensor_id,
        "owner": account,
        "batch_hash": batch_hash,
        "reading_count": 1,
        "sensor_type": "sampled",
        "value": 42.0,
        "signature": sig,
    })
    print(f"[{ts()}] SensorDataCommit response: {json.dumps(resp)}")
    return resp


# ── Main monitor loop ───────────────────────────────────────────────────────

def monitor(node: str, interval: int):
    print(f"[{ts()}] Verasens sensor pipeline monitor")
    print(f"[{ts()}] Node: {node}  Poll interval: {interval}s")

    info = get_node_info(node)
    account = info.get("account", "natoshisakamoto")
    print(f"[{ts()}] chain={info.get('chain_id')} epoch={info.get('epoch')} "
          f"peers={info.get('peer_count')} account={account}")
    print()

    last_epoch_seen = info.get("epoch", 0)
    seen_event_keys: set = set()

    while True:
        info = get_node_info(node)
        current_epoch = info.get("epoch", last_epoch_seen)
        account = info.get("account", account)

        # ── Activity feed: fast path for recent entries ──────────────────
        for entry in scan_activity_for_sensor(node):
            key = (entry.get("type"), entry.get("epoch") or entry.get("_epoch"),
                   entry.get("sensor_id") or entry.get("node_id") or entry.get("sensor_account"))
            if key in seen_event_keys:
                continue
            seen_event_keys.add(key)
            t = entry.get("type", "")
            ep = entry.get("epoch") or entry.get("_epoch", "?")

            if "COMMIT" in t.upper() or "Commit" in t:
                print(f"[{ts()}] SENSOR_DATA_COMMIT  epoch={ep}  "
                      f"sensor={entry.get('sensor_id','?')}  "
                      f"owner={entry.get('owner','?')}  "
                      f"readings={entry.get('reading_count','?')}  "
                      f"type={entry.get('sensor_type','?')}")

            elif "SENSOR_REWARD" in t.upper() or t in ("SensorReward", "SENSOR_REWARD"):
                dreams = entry.get("amount", 0)
                hone  = dreams / 10_000_000_000
                print(f"[{ts()}] SENSOR_REWARD        epoch={ep}  "
                      f"node={entry.get('node_id','?')}  "
                      f"amount={hone:.6f} HONE  ({dreams} dreams)")

            elif "GATEWAY" in t.upper():
                print(f"[{ts()}] GATEWAY_REWARD_SPLIT epoch={ep}  "
                      f"sensor={entry.get('sensor_account','?')}  "
                      f"gateway={entry.get('gateway_account','?')}  "
                      f"sensor_amt={entry.get('sensor_amount',0)} dreams  "
                      f"gateway_amt={entry.get('gateway_amount',0)} dreams")

        # ── Block scan for each newly sealed epoch ───────────────────────
        if current_epoch > last_epoch_seen:
            for ep in range(last_epoch_seen + 1, current_epoch + 1):
                block_entries = scan_block_for_sensor(node, ep)
                commits = [e for e in block_entries
                           if "COMMIT" in e.get("type","").upper() or "Commit" in e.get("type","")]
                rewards = [e for e in block_entries
                           if "REWARD" in e.get("type","").upper() or "Reward" in e.get("type","")]
                if commits or rewards:
                    print(f"[{ts()}] === Epoch {ep}: "
                          f"{len(commits)} commit(s), {len(rewards)} reward(s) ===")
                    for e in commits + rewards:
                        print(f"  {json.dumps(e)}")
                else:
                    print(f"[{ts()}] Epoch {ep} sealed — no sensor activity")
            last_epoch_seen = current_epoch

        # ── Periodic status line ─────────────────────────────────────────
        mpool = get_mempool_status(node)
        bal   = get_balance(node, account)
        print(f"[{ts()}] epoch={current_epoch}  peers={info.get('peer_count',0)}  "
              f"mempool={mpool.get('pending_count',0)}  "
              f"balance={bal.get('balance','?')} HONE")

        time.sleep(interval)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Verasens sensor pipeline monitor")
    parser.add_argument("--node",        default="http://localhost:4242")
    parser.add_argument("--interval",    type=int, default=30,
                        help="Poll interval in seconds")
    parser.add_argument("--submit-test", action="store_true",
                        help="Submit one test SensorDataCommit (sampled type) then exit")
    parser.add_argument("--rotate-test", action="store_true",
                        help="Run adaptive sensor scheduler: cycles at ~500ms, yield-weighted "
                             "priority, exponential barren backoff, epoch cap for event sensors. "
                             "Mirrors hone_scheduler.c. Requires --posting-key.")
    parser.add_argument("--account", default="",
                        help="Override account name (default: read from node /api/node/info)")
    parser.add_argument("--posting-key", default="",
                        help="64-char hex ed25519 seed (HONE_POSTING_KEY) for signing")
    args = parser.parse_args()

    if args.rotate_test:
        if not args.posting_key:
            print("ERROR: --rotate-test requires --posting-key <64-char-hex-seed>",
                  file=sys.stderr)
            sys.exit(1)
        info = get_node_info(args.node)
        account = args.account or info.get("account", "natoshisakamoto")
        try:
            run_adaptive_scheduler(args.node, account, args.posting_key)
        except KeyboardInterrupt:
            print(f"\n[{ts()}] Scheduler stopped.")
        sys.exit(0)

    if args.submit_test:
        if not args.posting_key:
            print("ERROR: --submit-test requires --posting-key <64-char-hex-seed>",
                  file=sys.stderr)
            sys.exit(1)
        info = get_node_info(args.node)
        account = args.account or info.get("account", "natoshisakamoto")
        submit_test_commit(args.node, account, args.posting_key)
        sys.exit(0)

    try:
        monitor(args.node, args.interval)
    except KeyboardInterrupt:
        print(f"\n[{ts()}] Stopped.")
