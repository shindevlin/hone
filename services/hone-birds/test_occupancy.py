"""Occupancy score tests (in-memory, no network). Run: python test_occupancy.py"""
import os, sqlite3, sys, threading

import h3

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import hone_occupancy as occ  # noqa: E402

KISKADEE, CARDINAL = "Pitangus sulphuratus", "Cardinalis cardinalis"
LAT, LON = 13.68, -89.24                     # Antiguo Cuscatlan
C9 = h3.latlng_to_cell(LAT, LON, 9)
C7 = h3.cell_to_parent(C9, 7)
OTHER9 = h3.grid_disk(C9, 1)[1] if h3.grid_disk(C9, 1)[0] == C9 else h3.grid_disk(C9, 1)[0]
DAY0 = 1793750400 // 86400 * 86400          # a UTC midnight in Nov 2026
TZ = -360                                     # UTC-6


def fresh():
    db = sqlite3.connect(":memory:", check_same_thread=False)
    occ.init(db, threading.RLock(), lambda c, l: l != CARDINAL, months=lambda c, l: None)


def win_at(day, local_hour, quarter=0):
    t = DAY0 + day * 86400 + (local_hour * 3600 - TZ * 60) + quarter * 900
    return t // 900


def send(w, frames=60, species=(), listener="josh", silenced=0, cell=C9):
    p = {"v": 1, "sid": f"{listener}-{w}", "window": w, "cell": C7, "frames": frames, "human": 0,
         "silenced": silenced, "tz_min": TZ, "source": "phone",
         "species": [{"name": n, "cell": cell, "events": e, "conf_q8": int(c * 255)} for n, e, c in species]}
    return occ.ingest_window(p, listener, 1, now=(w + 2) * 900)


def view(name, t):
    return next((v for v in occ.area_view(C7, now=t) if v["latin"] == name), None)


def day_of_listening(day, heard=(), listener="josh"):
    """Listens 05:00-19:00 local, every quarter hour; `heard` species in the 06:00-07:00 windows."""
    for h in range(5, 19):
        for q in range(4):
            sp = heard if h == 6 else ()
            send(win_at(day, h, q), species=sp, listener=listener)


def test_resident_stays_regular_through_quiet_day():
    fresh()
    for d in range(10):
        day_of_listening(d, heard=[(KISKADEE, 2, 0.80)])
    end9 = (win_at(9, 19) + 1) * 900
    v = view(KISKADEE, end9)
    assert v["status"] == "Regular here" and v["p"] >= 0.99, v
    day_of_listening(10)                         # a whole quiet day
    v = view(KISKADEE, (win_at(10, 19) + 1) * 900)
    assert v["status"] == "Regular here" and v["p"] >= 0.98, v
    day_of_listening(11); day_of_listening(12)   # three quiet days: still regular, a little lower
    v12 = view(KISKADEE, (win_at(12, 19) + 1) * 900)
    assert v12["p"] >= 0.95 and v12["status"] == "Regular here", v12
    assert v12["tier"] in ("likely", "confirmed"), v12
    print("resident:", v["p"], "->", v12["p"], v12["status"])


def test_one_off_out_of_range_fades():
    fresh()
    send(win_at(0, 7), species=[(CARDINAL, 1, 0.55)])
    v0 = view(CARDINAL, (win_at(0, 7) + 2) * 900)
    assert v0["status"] == "Heard recently" and v0["tier"] is None and v0["p"] < 0.05, v0
    for d in range(1, 5):
        day_of_listening(d)
    v4 = view(CARDINAL, (win_at(4, 19) + 1) * 900)
    assert v4["status"] == "Possible" and v4["tier"] is None, v4
    assert v4["p"] < v0["p"] / 2 and v4["p"] < 0.01, (v0, v4)
    print("cardinal one-off:", v0["p"], "->", v4["p"], v4["status"])


def test_out_of_range_repeated_and_corroborated_rises():
    fresh()
    for d in range(6):
        day_of_listening(d, heard=[(CARDINAL, 3, 0.75)], listener="josh")
        day_of_listening(d, heard=[(CARDINAL, 3, 0.75)], listener="shindevlin")
    v = view(CARDINAL, (win_at(5, 19) + 1) * 900)
    assert v["p"] >= 0.99 and v["tier"] == "rare" and v["listeners"] == 2, v
    print("cardinal repeated by two listeners:", v["p"], v["tier"])


def test_single_listener_rare_needs_second_listener():
    fresh()
    for d in range(6):
        day_of_listening(d, heard=[(CARDINAL, 3, 0.9)])
    v = view(CARDINAL, (win_at(5, 19) + 1) * 900)
    assert v["tier"] is None, v                  # p may be high, but no second listener or photo


def test_silenced_hours_never_count_as_absence():
    fresh()
    for d in range(3):
        day_of_listening(d, heard=[(KISKADEE, 2, 0.8)])
    a = view(KISKADEE, (win_at(3, 19) + 1) * 900)["p"]
    for h in range(5, 19):
        for q in range(4):
            send(win_at(3, h, q), frames=0, silenced=180)    # Android silenced the mic all day
    b = view(KISKADEE, (win_at(3, 19) + 1) * 900)["p"]
    assert abs(a - b) < 1e-9, (a, b)
    fresh()
    for d in range(3):
        day_of_listening(d, heard=[(KISKADEE, 2, 0.8)])
    day_of_listening(3)                                     # real listening does lower it
    c = view(KISKADEE, (win_at(3, 19) + 1) * 900)["p"]
    assert c < a, (a, c)
    print("silenced day:", a, "==", b, "; listened quiet day:", c)


def test_owl_not_penalised_at_noon():
    fresh()
    owl = "Megascops cooperi"
    send(win_at(0, 22), species=[(owl, 2, 0.8)])
    a = view(owl, (win_at(0, 22) + 2) * 900)["p"]
    for h in range(10, 15):
        for q in range(4):
            send(win_at(1, h, q))
    b = view(owl, (win_at(1, 15) + 1) * 900)["p"]
    send(win_at(0, 22) + 0, species=[])                      # duplicate window id is ignored
    assert b > a * 0.6, (a, b)                               # only drift, almost no absence


def test_calibration_fits_temperature():
    fresh()
    labels = [{"latin": KISKADEE, "conf_q8": int(255 * c), "label": y, "t": i}
              for i, (c, y) in enumerate([(0.9, True)] * 12 + [(0.6, False)] * 8 + [(0.6, True)] * 4 + [(0.9, False)] * 2)]
    occ.add_labels("josh", labels)
    T = occ.temperature(KISKADEE)
    assert 0.25 <= T <= 4.0 and T != occ.DEFAULT_T, T


def test_departure_and_arrival_events():
    fresh()
    for d in range(5):
        day_of_listening(d, heard=[(KISKADEE, 2, 0.8)])
    kinds = [c["kind"] for c in occ.changes()]
    assert kinds.count("arrival") == 1 and kinds[0] == "arrival", kinds
    for d in range(5, 40):
        day_of_listening(d)
    kinds = [c["kind"] for c in occ.changes()]
    assert "departure" in kinds, kinds
    v = view(KISKADEE, (win_at(39, 19) + 1) * 900)
    assert v["status"] in ("Fading", "No longer heard"), v
    assert all(c["chain"].startswith("not submitted") for c in occ.changes())
    print("after 35 quiet days:", v["p"], v["status"], kinds)


if __name__ == "__main__":
    for name, fn in list(globals().items()):
        if name.startswith("test_"):
            fn(); print("ok", name)
    print("ALL OCCUPANCY TESTS PASSED")
