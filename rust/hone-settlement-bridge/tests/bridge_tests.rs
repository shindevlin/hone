//! End-to-end tests for the settlement bridge's own logic.
//!
//! The chain submission boundary is `MockChain` (a model of the real chain's
//! nonce / no-peers / balance semantics). NO test here claims a real on-chain
//! post succeeded — see the crate docs and the PLATFORM_PRD.md DECISION RECORD
//! "Build status" for what is real vs. mocked.
//!
//! Required scenarios (from the seam's design record):
//!   1. idempotent re-post — a retried/duplicate intent posts exactly once
//!   2. crash-recovery re-drive — restart re-drives pending/posting/failed
//!   3. a USD charge always eventually settles — no collected-but-unsettled
//!   4. wrong/stale fx_rate is rejected

use hone_settlement_bridge::{
    reconcile, ChainSubmit, DriveResult, FxPolicy, FxRate, FxRateBook, FxRateId, IntentStatus,
    IntentStore, JsonFileStore, MemStore, MockChain, SettlementIntent, SettlementWorker,
    SigningIdentity, WorkerConfig,
};
use hone_types::entry::LedgerEntry;
use hone_types::HUNITS_PER_HONE;

const CHAIN_ID: &str = "hone-testnet";
const SETTLE_ACCT: &str = "platform-settlement";

fn identity() -> SigningIdentity {
    SigningIdentity::new(
        SETTLE_ACCT,
        ed25519_dalek::SigningKey::from_bytes(&[9u8; 32]),
    )
}

/// A pinned rate: 1 HONE = $0.50 => 200_000_000 hunits per USD cent, fresh.
fn fresh_book(now: u64) -> FxRateBook {
    let mut b = FxRateBook::new();
    b.pin(FxRate {
        id: FxRateId("r-fresh".into()),
        hunits_per_usd_cent: (HUNITS_PER_HONE as u128) * 2 / 100,
        as_of: now,
        source: "test-oracle".into(),
    });
    b
}

fn mk_intent(id: &str, ueid: &str, sensor: &str, owner: &str, batch: &str, cents: u64) -> SettlementIntent {
    SettlementIntent::new_pending(
        id,
        ueid,
        sensor,
        owner,
        batch,
        cents,
        FxRateId("r-fresh".into()),
        100,
    )
}

fn worker<'a>(
    store: &'a mut MemStore,
    chain: &'a mut MockChain,
    fx: &'a FxRateBook,
    id: &'a SigningIdentity,
    now: u64,
) -> SettlementWorker<'a, MemStore, MockChain> {
    SettlementWorker {
        store,
        chain,
        fx,
        fx_policy: FxPolicy::default(),
        identity: id,
        config: WorkerConfig {
            chain_id: CHAIN_ID.to_string(),
            loud_after_attempts: 5,
        },
        now,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 1. Idempotent re-post: a retried/duplicate intent posts exactly once.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn duplicate_intent_is_rejected_by_idempotency_key() {
    let mut store = MemStore::new();
    store
        .enqueue(mk_intent("i1", "q1", "s1", "owner-a", "batch-x", 250))
        .unwrap();
    // Same (sensor, batch, usage_event) with a different id => duplicate.
    let err = store
        .enqueue(mk_intent("i2", "q1", "s1", "owner-a", "batch-x", 250))
        .unwrap_err();
    assert!(
        format!("{err}").contains("duplicate"),
        "duplicate intent must be rejected, got {err}"
    );
}

#[test]
fn redriving_a_posted_intent_does_not_double_post() {
    let now = 1000;
    let id = identity();
    let fx = fresh_book(now);
    let mut store = MemStore::new();
    let mut chain = MockChain::new(7);
    store
        .enqueue(mk_intent("i1", "q1", "s1", "owner-a", "batch-x", 250))
        .unwrap();

    // First drive: posts.
    {
        let mut w = worker(&mut store, &mut chain, &fx, &id, now);
        let r = w.redrive();
        assert_eq!(r.len(), 1);
        assert!(matches!(r[0].1, DriveResult::Posted { .. }), "got {:?}", r[0].1);
    }
    let after_first = chain.next_nonce(SETTLE_ACCT);
    let posted = store.get("i1").unwrap().unwrap();
    assert_eq!(posted.status, IntentStatus::Posted);
    let hash1 = posted.chain_entry_hash.clone().unwrap();

    // Second drive: the intent is Posted, so it's not in the open set — no
    // second submit, nonce unchanged, hash unchanged.
    {
        let mut w = worker(&mut store, &mut chain, &fx, &id, now);
        let r = w.redrive();
        assert!(r.is_empty(), "posted intent must not be re-driven, got {r:?}");
    }
    assert_eq!(
        chain.next_nonce(SETTLE_ACCT),
        after_first,
        "nonce must not advance on redrive of a posted intent"
    );
    assert_eq!(store.get("i1").unwrap().unwrap().chain_entry_hash.unwrap(), hash1);
}

#[test]
fn resubmitting_same_nonce_reconciles_not_double_credits() {
    // Directly exercise the chain's double-post guard: two submits with the same
    // pinned nonce => second is NonceAlreadyUsed, and the worker reconciles it
    // to Posted rather than crediting again.
    let now = 1000;
    let id = identity();
    let fx = fresh_book(now);
    let mut store = MemStore::new();
    let mut chain = MockChain::new(3);
    store
        .enqueue(mk_intent("i1", "q1", "s1", "owner-a", "batch-x", 250))
        .unwrap();

    // Pin + post once.
    {
        let mut w = worker(&mut store, &mut chain, &fx, &id, now);
        let intent = store_get(&w.store, "i1");
        let _ = w.post_settlement(&intent);
    }
    let posted = store.get("i1").unwrap().unwrap();
    assert_eq!(posted.status, IntentStatus::Posted);
    let pinned_nonce = posted.nonce.unwrap();

    // Force the intent back to Posting (as if a crash lost the `posted` write)
    // and re-drive: chain now reports NonceAlreadyUsed, worker reconciles.
    let mut reverted = posted.clone();
    reverted.status = IntentStatus::Posting;
    reverted.chain_entry_hash = None;
    store.update(&reverted).unwrap();

    {
        let mut w = worker(&mut store, &mut chain, &fx, &id, now);
        let intent = store_get(&w.store, "i1");
        let r = w.post_settlement(&intent);
        assert!(matches!(r, DriveResult::Posted { .. }), "got {r:?}");
    }
    // Nonce did NOT advance again (no second credit): still pinned_nonce+1.
    assert_eq!(chain.next_nonce(SETTLE_ACCT), pinned_nonce + 1);
    assert_eq!(store.get("i1").unwrap().unwrap().status, IntentStatus::Posted);
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. Crash-recovery re-drive: restart re-drives pending/posting/failed.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn crash_before_submit_is_recovered_on_restart() {
    // Simulate: intent enqueued (pending), process crashes before ever posting.
    // A brand-new worker (restart) over the SAME durable store drives it to
    // posted.
    let now = 1000;
    let id = identity();
    let fx = fresh_book(now);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("intents.json");

    {
        let mut store = JsonFileStore::open(&path).unwrap();
        store
            .enqueue(mk_intent("i1", "q1", "s1", "owner-a", "batch-x", 250))
            .unwrap();
        // crash here — worker never ran.
    }

    // Restart: fresh store handle + fresh chain, redrive.
    let mut store = JsonFileStore::open(&path).unwrap();
    let mut chain = MockChain::new(11);
    {
        let mut w = SettlementWorker {
            store: &mut store,
            chain: &mut chain,
            fx: &fx,
            fx_policy: FxPolicy::default(),
            identity: &id,
            config: WorkerConfig { chain_id: CHAIN_ID.into(), loud_after_attempts: 5 },
            now,
        };
        let r = w.redrive();
        assert_eq!(r.len(), 1);
        assert!(matches!(r[0].1, DriveResult::Posted { .. }), "got {:?}", r[0].1);
    }
    assert_eq!(store.get("i1").unwrap().unwrap().status, IntentStatus::Posted);
}

#[test]
fn crash_in_flight_after_seal_is_recovered_without_double_post() {
    // The dangerous case: the chain APPLIED the entry, but the worker crashed
    // before recording `posted` (it saw a transport error, not the hash). On
    // restart, redrive must find the sealed entry by pinned nonce and reconcile
    // — NOT submit again.
    let now = 1000;
    let id = identity();
    let fx = fresh_book(now);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("intents.json");

    let mut chain = MockChain::new(5);
    chain.crash_after_seal_once = true; // next submit seals then reports crash

    {
        let mut store = JsonFileStore::open(&path).unwrap();
        store
            .enqueue(mk_intent("i1", "q1", "s1", "owner-a", "batch-x", 250))
            .unwrap();
        let intent = store.get("i1").unwrap().unwrap();
        let mut w = SettlementWorker {
            store: &mut store,
            chain: &mut chain,
            fx: &fx,
            fx_policy: FxPolicy::default(),
            identity: &id,
            config: WorkerConfig { chain_id: CHAIN_ID.into(), loud_after_attempts: 5 },
            now,
        };
        // This submit seals on-chain but the worker sees a transient "crash".
        let r = w.post_settlement(&intent);
        assert!(matches!(r, DriveResult::Retry { .. }), "expected retry, got {r:?}");
    }
    // The chain sealed exactly one entry (nonce advanced to 2).
    assert_eq!(chain.next_nonce(SETTLE_ACCT), 2);
    // Durable state: the worker saw a transient error so the intent is `Failed`
    // (retryable) — but crucially with a PINNED nonce, which is what lets
    // crash-in-flight recovery find the already-sealed entry and avoid a
    // double-post. It is NOT `Posted` (the worker never learned the hash).
    let mid = JsonFileStore::open(&path).unwrap().get("i1").unwrap().unwrap();
    assert_ne!(mid.status, IntentStatus::Posted);
    assert_eq!(mid.nonce, Some(1), "nonce must be pinned before the submit that sealed");

    // Restart: redrive must reconcile via find_sealed, no second submit.
    let mut store = JsonFileStore::open(&path).unwrap();
    {
        let mut w = SettlementWorker {
            store: &mut store,
            chain: &mut chain,
            fx: &fx,
            fx_policy: FxPolicy::default(),
            identity: &id,
            config: WorkerConfig { chain_id: CHAIN_ID.into(), loud_after_attempts: 5 },
            now,
        };
        let r = w.redrive();
        assert_eq!(r.len(), 1);
        assert!(matches!(r[0].1, DriveResult::Posted { .. }), "got {:?}", r[0].1);
    }
    // CRUCIAL: nonce did NOT advance again — no double-post.
    assert_eq!(
        chain.next_nonce(SETTLE_ACCT),
        2,
        "crash-in-flight recovery must not submit a second entry"
    );
    assert_eq!(store.get("i1").unwrap().unwrap().status, IntentStatus::Posted);
}

// ─────────────────────────────────────────────────────────────────────────────
// 3. A USD charge always eventually settles (no collected-but-unsettled).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn intent_under_repeated_transient_failures_eventually_settles() {
    let now = 1000;
    let id = identity();
    let fx = fresh_book(now);
    let mut store = MemStore::new();
    let mut chain = MockChain::new(2);
    store
        .enqueue(mk_intent("i1", "q1", "s1", "owner-a", "batch-x", 250))
        .unwrap();

    // No peers for the first few ticks (hardline) — must NOT post, must stay open.
    chain.has_peers = false;
    for _ in 0..3 {
        let mut w = worker(&mut store, &mut chain, &fx, &id, now);
        let r = w.redrive();
        assert!(matches!(r[0].1, DriveResult::Retry { .. }), "got {:?}", r[0].1);
        assert_ne!(store.get("i1").unwrap().unwrap().status, IntentStatus::Posted);
    }
    // Peers return: next redrive settles it.
    chain.has_peers = true;
    {
        let mut w = worker(&mut store, &mut chain, &fx, &id, now);
        let r = w.redrive();
        assert!(matches!(r[0].1, DriveResult::Posted { .. }), "got {:?}", r[0].1);
    }
    let final_intent = store.get("i1").unwrap().unwrap();
    assert_eq!(final_intent.status, IntentStatus::Posted);
    assert!(final_intent.chain_entry_hash.is_some());
    // Exactly one seal.
    assert_eq!(chain.next_nonce(SETTLE_ACCT), 2);
}

#[test]
fn underfunded_settlement_account_retries_then_settles_after_topup() {
    let now = 1000;
    let id = identity();
    let fx = fresh_book(now);
    let mut store = MemStore::new();
    let mut chain = MockChain::new(2);
    // 250 cents at 200_000_000 hunits/cent => 50_000_000_000 hunits needed.
    chain.balance = 1_000; // way too low
    store
        .enqueue(mk_intent("i1", "q1", "s1", "owner-a", "batch-x", 250))
        .unwrap();

    {
        let mut w = worker(&mut store, &mut chain, &fx, &id, now);
        let r = w.redrive();
        assert!(matches!(r[0].1, DriveResult::Retry { .. }), "got {:?}", r[0].1);
    }
    assert_ne!(store.get("i1").unwrap().unwrap().status, IntentStatus::Posted);

    // Treasury top-up.
    chain.balance = u64::MAX;
    {
        let mut w = worker(&mut store, &mut chain, &fx, &id, now);
        let r = w.redrive();
        assert!(matches!(r[0].1, DriveResult::Posted { .. }), "got {:?}", r[0].1);
    }
    assert_eq!(store.get("i1").unwrap().unwrap().status, IntentStatus::Posted);
}

#[test]
fn reconciliation_reports_posted_vs_open() {
    let now = 1000;
    let id = identity();
    let fx = fresh_book(now);
    let mut store = MemStore::new();
    let mut chain = MockChain::new(2);
    store.enqueue(mk_intent("i1", "q1", "s1", "owner-a", "batch-x", 250)).unwrap();
    store.enqueue(mk_intent("i2", "q2", "s2", "owner-b", "batch-y", 100)).unwrap();
    // Post only i1 (i2 blocked by no peers won't happen; instead post both then
    // check totals).
    {
        let mut w = worker(&mut store, &mut chain, &fx, &id, now);
        let _ = w.redrive();
    }
    let r = reconcile(&store).unwrap();
    assert_eq!(r.total_intents, 2);
    assert_eq!(r.posted, 2);
    assert_eq!(r.open, 0);
    assert_eq!(r.usd_cents_posted, 350);
    // 250c*2e8 + 100c*2e8 = (350)*2e8 = 70_000_000_000 hunits.
    assert_eq!(r.hunits_posted, 70_000_000_000);
}

// ─────────────────────────────────────────────────────────────────────────────
// 4. Wrong/stale fx_rate is rejected (no post at a guessed/stale rate).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn stale_fx_rate_is_not_posted() {
    let id = identity();
    let mut store = MemStore::new();
    let mut chain = MockChain::new(2);
    // Rate pinned at t=0; drive at t = well beyond max_age.
    let mut fx = FxRateBook::new();
    fx.pin(FxRate {
        id: FxRateId("r-fresh".into()),
        hunits_per_usd_cent: (HUNITS_PER_HONE as u128) * 2 / 100,
        as_of: 0,
        source: "stale-oracle".into(),
    });
    store
        .enqueue(mk_intent("i1", "q1", "s1", "owner-a", "batch-x", 250))
        .unwrap();

    let late = FxPolicy::default().max_age_secs + 10_000;
    {
        let mut w = worker(&mut store, &mut chain, &fx, &id, late);
        let r = w.redrive();
        assert!(
            matches!(&r[0].1, DriveResult::Retry { reason } if reason.contains("fx")),
            "stale fx must yield a non-posting retry, got {:?}",
            r[0].1
        );
    }
    // Nothing posted, nonce not consumed.
    assert_ne!(store.get("i1").unwrap().unwrap().status, IntentStatus::Posted);
    assert_eq!(chain.next_nonce(SETTLE_ACCT), 1);
}

#[test]
fn missing_fx_rate_is_not_posted() {
    let id = identity();
    let now = 1000;
    let fx = FxRateBook::new(); // empty — the pinned id resolves to nothing
    let mut store = MemStore::new();
    let mut chain = MockChain::new(2);
    store
        .enqueue(mk_intent("i1", "q1", "s1", "owner-a", "batch-x", 250))
        .unwrap();
    {
        let mut w = worker(&mut store, &mut chain, &fx, &id, now);
        let r = w.redrive();
        assert!(matches!(&r[0].1, DriveResult::Retry { reason } if reason.contains("fx")), "got {:?}", r[0].1);
    }
    assert_eq!(chain.next_nonce(SETTLE_ACCT), 1, "no post on missing fx rate");
}

// ─────────────────────────────────────────────────────────────────────────────
// Extra: the posted entry pays the correct owner the correct hunits.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn posted_entry_pays_correct_owner_and_amount() {
    let now = 1000;
    let id = identity();
    let fx = fresh_book(now);
    let mut store = MemStore::new();

    // A capturing chain that records the exact submitted entry.
    struct CapturingChain {
        inner: MockChain,
        last: Option<LedgerEntry>,
    }
    impl ChainSubmit for CapturingChain {
        fn submit(&mut self, entry: &LedgerEntry, sig: &str) -> hone_settlement_bridge::SubmitOutcome {
            self.last = Some(entry.clone());
            self.inner.submit(entry, sig)
        }
        fn find_sealed(&self, buyer: &str, nonce: u64) -> Option<String> {
            self.inner.find_sealed(buyer, nonce)
        }
        fn current_epoch(&self) -> u64 {
            self.inner.current_epoch()
        }
        fn next_nonce(&self, account: &str) -> u64 {
            self.inner.next_nonce(account)
        }
    }

    let mut chain = CapturingChain { inner: MockChain::new(4), last: None };
    store
        .enqueue(mk_intent("i1", "q1", "sensor-42", "owner-zoe", "batch-abc", 250))
        .unwrap();
    {
        let mut w = SettlementWorker {
            store: &mut store,
            chain: &mut chain,
            fx: &fx,
            fx_policy: FxPolicy::default(),
            identity: &id,
            config: WorkerConfig { chain_id: CHAIN_ID.into(), loud_after_attempts: 5 },
            now,
        };
        let intent = store_get(&w.store, "i1");
        let r = w.post_settlement(&intent);
        assert!(matches!(r, DriveResult::Posted { .. }), "got {r:?}");
    }
    let entry = chain.last.expect("an entry was submitted");
    match entry {
        LedgerEntry::SensorDataPurchase { sensor_id, buyer, owner, batch_hash, fee, signed_by, nonce, epoch } => {
            assert_eq!(sensor_id, "sensor-42");
            assert_eq!(buyer, SETTLE_ACCT, "buyer must be the platform settlement account");
            assert_eq!(owner, "owner-zoe", "owner must be the sensor owner from the intent");
            assert_eq!(batch_hash, "batch-abc", "batch_hash carries the provenance discriminator");
            assert_eq!(fee, 50_000_000_000, "250 cents * 2e8 hunits/cent");
            assert_eq!(signed_by, SETTLE_ACCT, "only the settlement account signs");
            assert_eq!(nonce, 1);
            assert_eq!(epoch, 4);
        }
        other => panic!("expected SensorDataPurchase, got {other:?}"),
    }
}

/// Helper: fetch an intent (panics if missing) — keeps the borrow of `store`
/// short so we can pass it to `post_settlement` afterwards.
fn store_get(store: &&mut MemStore, id: &str) -> SettlementIntent {
    store.get(id).unwrap().unwrap()
}
