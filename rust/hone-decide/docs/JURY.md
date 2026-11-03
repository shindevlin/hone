# hone-decide jury API (v1)

This is the hone-decide side of the decide jury (spec: Hone project doc
`claude/hone-decide-jury-spec-2026-11-03.md`). The node side (entries, sampling, counting,
shadow mode) is built by the chain agent against this contract.

- The service is localhost only and serves one request at a time (concurrency 1).
- Votes are stored in `--votes-dir`, which is `/var/lib/hone-decide/votes` under systemd via `StateDirectory`.

## POST /vote
Request:
```json
{"epoch": 352010, "items": [{"entry_hash": "<64 hex>", "task": "peer_behavior", "features": {"...": "..."}}]}
```
- **Items:** at most 32 per call. Plan on about 10 s per item on nebra; set the client timeout to ≥ 10 s × items + 30 s.
- **Features gate:** `features` must be an object of numbers, bools, null, and token strings
  matching `[A-Za-z0-9_:.+\-/%]{0,64}`. Nesting is at most 3 levels and the total is at most 2000 chars.
  - Anything else is reported in `errors` for that item and gets no vote.
  - Unknown item fields are rejected, so there is no canary flag. Canaries must be shaped exactly like real items.
  - `features` is rendered as sorted-key JSON and becomes the model context. The task's fixed
    question and candidates come from the registry (`GET /tasks`). Nothing else reaches the model.
- **Revoting:** an entry already voted in this epoch returns the stored vote (same leaf). There is no revote.
- **Epoch guards:** an epoch at or below the highest sealed epoch returns `409`. An epoch more than 2 ahead
  of the newest epoch seen returns `400`. The allowance grows by 1 per 30 s of wall time since the newest
  epoch advanced, so a node that was offline can resume.
- **Errors:** a bad item never fails the call; it goes in `errors[]`. Any non-200 response for the whole call,
  or a timeout, means the node skips voting that epoch.

Response:
```json
{"epoch": 352010, "model_hash": "<64 hex>", "latency_ms": 9800,
 "votes": [{"entry_hash": "...", "task": "...", "option": "normal", "conf_q8": 231, "t95_q8": 145, "leaf": "<64 hex>"}],
 "errors": [{"entry_hash": "...", "error": "..."}]}
```
- `option` is a candidate name, or `abstain` when `conf < t95`.
- `conf_q8 = round(conf × 255)` and `t95_q8 = round(t95 × 255)`.
- **Where t95 comes from:**
  1. the model dir's `thresholds.json` (fitted per model on held-out data, see `py/fit_thresholds.py`);
  2. otherwise the registry's `abstain_below` (from `CONFIDENCE_MAP.md`);
  3. otherwise 0.95.

## Encoding
```
vote_bytes = "hone-decide-vote-v1" || entry_hash (32 raw bytes) || epoch (u64 LE)
             || u8 len || task (utf8) || u8 len || option (utf8, "abstain" when abstaining) || conf_q8 (u8)
leaf       = sha256(vote_bytes || salt)          salt = 32 random bytes (/dev/urandom)
tree       = leaves sorted by entry_hash; parent = sha256(0x01 || left || right);
             an odd last node is carried up unchanged; empty epoch root = 32 zero bytes
```

## POST /commit/{epoch} and GET /commit/{epoch}
Both return `{"epoch", "root", "n_votes", "model_hash", "sealed": true}`.
- **POST seals** the epoch: the commit file is written atomically and fsynced, and no more votes are accepted.
  It is idempotent: repeats return the same root.
- **GET is read-only:** it returns 404 until the epoch is sealed, and the node may poll it.
- The node calls POST once its epoch is over, then publishes `DECIDE_COMMIT`.

## GET /reveal/{epoch}/{entry_hash}
Returns `{"vote": {"entry_hash", "epoch", "task", "option", "conf_q8", "t95_q8", "model_hash"}, "salt", "leaf", "proof": [{"side": "L"|"R", "hash"}], "root", "committed_root"}`.
- `t95_q8` and `model_hash` are not part of the leaf. They are returned so the node can re-check abstain and
  compare against `DECIDE_MODEL_HASH`.
- All hashes are lowercase hex, 64 chars.
- `side` is the sibling's position: `L` means `acc = sha256(01 || sibling || acc)`.
- Reference verifier: `py/jury_verify.py`.
- Returns 404 when the epoch is unsealed, was never voted, or has been pruned.

## GET /capacity
Returns `{"ms_per_vote_p50", "samples", "epoch_ms": 30000, "budget_fraction", "concurrency": 1, "votes_per_epoch"}`.
- `votes_per_epoch = floor(epoch_ms × budget_fraction / p50)`, measured over the last 50 voted items.
- If nothing has been voted yet, one fixed peer_behavior item is run as a self-benchmark.
- `HONE_DECIDE_BUDGET` sets the fraction (default 0.5).
- `votes_per_100_epochs` covers nodes slower than one vote per budgeted epoch. nebra measures about
  22 s per HONE vote, so it reports `votes_per_epoch: 0` and `votes_per_100_epochs: 68`; the node should sample
  with that rate. grouchly reports about 10–14 votes/epoch.

## model_hash
`sha256` over the sorted model-dir files `{config.json, decide.json, head.safetensors,
thresholds.json (if present), tokenizer.json, <weights>}`, hashing `name || 0x00 || sha256(file)` for each.
- It is identical on every node with the same files, so pin it as `DECIDE_MODEL_HASH`.
- It is also in `GET /health`.

## Durability and retention
- **Vote files:** `<epoch>.votes` is JSON lines, appended once per `/vote` call, followed by `fdatasync` and a directory fsync.
  - Reads skip unparsable lines, so a power cut that leaves a torn or NUL-filled tail does not break the epoch.
  - A vote whose line was torn is lost. It was never committed, so nothing can be challenged on it.
- **Commit files:** `<epoch>.commit` is written to a temp file, fsynced, renamed, and the directory is fsynced.
- **Retention:** epochs older than newest − 2880 are deleted on the first vote of a new epoch.
- **Size:** about 330 bytes per vote. nebra at 1–3 votes/epoch for 2880 epochs is ≤ 3 MB. A busy 14 votes/epoch node is about 13 MB.

## Feature shapes (examples)
The tasks and their generators are in `py/gen_tasks.py` and `py/gen_tasks2.py`. Send the same shape as the
generators, with token strings only.
- **peer_behavior:** `{"peer", "window_epochs", "stats": {...}, "network_medians": {...}}`
- **coverage_usefulness:** `{"report": {...}, "cell_this_epoch": {...}, "derived": {...}, "rules": {...}}`
- **entry_screen:** `{"entry": {...}, "submitter_recent": {...}, "derived": {...}}`
- **sensor_plausibility_v2:** `{"sensor_id", "kind", "unit", "physical_range", "recent": [...], "latest": {...}, "derived": {...}}`

## Phones (eligibility)
`serve --eligible-file F` turns voting on only while F contains `1`.
- The Android juror service writes F from three conditions: charging, unmetered Wi-Fi, and thermal status below SEVERE.
- While ineligible, `/capacity` reports `votes_per_epoch: 0, eligible: false` and `/vote` returns 503. The node skips the epoch.
