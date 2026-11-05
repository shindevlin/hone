#!/usr/bin/env bash
# Held-out replay with the dev-tuned generic setting (tuned on chiffchaff day1->day2 only):
# join_tau 0.62, back_tau 0.60, merge_tau 0.75, k_min 5. Logistic per species from eval5.
cd /home/beastly/voiceprint-eval
g=out/gates-final-sim.json
echo '{"species": {}}' > $g
for sp in "Athene noctua" "Anthus trivialis" "Phylloscopus collybita" "Parus major" "Apteryx haastii" "Pitangus sulphuratus"; do
  venv/bin/python scripts/addgate.py $g "$sp" '{"join_tau": 0.62, "back_tau": 0.6, "merge_tau": 0.75, "k_min": 5}' > /dev/null
done
venv/bin/python -u scripts/sim.py out/sim-final.json $g \
  "Athene_noctua=out/littleowl.npz:sessions:acrossyear-trn,acrossyear-tst" \
  "Anthus_trivialis=out/pipit.npz:sessions:acrossyear-trn,acrossyear-tst" \
  "Phylloscopus_collybita@year=out/chiffchaff.npz:sessions:acrossyear-trn,acrossyear-tst" \
  "Phylloscopus_collybita@day=out/chiffchaff.npz:sessions:withinyear-trn,withinyear-tst" \
  "Parus_major=out/greattit.npz:dates" "$@"
echo SIM_DONE
