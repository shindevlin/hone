# Voiceprint evaluation (2026-11-04)

Required before any naming offer or bird card. Model: Perch v2, the 8-bit LiteRT build the
phone runs (`perch_v2_8bit.tflite`, sha256 9ccbfc42...). Every number below comes from that
model and from the shipped 64-dim int8 voiceprint (`c-rp64`), unless marked otherwise.
Scripts: `eval/scripts/`. Raw outputs were on beastly at `~/voiceprint-eval/out/`.

## Datasets and licences

| Set | Species | Individuals | Split used | Source | Licence |
|---|---|---|---|---|---|
| Stowell et al. 2018 | Little owl *Athene noctua* | 16 | 2017 train -> test (across year) | zenodo.org/records/1413495 | CC BY 4.0 |
| same | Tree pipit *Anthus trivialis* | 10 | across year | same | CC BY 4.0 |
| same | Chiffchaff *Phylloscopus collybita* | 13 / 10 | day 1 -> day 2; across year | same | CC BY 4.0 |
| Gallego Londono et al. 2026 (MEE, doi 10.1111/2041-210x.70399) | Great tit *Parus major* (Wytham, ringed) | 48 fetched, 23 with a later day | first day -> days >= 2 later | zenodo.org/records/20145425 | CC BY 4.0 audio, MIT code |
| same | Great spotted kiwi *Apteryx haastii* | 18 (F99/M99 unidentified, dropped) | earliest season -> > 60 days later | same | CC BY 4.0 |
| xeno-canto via GBIF (dataset b1047888) | Great Kiskadee *Pitangus sulphuratus* | 120 recordings, 46 usable, one per recordist+place | first 40% -> last 40% of one recording | xeno-canto.org | per file: 106 CC BY-NC-SA, 8 CC BY-NC-ND, 2 CC BY-SA, 4 CC0 |

Not used: Lostanlen et al. (zenodo 15054394, CC BY-NC 4.0) cockatoo/penguin embeddings. They are
not Perch v2 int8 and the licence is non-commercial. Kiskadee audio was used only for
evaluation on beastly. It is not redistributed and not committed. A per-file manifest with
recordist, place, date and licence is `data/xc/Pitangus_sulphuratus/manifest.tsv` on beastly.
The NC/ND terms allow this evaluation use but would not allow shipping the audio or a model trained on it.

## 1. Embedding-level re-identification (eval.py)

Enrol half the individuals per rotation (20 rotations) from the earlier session, then decide on
k calls from the later session. A decision is the best enrolled centroid if the cosine reaches tau,
else "unknown". Precision = right names / names given. Recall = right names / decisions about
enrolled birds. Tau is held out: the strictest per-species tau reaching P >= 0.9 on the other
species (eval5, without kiwi). The kiwi row is from eval6.

| Set | AUC (pairs) | EER | k=1 P / R | k=5 P / R |
|---|---|---|---|---|
| little owl, across year | 0.817 | 0.26 | 0.91 / 0.13 | 0.94 / 0.37 |
| tree pipit, across year | 0.600 | 0.43 | 1.00 / 0.02 | 0.96 / 0.09 |
| chiffchaff, day 1 -> 2 | 0.648 | 0.40 | 0.91 / 0.06 | 0.97 / 0.24 |
| chiffchaff, across year | 0.530 | 0.48 | 0 / 0 | 0 / 0 |
| great tit, >= 2 days later | 0.686 | 0.38 | 0.95 / 0.05 | 0.92 / 0.23 |
| great tit, same day (reference) | 0.805 | 0.27 | 1.00 / 0.15 | 1.00 / 0.56 |
| great spotted kiwi, across season | 0.881 | 0.20 | 0.67 / 0.77 | 0.58 / 0.89 |
| **kiskadee, two halves of ONE recording** | 0.974 | 0.08 | 0.96 / 0.55 | 0.98 / 0.96 |

Other projections we tried, on little owl AUC: raw 1536 0.725, centred 1536 0.846, centred rp128
0.848, centred rp64 0.817, whitened 0.68-0.72, PCA-64 from other species 0.773, LDA from other
species 0.796. Centring per place is the main gain. A learned head trained on other species did
not transfer. We ship rp64: it is 68 bytes, needs no training data, and is 3% AUC below the best.

## 2. Whole-system replay (sim.py, through the real crate via examples/vp_sim)

40 simulated homes per species, each with 2-5 real individuals interleaved, 30 s apart,
over real nights. Labels never enter the crate. Generic parameters were tuned on the
chiffchaff day 1 -> 2 set ONLY: join 0.62, back 0.60, merge 0.75, k_min 5.
"Gap" counts a "X is back" said on a night after a real gap (the next session, or >= 2 days later)
about a candidate that formed before the gap. This is the number that matters.

| Species | back events | precision | false back (owner absent) | gap events | gap precision | gap recall | ids per bird |
|---|---|---|---|---|---|---|---|
| little owl, across year | 74 | 1.00 | 0 | 22 | **1.00** | 0.20 | 1.72 |
| tree pipit, across year | 24 | 0.83 | 0 | 13 | 0.69 | 0.10 | 1.64 |
| chiffchaff, across year | 0 | n/a | n/a | 0 | n/a | 0 | 1.12 |
| chiffchaff day 1 -> 2 (dev set, optimistic) | 124 | 0.97 | 0 | 56 | 1.00 | 0.42 | 1.45 |
| great tit, real dates | 5 | 1.00 | 0 | 0 | n/a | 0 | 1.08 |
| great spotted kiwi, real dates | 996 | 0.82 | 0.17 | 838 | 0.79 | 0.43 | 3.05 |

Likelihood calibration: for little owl the mean likelihood shown on correct backs is 0.94, with no
wrong backs. For failing species, wrong backs carry the same high likelihoods as right ones
(pipit 0.99 vs 0.96, kiwi 0.88 vs 0.91). The number is only meaningful for species that pass
the gate, which is one more reason the gate exists.

## 3. Naming gate

Naming is allowed when the held-out replay gap precision is >= 0.90 with >= 20 gap events, AND
embedding-level k=5 precision is >= 0.90 across sessions, on data not used for tuning.

| Species | Gate | Why |
|---|---|---|
| *Athene noctua* little owl | **PASS** | gap P 1.00 (22), k5 P 0.94. Recall is low (0.20): most returns go unannounced, but the ones said were right |
| *Anthus trivialis* tree pipit | fail | gap P 0.69 |
| *Phylloscopus collybita* chiffchaff | fail | across year at chance. Day 1 -> 2 passes, but that is the tuning set |
| *Parus major* great tit | fail | only 5 backs, 0 after a gap; repertoire singer |
| *Apteryx haastii* great spotted kiwi | fail | gap P 0.79, 17% backs about an absent bird, 3 ids per bird |
| *Pitangus sulphuratus* Great Kiskadee | **fail (no evidence yet)** | see below |
| every other species | fail by default | not evaluated |

## 4. Great Kiskadee at Josh's home

Kiskadees can plausibly pass, but nothing tested so far proves it.
- For: within one recording, 46 kiskadees separate better than any other species we tested (AUC
  0.97 vs 0.80 for great tit same day and 0.82 for owl). Kiskadees are suboscines with largely
  innate, stereotyped calls. In another tyrant flycatcher, Eastern Phoebe, songs are individually
  distinctive (85-90% assignment; Foote, Palazzi & Mennill 2013, Bioacoustics 22:137) and repeatable
  across sessions.
- Against: same-recording halves share mic, distance, weather and background. That inflates the
  number: great tit drops from 0.81 same day to 0.69 days later. Kiskadees have several call
  types ("bien-te-veo", rattles, alarm), and a home often hosts a pair plus neighbours.
- No public dataset of individually marked kiskadees exists that we could find.

To open the gate, run Josh's test (needs Josh): run the phone in shadow mode at the home for 14+
nights (clustering on, naming hidden), and record ground truth for one more bird. That can be a
second property 1+ km away (different birds), or an in-person note on nights when he sees the
marked or distinctive bird call. If the replay on those nights shows gap precision >= 0.9, flip
`naming_allowed` for *Pitangus sulphuratus* in `eval/gates.json` and record the run here.
