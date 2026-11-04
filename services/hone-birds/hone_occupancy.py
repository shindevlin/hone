"""Bird occupancy scores per (H3 cell, species, season), served by the birds indexer.

Every phone and camera station feeds shared scores; the phone shows a local view.

Score = log-odds L that the species uses the cell this season.
  Prior L0 from the GBIF range: in range and in season / in range out of season / range
  unknown / out of range.
  Evidence up: each distinct vocal event (the phone merges successive frames of one call),
  weighted by calibrated confidence; later events in one window count less. A listener
  (account or station) that is new to the cell or its ring in the last 7 days weighs extra.
  A new day heard adds residency. Capped per day.
  Evidence down (weak, slow): only windows that really listened (non-speech, non-silenced
  frames) count; weighted by the species' detectability at that local hour (owls at noon ~0);
  capped per day so one quiet day barely moves a resident; none out of season. Plus a drift
  back to the prior: fast (days) for one-off entries, slow (weeks) for residents.
  Calibration: temperature scaling of Perch scores per species from yes/no labels (users who
  opted in), global fallback, default T=1.5.

Status (for people): Regular here / Heard recently / Possible / Fading / Gone for the season /
No longer heard.  Tiers (for the map): likely p>=0.90; confirmed p>=0.98 or a second
independent listener; rare (out of range) needs p>=0.99 plus a second listener or a photo.
Change events (arrival = a species reaches likely; departure = a likely species falls below
0.90 after enough listening) are computed here only: no chain submission until a chain
format exists.

Privacy: listening-effort windows arrive at res 7 (or the home-area cell); detection cells
are kept at res 9 at most. Nothing here stores audio or exact positions.
"""
import json, math, re, threading, time

import h3

WINDOW_S = 900
DAY_S = 86400
L_MIN, L_MAX = -9.0, 9.0
P_LIKELY, P_CONFIRMED, P_RARE = 0.90, 0.98, 0.99
PRIOR_P = {"in_season": 0.20, "off_season": 0.03, "unknown": 0.05, "out": 0.003}
EVENT_BASE, EVENT_GAIN = 0.8, 1.0      # per-event log-odds: BASE + GAIN * logit(calibrated conf)
REPEAT_WEIGHT = 0.3                    # 2nd+ events in one window
WINDOW_UP_CAP = 4.0
DAY_UP_CAP = 5.0
NEW_LISTENER_X = 1.5
NEW_DAY_BONUS = 0.7
ABSENCE_PER_WINDOW = 0.08              # x detectability x effort
ABSENCE_DAY_CAP = 0.6
ABSENCE_FLOOR = 3.0                    # absence never pushes more than this below the prior
FULL_EFFORT_FRAMES = 60                # 5 min of real listening in a 15-min window
TAU_ONE_OFF_D, TAU_RESIDENT_D = 3.0, 21.0
DEPARTURE_WINDOWS = 40                 # effective listening windows since last heard
DEFAULT_T = 1.5
NOCTURNAL = re.compile(r"^(Strix|Bubo|Megascops|Tyto|Glaucidium|Ciccaba|Pulsatrix|Asio|Athene|Otus|Lophostrix|"
                       r"Pseudoscops|Aegolius|Nyctibius|Nyctidromus|Caprimulgus|Antrostomus|Chordeiles|"
                       r"Hydropsalis|Nyctiphrynus|Nycticorax|Nyctanassa)\b")
STATUS_WORDS = ["Regular here", "Heard recently", "Possible", "Fading", "Gone for the season", "No longer heard"]

_db = _lock = None  # the indexer's lock must be an RLock (scores call back into range lookups)
_in_range = _months = _photo_near = None


def init(db, lock, in_range, months=None, photo_near=None):
    """in_range(cell, latin) -> True/False/None; months(cell, latin) -> set of 1..12 or None
    (None = unknown, treated as in season); photo_near(cell, latin) -> bool."""
    global _db, _lock, _in_range, _months, _photo_near
    _db, _lock, _in_range = db, lock, in_range
    _months = months or (lambda c, l: None)
    _photo_near = photo_near or (lambda c, l: False)
    db.executescript("""
    CREATE TABLE IF NOT EXISTS occ_windows(wid TEXT PRIMARY KEY, account TEXT, listener TEXT, window INTEGER,
      cell7 TEXT, frames INTEGER, human INTEGER, silenced INTEGER, tz_min INTEGER, source TEXT, received INTEGER);
    CREATE INDEX IF NOT EXISTS ow_cell ON occ_windows(cell7, window);
    CREATE TABLE IF NOT EXISTS occ_events(listener TEXT, window INTEGER, cell TEXT, latin TEXT, events INTEGER,
      conf_q8 INTEGER, hour INTEGER, PRIMARY KEY(listener, window, cell, latin));
    CREATE INDEX IF NOT EXISTS oe_latin ON occ_events(latin);
    CREATE TABLE IF NOT EXISTS occ_state(cell TEXT, latin TEXT, season TEXT, cell7 TEXT, L REAL, L0 REAL,
      updated INTEGER, last_heard INTEGER, days TEXT, listeners TEXT, peak REAL, abs_day INTEGER, abs_today REAL,
      up_day INTEGER, up_today REAL, eff_since_heard REAL, tier TEXT, in_range INTEGER,
      PRIMARY KEY(cell, latin, season));
    CREATE INDEX IF NOT EXISTS os_cell7 ON occ_state(cell7);
    CREATE TABLE IF NOT EXISTS occ_changes(t INTEGER, kind TEXT, cell TEXT, latin TEXT, season TEXT, p REAL);
    CREATE TABLE IF NOT EXISTS occ_labels(account TEXT, latin TEXT, conf_q8 INTEGER, label INTEGER, t INTEGER,
      PRIMARY KEY(account, latin, t));
    CREATE TABLE IF NOT EXISTS occ_temp(latin TEXT PRIMARY KEY, T REAL, n INTEGER);
    """)
    db.commit()


# ── small maths ────────────────────────────────────────────────────────────────────────
def logit(p):
    p = min(max(p, 1e-6), 1 - 1e-6)
    return math.log(p / (1 - p))


def sigmoid(x):
    return 1 / (1 + math.exp(-x))


def season_of(t, lat):
    """Meteorological season, flipped in the southern hemisphere; tropics use the same labels."""
    m = time.gmtime(t).tm_mon
    s = ["DJF", "DJF", "MAM", "MAM", "MAM", "JJA", "JJA", "JJA", "SON", "SON", "SON", "DJF"][m - 1]
    if lat < -23.5:
        s = {"DJF": "JJA", "JJA": "DJF", "MAM": "SON", "SON": "MAM"}[s]
    y = time.gmtime(t).tm_year + (1 if m == 12 else 0)
    return f"{y}-{s}"


def cell_key(cell):
    """Detections are scored at res 9 at most; coarser cells (home area) stay as they are."""
    r = h3.get_resolution(cell)
    return h3.cell_to_parent(cell, 9) if r > 9 else cell


def cell7_of(cell):
    r = h3.get_resolution(cell)
    return h3.cell_to_parent(cell, 7) if r > 7 else cell


def default_detectability(latin, hour):
    if NOCTURNAL.match(latin):
        return 1.0 if hour >= 19 or hour < 5 else (0.3 if hour in (5, 18) else 0.02)
    if 5 <= hour < 9:
        return 1.0
    if 9 <= hour < 16:
        return 0.6
    if 16 <= hour < 19:
        return 0.8
    return 0.03


def detectability(latin, hour):
    """From this species' detection hours across all listeners (>=20 events), else a default."""
    with _lock:
        rows = _db.execute("SELECT hour, SUM(events) FROM occ_events WHERE latin=? AND hour IS NOT NULL GROUP BY hour",
                           (latin,)).fetchall()
    n = sum(r[1] for r in rows)
    if n < 20:
        return default_detectability(latin, hour)
    hist = [0.0] * 24
    for h, c in rows:
        hist[int(h) % 24] = c
    sm = [(hist[(i - 1) % 24] + 2 * hist[i] + hist[(i + 1) % 24]) / 4 for i in range(24)]
    return max(0.02, sm[hour % 24] / max(sm))


def temperature(latin):
    with _lock:
        r = _db.execute("SELECT T FROM occ_temp WHERE latin=?", (latin,)).fetchone() or \
            _db.execute("SELECT T FROM occ_temp WHERE latin='*'").fetchone()
    return r[0] if r else DEFAULT_T


def calibrated(latin, conf):
    return sigmoid(logit(conf) / temperature(latin))


def prior(cell, latin, t):
    inr = _in_range(cell, latin)
    lat = h3.cell_to_latlng(cell)[0]
    if inr is None:
        kind = "unknown"
    elif inr is False:
        kind = "out"
    else:
        months = _months(cell, latin)
        kind = "in_season" if months is None or time.gmtime(t).tm_mon in months else "off_season"
    return logit(PRIOR_P[kind]), inr, kind, lat


# ── state ──────────────────────────────────────────────────────────────────────────────
def _load(cell, latin, t):
    L0, inr, kind, lat = prior(cell, latin, t)
    season = season_of(t, lat)
    r = _db.execute("SELECT * FROM occ_state WHERE cell=? AND latin=? AND season=?", (cell, latin, season)).fetchone()
    if r:
        cols = [c[0] for c in _db.execute("SELECT * FROM occ_state LIMIT 0").description]
        s = dict(zip(cols, r))
        s["days"] = json.loads(s["days"]); s["listeners"] = json.loads(s["listeners"])
        s["L0"] = L0; s["in_range"] = None if inr is None else int(inr); s["_kind"] = kind
        return s
    # New season: carry the last season's evidence over, halfway back to the prior.
    old = _db.execute("SELECT L, L0, days, listeners, peak, tier FROM occ_state WHERE cell=? AND latin=? ORDER BY updated DESC LIMIT 1",
                      (cell, latin)).fetchone()
    L = L0 + (old[0] - old[1]) * 0.5 if old else L0
    return {"cell": cell, "latin": latin, "season": season, "cell7": cell7_of(cell), "L": L, "L0": L0,
            "updated": t, "last_heard": None, "days": json.loads(old[2]) if old else [],
            "listeners": json.loads(old[3]) if old else {}, "peak": old[4] if old else sigmoid(L), "abs_day": 0,
            "abs_today": 0.0, "up_day": 0, "up_today": 0.0, "eff_since_heard": 0.0, "tier": old[5] if old else None,
            "in_range": None if inr is None else int(inr), "_kind": kind}


def _drift(s, t):
    dt = t - s["updated"]
    if dt <= 0:
        return
    resident = len(s["days"]) >= 2 or len(s["listeners"]) >= 2
    tau = (TAU_RESIDENT_D if resident else TAU_ONE_OFF_D) * DAY_S
    s["L"] = s["L0"] + (s["L"] - s["L0"]) * math.exp(-dt / tau)
    s["updated"] = t


def _save(s, t):
    p = sigmoid(s["L"])
    s["peak"] = max(s.get("peak") or 0.0, p)
    new_tier = tier_eff(s)
    old = s.get("tier")
    if new_tier and not old:
        _db.execute("INSERT INTO occ_changes VALUES(?,?,?,?,?,?)", (t, "arrival", s["cell"], s["latin"], s["season"], p))
    elif old and not new_tier and s["eff_since_heard"] >= DEPARTURE_WINDOWS:
        _db.execute("INSERT INTO occ_changes VALUES(?,?,?,?,?,?)", (t, "departure", s["cell"], s["latin"], s["season"], p))
    s["tier"] = new_tier
    _db.execute("INSERT OR REPLACE INTO occ_state VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)", (
        s["cell"], s["latin"], s["season"], s["cell7"], s["L"], s["L0"], s["updated"], s["last_heard"],
        json.dumps(s["days"][-60:]), json.dumps(s["listeners"]), s["peak"], s["abs_day"], s["abs_today"],
        s["up_day"], s["up_today"], s["eff_since_heard"], s["tier"], s["in_range"]))


def tier_eff(s):
    """Map tier with hysteresis: a likely species keeps its tier until enough listening without
    hearing it (a departure) or until p falls below 0.5, so a quiet night is not a departure."""
    t = tier_of(s)
    if t or not s.get("tier"):
        return t
    if sigmoid(s["L"]) >= 0.5 and s["eff_since_heard"] < DEPARTURE_WINDOWS:
        return s["tier"]
    return None


def tier_of(s):
    p = sigmoid(s["L"])
    indep = len(s["listeners"])
    if s.get("in_range") == 0:
        if p >= P_RARE and (indep >= 2 or _photo_near(s["cell"], s["latin"])):
            return "rare"
        return None
    if p >= P_CONFIRMED or (p >= P_LIKELY and indep >= 2):
        return "confirmed"
    if p >= P_LIKELY:
        return "likely"
    return None


def status_of(s, t):
    """Plain words for people."""
    p = sigmoid(s["L"])
    heard_ago = (t - s["last_heard"]) / DAY_S if s["last_heard"] else None
    recent_days = [d for d in s["days"] if d >= int(t // DAY_S) - 14]
    if heard_ago is not None and heard_ago <= 3:
        return "Regular here" if p >= P_LIKELY and len(recent_days) >= 3 else "Heard recently"
    if (s.get("peak") or 0) >= P_LIKELY:
        if s.get("_kind") == "off_season":
            return "Gone for the season"
        if p >= P_LIKELY:
            return "Regular here"
        if p >= 0.5:
            return "Fading"
        return "No longer heard"
    return "Possible"


# ── evidence ───────────────────────────────────────────────────────────────────────────
def _hear(s, listener, events, conf, t, ring_listeners):
    day = int(t // DAY_S)
    if s["up_day"] != day:
        s["up_day"], s["up_today"] = day, 0.0
    c = calibrated(s["latin"], conf)
    per = max(0.0, EVENT_BASE + EVENT_GAIN * logit(c))
    up = min(WINDOW_UP_CAP, per * (1 + REPEAT_WEIGHT * max(0, events - 1)))
    recent = {k for k, d in ring_listeners.items() if d >= day - 7}
    if listener not in recent and recent:
        up *= NEW_LISTENER_X             # an independent listener corroborates
    if day not in s["days"]:
        up += NEW_DAY_BONUS
        s["days"].append(day)
    up = max(0.0, min(up, DAY_UP_CAP - s["up_today"]))
    s["up_today"] += up
    s["L"] = min(L_MAX, s["L"] + up)
    s["listeners"][listener] = day
    s["last_heard"] = t
    s["eff_since_heard"] = 0.0


def _miss(s, hour, effort, t):
    if s.get("_kind") in ("off_season",):
        return                           # migrants out of season are not penalised
    day = int(t // DAY_S)
    if s["abs_day"] != day:
        s["abs_day"], s["abs_today"] = day, 0.0
    d = detectability(s["latin"], hour)
    down = ABSENCE_PER_WINDOW * d * effort
    down = max(0.0, min(down, ABSENCE_DAY_CAP - s["abs_today"]))
    s["abs_today"] += down
    if down > 0:
        s["L"] = max(s["L"] - down, min(s["L"], s["L0"] - ABSENCE_FLOOR), L_MIN)
    s["eff_since_heard"] += effort * d


def _ring_listeners(cell, latin):
    cells = list(h3.grid_disk(cell, 1)) if h3.get_resolution(cell) >= 7 else [cell]
    out = {}
    q = "SELECT listeners FROM occ_state WHERE latin=? AND cell IN (%s)" % ",".join("?" * len(cells))
    for (ls,) in _db.execute(q, [latin] + cells).fetchall():
        for k, d in json.loads(ls).items():
            out[k] = max(out.get(k, 0), d)
    return out


def ingest_window(p, account, verified, now=None):
    """p = {"v":1,"sid","window","cell"(res<=7),"frames","human","silenced","tz_min","source","station"?,
    "species":[{"name","cell","events","conf_q8"}]}. Only verified rows change scores."""
    now = now or time.time()
    w = int(p["window"])
    cell7 = p["cell"]
    if not h3.is_valid_cell(cell7) or h3.get_resolution(cell7) > 7:
        raise ValueError("effort cell must be res 7 or coarser")
    station = str(p.get("station") or "")
    listener = f"station:{station}" if re.fullmatch(r"[0-9a-f]{16}", station) else f"acct:{account}"
    frames, human, silenced = int(p.get("frames", 0)), int(p.get("human", 0)), int(p.get("silenced", 0))
    tz = int(p.get("tz_min", 0))
    t = (w + 1) * WINDOW_S
    if t > now + WINDOW_S:
        raise ValueError("window in the future")
    hour = int(((t + tz * 60) % DAY_S) // 3600)
    wid = f"{listener}|{w}|{str(p.get('sid', ''))[:40]}"
    heard = []
    for sp in (p.get("species") or [])[:40]:
        name, c = str(sp.get("name", "")), str(sp.get("cell", cell7))
        if not re.fullmatch(r"[A-Z][a-z]+ [a-z][a-z-]+", name) or not h3.is_valid_cell(c):
            continue
        ck = cell_key(c)
        if cell7_of(ck) != cell7 and h3.get_resolution(ck) >= 7 and h3.cell_to_parent(ck, 7) != cell7:
            pass  # a detection just over the res-7 edge is fine; scored in its own cell
        heard.append((ck, name, max(1, min(int(sp.get("events", 1)), 50)), max(0, min(int(sp.get("conf_q8", 0)), 255)) / 255))
    with _lock:
        if _db.execute("SELECT 1 FROM occ_windows WHERE wid=?", (wid,)).fetchone():
            return {"ok": True, "duplicate": True}
        _db.execute("INSERT INTO occ_windows VALUES(?,?,?,?,?,?,?,?,?,?,?)",
                    (wid, account, listener, w, cell7, frames, human, silenced, tz, str(p.get("source", "phone"))[:8], int(now)))
        for ck, name, ev, conf in heard:
            _db.execute("INSERT OR REPLACE INTO occ_events VALUES(?,?,?,?,?,?,?)", (listener, w, ck, name, ev, int(conf * 255), hour))
        if verified == 1:
            _apply(cell7, heard, listener, frames, hour, t)
        _db.commit()
    return {"ok": True, "heard": len(heard), "listened": frames > 0}


def _apply(cell7, heard, listener, frames, hour, t):
    heard_keys = set()
    for ck, name, ev, conf in heard:
        s = _load(ck, name, t)
        _drift(s, t)
        _hear(s, listener, ev, conf, t, _ring_listeners(ck, name))
        _save(s, t)
        heard_keys.add((ck, name))
    # Absence: only real listening. Silenced and speech-dropped frames are not in `frames`.
    effort = min(1.0, frames / FULL_EFFORT_FRAMES)
    if effort <= 0:
        return
    heard_names = {n for _, n in heard_keys}
    rows = _db.execute("SELECT DISTINCT cell, latin FROM occ_state WHERE cell7=? OR cell=?", (cell7, cell7)).fetchall()
    for cell, latin in rows:
        if latin in heard_names:
            continue
        s = _load(cell, latin, t)
        _drift(s, t)
        _miss(s, hour, effort, t)
        _save(s, t)


def ingest_sighting(cell, latin, conf_q8, frames, window, listener, verified, now=None):
    """Older phones and camera stations send sightings, not windows: positive evidence only."""
    if verified not in (1, 3, 4):
        return
    t = (int(window) + 1) * WINDOW_S
    ck = cell_key(cell)
    with _lock:
        s = _load(ck, latin, t)
        _drift(s, t)
        _hear(s, listener, max(1, min(int(frames), 3)), int(conf_q8) / 255, t, _ring_listeners(ck, latin))
        _save(s, t)
        _db.execute("INSERT OR IGNORE INTO occ_events VALUES(?,?,?,?,?,?,?)",
                    (listener, int(window), ck, latin, max(1, min(int(frames), 3)), int(conf_q8), None))
        _db.commit()


# ── labels + calibration ───────────────────────────────────────────────────────────────
def add_labels(account, labels, now=None):
    rows = []
    for x in (labels or [])[:200]:
        name = str(x.get("latin", ""))
        if not re.fullmatch(r"[A-Z][a-z]+ [a-z][a-z-]+", name) or not isinstance(x.get("label"), bool):
            continue
        rows.append((account, name, max(0, min(int(x.get("conf_q8", 0)), 255)), 1 if x["label"] else 0, int(x.get("t", 0))))
    with _lock:
        _db.executemany("INSERT OR REPLACE INTO occ_labels VALUES(?,?,?,?,?)", rows)
        _db.commit()
    refit()
    return {"ok": True, "stored": len(rows)}


def _fit(pairs):
    best, bt = None, DEFAULT_T
    for i in range(5, 81):
        T = i / 20
        nll = 0.0
        for c, y in pairs:
            q = sigmoid(logit(c) / T)
            nll -= math.log(q if y else 1 - q)
        if best is None or nll < best:
            best, bt = nll, T
    return bt


def refit():
    with _lock:
        rows = _db.execute("SELECT latin, conf_q8, label FROM occ_labels").fetchall()
    by = {}
    for latin, c, y in rows:
        by.setdefault(latin, []).append((max(1, min(c, 254)) / 255, y))
    out = []
    allp = [x for v in by.values() for x in v]
    if len(allp) >= 30:
        out.append(("*", _fit(allp), len(allp)))
    for latin, v in by.items():
        if len(v) >= 10:
            out.append((latin, _fit(v), len(v)))
    with _lock:
        _db.executemany("INSERT OR REPLACE INTO occ_temp VALUES(?,?,?)", out)
        _db.commit()


# ── reading ────────────────────────────────────────────────────────────────────────────
def _view(s, t):
    _drift(s, t)
    return {"latin": s["latin"], "p": round(sigmoid(s["L"]), 4), "status": status_of(s, t), "tier": tier_eff(s),
            "listeners": len(s["listeners"]), "days_heard": len(s["days"]),
            "last_heard_days": None if not s["last_heard"] else round((t - s["last_heard"]) / DAY_S, 1)}


def area_view(cell, now=None):
    """Local view for a res-7 (or coarser) cell: one row per species, best cell; exact cells
    only for map tiers. Weak entries are listed (status words) without their res-9 cell."""
    t = now or time.time()
    c7 = cell if h3.get_resolution(cell) <= 7 else h3.cell_to_parent(cell, 7)
    with _lock:
        rows = _db.execute("SELECT cell, latin FROM occ_state WHERE cell7=? OR cell=?", (c7, c7)).fetchall()
        best = {}
        for cell_, latin in rows:
            s = _load(cell_, latin, t)
            v = _view(s, t)
            if v["tier"]:
                v["cell"] = cell_
            if latin not in best or v["p"] > best[latin]["p"]:
                best[latin] = v
    order = {w: i for i, w in enumerate(STATUS_WORDS)}
    return sorted(best.values(), key=lambda v: (order[v["status"]], -v["p"]))


def map_cells(res=7, now=None):
    t = now or time.time()
    with _lock:
        rows = _db.execute("SELECT cell, latin FROM occ_state WHERE tier IS NOT NULL").fetchall()
        agg = {}
        for cell, latin in rows:
            s = _load(cell, latin, t)
            tr = tier_eff(s)
            if not tr:
                continue
            c = h3.cell_to_parent(cell, res) if h3.get_resolution(cell) > res else cell
            e = agg.setdefault(c, {"cell": c, "species": 0, "confirmed": 0, "rare": 0})
            e["species"] += 1
            e["confirmed"] += tr == "confirmed"
            e["rare"] += tr == "rare"
    return list(agg.values())


def changes(since=0, limit=500):
    with _lock:
        rows = _db.execute("SELECT t, kind, cell, latin, season, p FROM occ_changes WHERE t>=? ORDER BY t LIMIT ?",
                           (int(since), min(int(limit), 2000))).fetchall()
    return [{"t": r[0], "kind": r[1], "cell": r[2], "latin": r[3], "season": r[4], "p": round(r[5], 4),
             "chain": "not submitted (no chain format yet)"} for r in rows]
