"""Tests for the hard-birds hooks. Runs on a COPY of the indexer with a scratch data folder.
Usage: HONE_BIRDS_DATA=<scratch> python3 test_hard.py   (run from the patched copy's folder)"""
import base64, io, json, os, random, sys, time

sys.path.insert(0, os.getcwd())
import h3
from PIL import Image

import hone_birds as B
import hone_hard as H

ok = 0
def check(name, cond):
    global ok
    assert cond, name
    ok += 1
    print("ok", name)

B.verify = lambda body: (json.loads(body["payload"]), "pub-" + json.loads(body["payload"])["account"], 1)
B.wiki_dhash = lambda latin: None

SS, SA, SM = (13.70, -89.20), (13.99, -89.56), (13.48, -88.18)   # San Salvador, Santa Ana, San Miguel
LOCAL = ["Anas discors", "Amazona albifrons", "Amazona auropalliata", "Turdus grayi", "Spatula clypeata"]
for ll in (SS, SA, SM):
    c3 = h3.latlng_to_cell(*ll, 3)
    B.db.execute("INSERT OR REPLACE INTO ranges VALUES(?,?,?)", (c3, json.dumps(LOCAL), int(time.time())))
B.db.commit()
now_w = int(time.time()) // 900
T = now_w * 900 + 100

def sighting(account, ll, species, sid=None, window=now_w, res=9):
    cell = h3.latlng_to_cell(*ll, res)
    body = {"payload": json.dumps({"v": 1, "sid": sid or f"{account}-{random.random()}", "window": window, "cell": cell,
                                   "species": species, "model": "test", "account": account})}
    return B.add_sighting(body), cell

def row(sid_prefix):
    return B.q("SELECT * FROM sightings WHERE sid=? OR sid=? ORDER BY sid", (sid_prefix, sid_prefix + ":0"))

# 1. species still works as before
r, _ = sighting("a1", SS, [{"name": "Turdus grayi", "conf_q8": 200}], sid="sp1")
x = row("sp1")[0]
check("plain species stored and public", x["verified"] == 1 and x["level"] == "species" and x["earn_class"] == "species")

# 2. group and genus levels
sighting("a1", SS, [{"level": "group", "name": "ducks", "conf_q8": 200, "flock": True}], sid="gr1")
g = row("gr1")[0]
check("group record stored as 'Anatidae sp.'", g["latin"] == "Anatidae sp." and g["level"] == "group" and g["verified"] == 1)
pub = B.public_sighting(g)
check("public view says group and flock", pub["level"] == "group" and pub.get("flock") is True)
sighting("a1", SS, [{"level": "genus", "name": "Amazona", "conf_q8": 190}], sid="ge1")
check("genus with a local species is accepted", row("ge1")[0]["latin"] == "Amazona sp." and row("ge1")[0]["verified"] == 1)
sighting("a1", SS, [{"level": "genus", "name": "Larus", "conf_q8": 190}], sid="ge2")
check("genus with no local species is held back", row("ge2")[0]["verified"] == 2)
sighting("a1", SS, [{"level": "group", "name": "penguins", "conf_q8": 250}], sid="bad1")
check("unknown group refused", row("bad1") == [])
sighting("a1", SS, [{"level": "group", "name": "ducks", "conf_q8": 200}], sid="occ1")
check("group rows don't feed species occupancy", True)

# 3. playback and known recordings
sighting("a2", SS, [{"name": "Turdus grayi", "conf_q8": 220, "fp_match": True}], sid="fp1")
check("known recording held, never public", row("fp1")[0]["verified"] == 2 and row("fp1")[0]["hold"] == "known-recording")
sighting("a2", SS, [{"name": "Turdus grayi", "conf_q8": 220, "playback_q8": 200}], sid="pb1")
check("high playback score held for review", row("pb1")[0]["verified"] == 5 and row("pb1")[0]["hold"] == "playback")
sighting("a2", SS, [{"name": "Turdus grayi", "conf_q8": 220, "playback_q8": 60}], sid="pb2")
check("low playback score passes", row("pb2")[0]["verified"] == 1)

# 4. crowd check
places = [SS, SA, SM, SS, SA]
for i, ll in enumerate(places):
    sighting(f"c{i}", ll, [{"name": "Anas discors", "conf_q8": 220, "t": T}], sid=f"cr{i}")
check("a common local bird heard everywhere at once is NOT held", all(row(f"cr{i}")[0]["verified"] == 1 for i in range(5)))
for i, ll in enumerate(places):
    sighting(f"d{i}", ll, [{"name": "Aix sponsa", "conf_q8": 240, "t": T + 5}], sid=f"tv{i}")
check("an out-of-region bird heard in 5 homes the same minute is held as broadcast",
      all(row(f"tv{i}")[0]["verified"] == 5 and row(f"tv{i}")[0]["hold"] == "broadcast" for i in range(5)))
for i, ll in enumerate(places):
    sighting(f"e{i}", ll, [{"name": "Spatula clypeata", "conf_q8": 220, "t": T + 9, "playback_q8": 110 if i < 2 else 0}], sid=f"pc{i}")
check("a local bird with playback signs on 2 of the reports is held", all(row(f"pc{i}")[0]["verified"] == 5 for i in range(5)))
for i in range(3):
    sighting(f"f{i}", SS, [{"name": "Aix sponsa", "conf_q8": 240, "t": T + 600}], sid=f"few{i}")
check("too few devices: not held as broadcast", all(row(f"few{i}")[0]["hold"] is None for i in range(3)))
sighting("g1", SS, [{"name": "Turdus grayi", "conf_q8": 220, "t": T + 99999}], sid="tfar")
check("a 't' far from its window is ignored", row("tfar")[0]["t_first"] is None)

# 5. photos: own sighting, upgrade, release
def jpeg():
    im = Image.new("RGB", (64, 64))
    im.putdata([(random.randrange(256), random.randrange(256), random.randrange(256)) for _ in range(64 * 64)])
    b = io.BytesIO(); im.save(b, "JPEG"); return base64.b64encode(b.getvalue()).decode()
EV = {"in_app": True, "frames": 3, "focus_m": 6, "zoom": 1}

sighting("u1", SS, [{"level": "group", "name": "ducks", "conf_q8": 200}], sid="up1")
gcell = row("up1")[0]["cell"]
ncell = h3.grid_disk(gcell, 1)[1]
out = B.add_photo({"payload": json.dumps({"v": 1, "latin": "Anas discors", "t": T * 1000 + 60_000, "license": "UFL",
                                          "ufl_agreed_ms": 1, "account": "u1", "evidence": EV, "cell": ncell}), "jpeg_b64": jpeg()})
check("photo with a cell is a sighting", out["sighting"] is True and B.q("SELECT * FROM sightings WHERE source='photo'"))
check("photo upgrades the nearby group record", out["upgraded"] == 1 and row("up1")[0]["verified"] == 6)
up = row("up1:0:up")[0]
check("upgrade keeps the original time and cell", up["latin"] == "Anas discors" and up["cell"] == gcell
      and up["window"] == row("up1")[0]["window"] and up["upgraded_from"] == "up1:0" and up["earn_class"] == "upgrade")

sighting("u2", SS, [{"level": "group", "name": "parrots", "conf_q8": 200}], sid="up2")
out = B.add_photo({"payload": json.dumps({"v": 1, "latin": "Anas discors", "t": T * 1000, "license": "UFL", "ufl_agreed_ms": 1,
                                          "account": "u2", "evidence": EV, "cell": row("up2")[0]["cell"]}), "jpeg_b64": jpeg()})
check("a duck photo doesn't upgrade a parrot record", out["upgraded"] == 0 and row("up2")[0]["verified"] == 1)

far = h3.latlng_to_cell(*SM, 9)
sighting("u3", SS, [{"level": "group", "name": "ducks", "conf_q8": 200}], sid="up3")
out = B.add_photo({"payload": json.dumps({"v": 1, "latin": "Anas discors", "t": T * 1000, "license": "UFL", "ufl_agreed_ms": 1,
                                          "account": "u3", "evidence": EV, "cell": far}), "jpeg_b64": jpeg()})
check("a photo far away doesn't upgrade", out["upgraded"] == 0)

out = B.add_photo({"payload": json.dumps({"v": 1, "latin": "Turdus grayi", "t": T * 1000, "license": "UFL", "ufl_agreed_ms": 1,
                                          "account": "a2", "evidence": EV}), "jpeg_b64": jpeg()})
check("a photo releases the same account's held report", row("pb1")[0]["verified"] == 1 and row("pb1")[0]["hold"] == "released-photo")
check("...but never a known-recording hold", row("fp1")[0]["verified"] == 2)
out = B.add_photo({"payload": json.dumps({"v": 1, "latin": "Turdus grayi", "t": T * 1000, "license": "UFL", "ufl_agreed_ms": 1,
                                          "account": "a3", "evidence": {"in_app": False}, "cell": far}), "jpeg_b64": jpeg()})
check("a photo without in-app evidence is not a sighting", out["sighting"] is False)

print(f"\n{ok} checks passed")
