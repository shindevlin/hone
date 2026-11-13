# One-off: hold back already-published sightings of species not recorded near them (GBIF range).
import json, sqlite3, h3, time
d = sqlite3.connect('/var/lib/hone-birds/birds.db')
for _ in range(120):
    if d.execute("SELECT count(*) FROM ranges").fetchone()[0]: break
    time.sleep(10)
ranges = {c: set(json.loads(s)) for c, s in d.execute("SELECT cell, species FROM ranges")}
n = 0
for sid, cell, latin in d.execute("SELECT sid, cell, latin FROM sightings WHERE verified=1").fetchall():
    c3 = h3.cell_to_parent(cell, 3) if h3.get_resolution(cell) > 3 else cell
    if c3 in ranges and latin not in ranges[c3]:
        q8 = d.execute("SELECT conf_q8 FROM sightings WHERE sid=?", (sid,)).fetchone()[0]; d.execute("UPDATE sightings SET verified=? WHERE sid=?", (3 if q8 >= 217 else 2, sid)); n += 1
d.commit(); print("held back", n)
