# BTCPC Protocol Invariants Audit
**Date:** 2026-05-07  
**Scope:** Full chain state machine — reward logic, entry validation, daemon vs chain enforcement  
**Status:** All confirmed vulnerabilities patched

---

## Why This Audit Is Separate From the Security Scanner

The automated security scanner (`/security-review`) catches **outsider exploitation**: SQL injection, XSS, authentication bypass, credential theft, RCE via malformed input. Its threat model is an attacker who has no valid account.

This audit covers a different class: **insider economics abuse** — where the attacker is a valid participant who submits correctly signed, well-formed entries that the chain *was* accepting. No signature bypass. No malformed input. Just protocol rules that lived in the daemon but not in the state machine.

The scanner missed all of these because:
1. They were in pre-existing code, not in the diff it was given
2. They look like correct auth checks from a signature perspective
3. "Business logic" exclusions in the scanner rules filtered them out
4. They require understanding the reward economics to recognize as exploitable

---

## Confirmed Vulnerabilities — Fixed 2026-05-07

### V1 — Verifier self-verification
**File:** `src/inference.rs:apply_verify`  
**Class:** Conflict of interest / reward inflation  
**How:** A node could submit `InferenceJobVerify` for a job it won, earning both the miner reward and the verifier pool reward for the same work.  
**Daemon guard:** Yes — `run_inference_verifier` skipped `job.winner == self`  
**Chain guard:** None (missing)  
**Fix:** Added `bail!` in `apply_verify` when `verifier == job.winner`. Enforced at state machine level, not daemon level.

---

### V2 — Late bid after window closes
**File:** `src/inference.rs:apply_bid`  
**Class:** Bid manipulation  
**How:** `apply_bid` only checked `status == Posted`, not that the bid window epoch had not passed. A node could submit a bid after the window closed. While the daemon's award logic wouldn't select it, the bid record was written to state and could confuse award selection or inflate bid indices.  
**Daemon guard:** Yes — worker loop filtered `epoch < posted_epoch + bid_window_epochs`  
**Chain guard:** None (missing)  
**Fix:** Added epoch window check in `apply_bid`.

---

### V3 — Requester bids on own job
**File:** `src/inference.rs:apply_bid`  
**Class:** Self-dealing / reward extraction  
**How:** A job poster could bid on their own job as worker at minimum fee. Since bid selection uses reputation×fee scoring and the requester controls the job parameters, they could guarantee winning their own job and collecting the "worker" reward on top of getting the work done.  
**Daemon guard:** None (daemon would naturally not do this, but chain didn't stop it)  
**Chain guard:** None (missing)  
**Fix:** Added `bidder != job.requester` check in `apply_bid`.

---

### V4 — Requester cancels Awarded job to punish winner
**File:** `src/inference.rs:apply_cancel`  
**Class:** Worker punishment / griefing  
**How:** `apply_cancel` allowed the requester to cancel a job with `status == Awarded` at any time. A requester who lost a bid to a competitor could cancel the moment a worker was assigned, getting a full refund while the worker had already started running inference. Could be used to prevent any specific node from earning.  
**Daemon guard:** Yes — system-side cancellation only happens after `current_epoch > deadline_epoch`  
**Chain guard:** None (missing)  
**Fix:** Requester can only cancel `Posted` jobs. `Awarded` jobs can only be cancelled by `"system"` (post-deadline). Added `bail!` when `cancelled_by == requester && status == Awarded`.

---

### V5 — Unlimited verifiers per job
**File:** `src/inference.rs:apply_award`  
**Class:** Reward pool dilution  
**How:** The daemon selects `take(3)` verifiers, but the chain had no cap. A buggy or forked daemon node could assign 50 verifiers to a single job, splitting the fixed verifier pool reward so thin that legitimate verifiers earn near-zero per job.  
**Daemon guard:** Yes — `.take(3)` in `inference_daemon.rs`  
**Chain guard:** None (missing)  
**Fix:** Added `verifiers.len() >= 3` check in `apply_award`, plus duplicate verifier check.

---

### V6 — StorageHeartbeat accepts phantom accounts
**File:** `src/chain.rs` — `StorageHeartbeat` arm  
**Class:** Fake account reward farming  
**How:** `node_id` in a `StorageHeartbeat` was never verified as an existing on-chain account. Any string could be submitted as `node_id` and earn storage reward slots, draining the storage pool.  
**Daemon guard:** Yes — daemon only uses `cfg.account` which is a real registered account  
**Chain guard:** None (missing)  
**Fix:** Added `ensure_account(node_id, epoch)` before writing the heartbeat record.

---

### V7 — ServiceHeartbeat accepts phantom accounts
**File:** `src/chain.rs` — `ServiceHeartbeat` arm  
**Class:** Fake account reward farming  
**Same pattern as V6 but for the service pool.**  
**Fix:** Added `ensure_account(node_id, epoch)`.

---

### V8 — MempoolHeartbeat accepts phantom accounts
**File:** `src/chain.rs` — `MempoolHeartbeat` arm  
**Class:** Fake account reward farming  
**Same pattern as V6 but for the mempool pool.**  
**Fix:** Added `ensure_account(operator, epoch)`.

---

### V9 — SensorDataCommit accepts phantom owners
**File:** `src/chain.rs` — `SensorDataCommit` arm  
**Class:** Fake sensor reward farming  
**How:** `owner` was never verified as an existing account, and `sensor_id` ownership was never checked against a `SensorRegister` entry. Any node could claim commits for sensors they didn't register, under account names that don't exist.  
**Daemon guard:** Yes — daemon uses real `cfg.account`  
**Chain guard:** None (missing)  
**Fix:** Added `ensure_account(owner, epoch)` plus registered-sensor ownership check (VEC-2 and VEC-3 fixed in same pass — see below).

---

## Open Vectors — Fixed 2026-05-08

### VEC-2 — Sensor `reading_count` unbounded ✓ FIXED
**File:** `src/chain.rs` — `SensorDataCommit` arm  
**Fix:** Added `MAX_READINGS_PER_EPOCH = 10_000` cap. Submissions above this are rejected. No sensor type scores meaningfully past this value; the cap prevents unbounded state and future scoring drift.

### VEC-3 — Sensor ownership not verified at commit time ✓ FIXED
**File:** `src/chain.rs` — `SensorDataCommit` arm  
**Fix:** If `sensor:{sensor_id}` registration record exists on-chain, the `owner` field in the commit must match. Unregistered sensors are still accepted (open era), but once a sensor is claimed its data can only be committed by the owner.

### VEC-4 — `InferenceJobBid` epoch field used for window check ✓ FIXED
**File:** `src/inference.rs:apply_bid`  
**Fix:** Bid window check now uses `chain.current_epoch()` instead of the submitted `epoch` field. A node cannot manipulate the bid window by supplying a past-epoch value in their bid.

### VEC-6 — `InferenceJobComplete` has no epoch staleness window ✓ FIXED
**File:** `src/inference.rs:apply_complete`  
**Fix:** Added `COMPLETE_STALE_WINDOW = 3` epochs past the job's deadline. If `current_epoch > deadline_epoch + 3`, the completion is rejected. Mirrors the Mine staleness guard and closes the race between the deadline daemon and a slow worker on a stale-view node.

---

## Open Vectors — Require Design Changes or Future Triggers

### VEC-1 — Verifier rubber-stamp ring rotation
**File:** `src/inference.rs:apply_verify`  
**Risk:** Medium  
The suspension trigger fires at >99% approval rate over ≥10 verdicts. A ring of N verifiers can rotate so each processes fewer than 10 jobs before rotating, avoiding suspension indefinitely. Each earns block reward credit for rubber-stamping without genuine quality assessment.  
**Not fixable with a single guard.** Requires either: (a) longer suspension windows, (b) cross-verifier reputation consensus, or (c) random verifier assignment (`VERIFIER_ASSIGNMENT_ENABLED` flag exists but is off).  
**Watch when:** Daily job volume exceeds 100/epoch, or verifier pool value exceeds inference pool value.

### VEC-5 — Mempool relay count is self-reported
**File:** `src/main.rs` — `run_mempool_node`  
**Risk:** Low → Medium at scale  
`entries_relayed` comes from an in-process `AtomicUsize`. A modified binary can inflate it to any value. `mempool_relay_score()` divides by latency, but a node reporting 1ms + 100,000 relayed dominates the pool with no cross-node attestation.  
**Fix requires:** Peer-signed relay receipts, or moving to a stake-weighted scoring model where inflating claims above what stake can cover results in slash. Not implementable without cross-node coordination protocol.  
**Watch when:** Mempool pool reward share exceeds 5% of block reward, or when mempool stake is introduced.

### VEC-8 — `/v1/chat/completions` Bearer token equals public account name ✓ FIXED
**File:** `src/api.rs` — `post_v1_chat_completions`  
**Fixed:** 2026-05-08  
New entry type `AccountApiKeySet { account, api_key, epoch, nonce, signed_by }`. The `api_key` is a 64-char hex (32 random bytes) generated by the account owner and signed with their active key. The chain indexes `api_key:{token}` → account. The API handler resolves Bearer tokens via this index first; falls back to direct account name only for backwards compatibility during migration. CLI: `btcpc wallet api-key-gen --mnemonic "..."` generates, registers, and writes the key to `.btcpc/wallet.env`.

---

### VEC-7 — `StorageHeartbeat` `bytes_proven` is self-reported without penalty ✓ ALREADY HANDLED
**File:** `src/chain.rs` — `StorageHeartbeat` arm  
`proof_valid: false` submissions are already penalised at epoch seal: `score = raw * STORAGE_PROOF_NO_PROOF_BPS / 10000` (20% of normal score). This was already implemented in `main.rs`. Not a gap.

---

## Invariants That Should Be Tested Per Epoch

These are properties that must hold across all entry applications. Any violation is a protocol bug:

1. Total supply never exceeds 42,000,000 BTCPC (4,200,000,000,000,000 dreams)
2. A node cannot be both the worker winner and a verifier on the same job
3. A verifier cannot submit two verdicts on the same job
4. An account balance never goes negative (debit must have sufficient funds)
5. Epoch rewards cannot be issued for a future epoch
6. A job in `Cancelled`, `Paid`, or `Rejected` state cannot transition to any other state
7. `InferenceJobAward` for role `worker` sets exactly one winner; second award to same job as worker must fail
8. Reward pool distribution total per epoch ≤ `block_reward_at(epoch)` (net of reserve)
