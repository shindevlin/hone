import random, copy, unittest
import hone_rawfmt as F
P = {"dev": "abcd", "cls": "nebra", "sensors": {"wifi": "iw-2g", "env": "bme280"}, "loc": "fixed"}

class T(unittest.TestCase):
    def test_any_moment_rebuilds_exactly(self):
        rnd = random.Random(7)
        enc = F.ChangeEncoder(prov=P)
        recs, snaps = [], []
        t = 1_794_000_000.0
        enc.keyframe(t, "boot"); recs += enc.take(); snaps.append((t, copy.deepcopy(enc.reported)))
        aps = {f"wifi:{i:02x}": rnd.uniform(-90, -40) for i in range(40)}
        temp = 24.0
        for step in range(3000):
            t += rnd.choice([1, 5, 30, 60])
            vis = {k: (v + rnd.gauss(0, 4), {"f": 2437}) for k, v in aps.items() if rnd.random() < 0.7}
            enc.observe_set(t, "wifi", vis)
            temp += rnd.gauss(0, 0.08); enc.observe(t, "env.air_temp_c", round(temp, 2))
            enc.observe(t, "env.light_lux", max(0.0, rnd.gauss(300, 60)))
            if step % 15 == 0: enc.end_window(t, ["wifi"])
            if step % 400 == 0: enc.gap(t - 30, t, "ble", "permission_denied")
            if step == 1500: enc.keyframe(t, "day")
            recs += enc.take(); snaps.append((t, copy.deepcopy(enc.reported)))
        for (ts, snap) in snaps[::37] + snaps[-1:]:
            self.assertEqual(F.rebuild(recs, until=ts), snap, ts)
        # round trip through JSONL
        self.assertEqual(F.rebuild(F.from_jsonl(F.to_jsonl(recs))), snaps[-1][1])

    def test_deadbands(self):
        enc = F.ChangeEncoder(prov=P); enc.keyframe(0, "boot"); enc.take()
        enc.observe(1, "env.air_temp_c", 20.0); enc.observe(2, "env.air_temp_c", 20.1); enc.observe(3, "env.air_temp_c", 20.2)
        es = [r["e"] for r in enc.take()]
        self.assertEqual(es, ["appeared", "changed"])          # 0.1 inside the band, 0.2 outside
        enc.observe_set(4, "wifi", {"wifi:a": (-50, None)}); enc.observe_set(5, "wifi", {"wifi:a": (-55, None)})
        enc.observe_set(6, "wifi", {"wifi:a": (-57, None)})
        self.assertEqual([r["e"] for r in enc.take()], ["appeared", "changed"])   # 7 dB from the reported -50
        enc.observe_set(7, "wifi", {}); self.assertEqual(enc.take(), [])          # one miss is not gone
        enc.observe_set(8, "wifi", {}); self.assertEqual([r["e"] for r in enc.take()], ["disappeared"])

    def test_markers_and_forced(self):
        enc = F.ChangeEncoder(prov=P); enc.keyframe(0, "boot"); enc.take()
        enc.observe(10, "env.pressure_pa", 101300.0); enc.observe_set(10, "wifi", {"wifi:a": (-60, None)}); enc.take()
        enc.end_window(900, ["wifi", "ble"])
        r = enc.take()
        self.assertTrue(any(x["k"] == "wifi:*" and x["e"] == "unchanged" for x in r))
        self.assertTrue(any(x["k"] == "env.pressure_pa" and x.get("forced") and x["v"] == 101300.0 for x in r))

    def test_provenance_on_every_record(self):
        enc = F.ChangeEncoder(prov=P); enc.keyframe(0, "boot")
        enc.observe_set(1, "wifi", {"wifi:a": (-50, None)}); enc.observe(2, "env.rh_pct", 60.0)
        enc.obs(3, "lora:pkt", "lora_uplink", data="QUJD", rssi=-110)
        enc.gap(0, 900, "ble", "permission_denied"); enc.end_window(900, ["wifi"])
        recs = enc.take()
        self.assertIsNone(F.check_prov(recs))
        self.assertEqual({r["sen"] for r in recs if r.get("k") == "wifi:a"}, {"iw-2g"})
        self.assertEqual({r["sen"] for r in recs if r.get("k") == "env.rh_pct"}, {"bme280"})
        self.assertTrue(all(r["loc"] == "fixed" for r in recs))
        bad = dict(recs[1]); del bad["why"]
        self.assertIs(F.check_prov([bad]), bad)
        with self.assertRaises(ValueError): F.ChangeEncoder(prov={"cls": "x"})

    def test_obs_type_and_v3_provenance(self):
        enc = F.ChangeEncoder(prov=P); enc.keyframe(0, "boot")
        enc.observe(1, "env.pressure_pa", 101300.0); enc.observe_set(2, "cell", {"cell:ab": (-95, {"rat": "LTE"})})
        enc.observe_set(3, "wifi", {"wifi:a": (-50, None)}); enc.end_window(900, ["wifi", "cell"])
        recs = enc.take()
        self.assertIsNone(F.check_prov(recs))
        ot = {r["k"]: r["obs_type"] for r in recs if "k" in r}
        self.assertEqual(ot["env.pressure_pa"], "pressure"); self.assertEqual(ot["cell:ab"], "cell_coverage"); self.assertEqual(ot["wifi:a"], "wifi_env")
        self.assertTrue(any(r["k"] == "cell:ab" and r.get("forced") for r in recs if "k" in r))   # 15-min coverage sample
        bad = dict(recs[1]); del bad["obs_type"]
        self.assertIs(F.check_prov([bad]), bad)

    def test_colocation(self):
        a = {"wifi": [f"wifi:{i}" for i in range(20)], "ble": 30, "subghz": [100, 400, 900]}
        b = {"wifi": [f"wifi:{i}" for i in range(5, 25)], "ble": 26, "subghz": [110, 905]}
        far = {"wifi": [f"wifi:x{i}" for i in range(20)], "ble": 3, "subghz": [5000]}
        r = F.colocate(a, b); self.assertTrue(r["match"]); self.assertEqual(len(r["proof"]), 64)
        self.assertFalse(F.colocate(a, far)["match"])
        self.assertFalse(F.colocate({}, {})["match"])
        self.assertTrue(F.home_anchor(b, a)); self.assertFalse(F.home_anchor(far, a))
        recs = [{"s": F.SCHEMA_KF, "t": F.rfc3339(0), "state": {"wifi:a": {"v": -50}, "ble:x": {"v": -70}}},
                {"s": F.SCHEMA_EV, "t": F.rfc3339(10), "e": "appeared", "k": "wifi:b", "v": -60}]
        self.assertEqual(F.radio_env(recs), {"wifi": ["wifi:a", "wifi:b"], "ble": 1, "subghz": []})

if __name__ == "__main__":
    unittest.main()
