"""One-off (Nov 2026): rebuild GBIF range priors (100/250 km tiers + monthly seasons) for every
area with reports, then re-check stored sightings and re-score occ_state against them.
Run from /opt/hone-birds with its venv. Prints counts only."""
import json, sqlite3, sys, threading, time, urllib.request
from concurrent.futures import ThreadPoolExecutor
import h3
sys.path.insert(0, "/opt/hone-birds")
import hone_occupancy as occ

DB = "/var/lib/hone-birds/birds.db"
RANGE_KM, NEAR_KM, IN_NEAR, IN_WIDE, MONTHS_MIN, RARE_MIN_Q8 = 250, 100, 3, 20, 20, 217
GROUPS = {"classKey": [212, 131, 359], "orderKey": [1458]}
db = sqlite3.connect(DB, timeout=60, check_same_thread=False)
lock = threading.RLock()


def gbif(url):
    for i in range(4):
        try:
            req = urllib.request.Request(url, headers={"User-Agent": "hone-birds/1 (https://honemesh.net)"})
            return json.load(urllib.request.urlopen(req, timeout=60))
        except Exception:
            if i == 3:
                raise
            time.sleep(5 * (i + 1))


def name_of(k):
    with lock:
        r = db.execute("SELECT name FROM gbif_names WHERE key=?", (k,)).fetchone()
    if r:
        return r[0]
    n = gbif(f"https://api.gbif.org/v1/species/{k}").get("canonicalName")
    if n:
        with lock:
            db.execute("INSERT OR REPLACE INTO gbif_names VALUES(?,?)", (k, n)); db.commit()
    return n


def area(cell):
    return h3.latlng_to_cell(*h3.cell_to_latlng(cell), 3)


def build(c3):
    lat, lng = h3.cell_to_latlng(c3)
    near, wide, monthly = {}, {}, {}

    def facet(url, into):
        d = gbif(url)
        for c in d["facets"][0]["counts"] if d.get("facets") else []:
            into[int(c["name"])] = into.get(int(c["name"]), 0) + c["count"]
    for field, vals in GROUPS.items():
        for v in vals:
            base = f"https://api.gbif.org/v1/occurrence/search?{field}={v}&facet=speciesKey&facetLimit=5000&limit=0"
            facet(base + f"&geo_distance={lat:.4f},{lng:.4f},{NEAR_KM}km", near)
            facet(base + f"&geo_distance={lat:.4f},{lng:.4f},{RANGE_KM}km", wide)
            for m in range(1, 13):
                facet(base + f"&geo_distance={lat:.4f},{lng:.4f},{RANGE_KM}km&month={m}", monthly.setdefault(m, {}))
    keys = {k for k in wide if near.get(k, 0) >= IN_NEAR or wide[k] >= IN_WIDE}
    with ThreadPoolExecutor(8) as ex:
        named = {k: n for k, n in zip(keys, ex.map(name_of, keys)) if n}
    rows = []
    for k, n in named.items():
        counts = {m: monthly[m].get(k, 0) for m in range(1, 13)}
        months = None
        if sum(counts.values()) >= MONTHS_MIN:
            top = max(counts.values())
            months = sorted(m for m, c in counts.items() if c >= 0.1 * top)
        rows.append((c3, n, json.dumps(months), int(time.time())))
    with lock:
        db.execute("INSERT OR REPLACE INTO ranges VALUES(?,?,?)", (c3, json.dumps(sorted(set(named.values()))), int(time.time())))
        db.execute("DELETE FROM gbif_months WHERE cell=?", (c3,))
        db.executemany("INSERT OR REPLACE INTO gbif_months VALUES(?,?,?,?)", rows)
        db.commit()
    print("area", c3, "in range", len(named), "of", len(wide), "seasons", sum(1 for r in rows if r[2] != "null"), flush=True)


cells = {r[0] for r in db.execute("SELECT DISTINCT cell FROM sightings UNION SELECT DISTINCT cell FROM occ_state "
                                   "UNION SELECT DISTINCT cell FROM occ_events")}
areas = sorted({area(c) for c in cells})
for c3 in areas:
    build(c3)
R = {c: set(json.loads(s)) for c, s in db.execute("SELECT cell, species FROM ranges")}
M = {(c, l): (set(json.loads(m)) if json.loads(m) else None) for c, l, m in db.execute("SELECT cell, latin, months FROM gbif_months")}


def in_range(cell, latin):
    c3 = area(cell)
    return None if c3 not in R else latin in R[c3]


def months(cell, latin):
    return M.get((area(cell), latin))


def photo_near(cell, latin):
    c7 = h3.cell_to_parent(cell, 7) if h3.get_resolution(cell) > 7 else cell
    return db.execute("SELECT 1 FROM photos p WHERE p.latin=? AND p.verified=1 AND p.hidden=0 AND EXISTS("
                      "SELECT 1 FROM occ_windows w WHERE w.account=p.account AND w.cell7=?) LIMIT 1", (latin, c7)).fetchone() is not None


# Sightings: out of range -> held (3 if very confident, else 2); 3s now in range -> 1. Confirmed (4) stay.
moved = {}
for sid, cell, latin, q8, v in db.execute("SELECT sid, cell, latin, conf_q8, verified FROM sightings WHERE verified IN (1,3)").fetchall():
    inr = in_range(cell, latin)
    nv = v
    if inr is False:
        nv = 3 if q8 >= RARE_MIN_Q8 else 2
    elif inr and v == 3:
        nv = 1
    if nv != v:
        db.execute("UPDATE sightings SET verified=? WHERE sid=?", (nv, sid))
        moved[(v, nv)] = moved.get((v, nv), 0) + 1
db.commit()
print("sightings re-checked:", {f"{a}->{b}": n for (a, b), n in moved.items()}, flush=True)

# occ_state: new prior, same evidence (L - L0 kept), tier recomputed.
occ.init(db, lock, in_range, months=months, photo_near=photo_near)
cols = [c[0] for c in db.execute("SELECT * FROM occ_state LIMIT 0").description]
changed = {"in_range": 0, "tier": 0}
for r in db.execute("SELECT * FROM occ_state").fetchall():
    s = dict(zip(cols, r))
    s["days"] = json.loads(s["days"]); s["listeners"] = json.loads(s["listeners"])
    L0, inr, kind, _ = occ.prior(s["cell"], s["latin"], s["updated"])
    new_inr = None if inr is None else int(inr)
    s["L"] = L0 + (s["L"] - s["L0"]); s["L0"] = L0
    if new_inr != s["in_range"]:
        changed["in_range"] += 1
    s["in_range"] = new_inr
    t = occ.tier_eff(s)
    if t != s["tier"]:
        changed["tier"] += 1
    db.execute("UPDATE occ_state SET L=?, L0=?, in_range=?, tier=? WHERE cell=? AND latin=? AND season=?",
               (s["L"], L0, new_inr, t, s["cell"], s["latin"], s["season"]))
db.commit()
print("occ_state rows", len(cols) and db.execute("SELECT count(*) FROM occ_state").fetchone()[0], "changed", changed,
      "null in_range", db.execute("SELECT count(*) FROM occ_state WHERE in_range IS NULL").fetchone()[0], flush=True)
print("out of range now:", sorted({r[0] for r in db.execute("SELECT latin FROM occ_state WHERE in_range=0")}), flush=True)
print("DONE")
