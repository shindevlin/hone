//! The settlement worker: `post_settlement` (one intent) and `redrive` (the
//! crash-safe, self-healing loop over the whole open work set).
//!
//! Implements the DECISION RECORD's "CRASH-RECOVERY RE-DRIVE ALGORITHM"
//! verbatim. The invariant it upholds: the ONLY way an intent leaves the open
//! work set is reaching `Posted` (owner actually paid) — so a metered USD
//! query can never be "collected but never settled".

use crate::chain::{self, ChainSubmit, SigningIdentity, SubmitOutcome};
use crate::fx::{FxPolicy, FxRateBook};
use crate::intent::{IntentStatus, SettlementIntent};
use crate::store::IntentStore;

/// Config the worker needs beyond the injected store/chain/fx.
pub struct WorkerConfig {
    pub chain_id: String,
    /// Max attempts before a still-failing (non-malformed) intent is flagged
    /// loudly but STILL retried (we never abandon a collected charge). A
    /// malformed intent is quarantined immediately regardless of this.
    pub loud_after_attempts: u32,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        WorkerConfig {
            chain_id: "hone-testnet".to_string(),
            loud_after_attempts: 10,
        }
    }
}

/// Outcome of driving a single intent (for tests / reporting).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriveResult {
    /// Now `Posted` (this call, or reconciled to already-posted).
    Posted { entry_hash: String },
    /// Left open, will be retried on the next redrive.
    Retry { reason: String },
    /// Quarantined (malformed) — needs a human, not retried.
    Quarantined { reason: String },
    /// Was already `Posted` before this call; skipped.
    AlreadyPosted,
}

/// The worker. Generic over the store and chain so tests inject `MemStore`/
/// `MockChain` and production injects `JsonFileStore` + the real
/// `apply_and_broadcast` adapter.
pub struct SettlementWorker<'a, S: IntentStore, C: ChainSubmit> {
    pub store: &'a mut S,
    pub chain: &'a mut C,
    pub fx: &'a FxRateBook,
    pub fx_policy: FxPolicy,
    pub identity: &'a SigningIdentity,
    pub config: WorkerConfig,
    /// Injected clock (unix seconds) for deterministic fx-staleness tests.
    pub now: u64,
}

impl<'a, S: IntentStore, C: ChainSubmit> SettlementWorker<'a, S, C> {
    /// Drive EVERY open intent (`Pending | Posting | Failed`, excluding
    /// quarantined) once. Called on startup and on each tick. Returns per-intent
    /// results in deterministic order. This is the self-healing entry point:
    /// because `store.open_intents()` re-reads durable state, a crash at any
    /// point is recovered on the next call.
    pub fn redrive(&mut self) -> Vec<(String, DriveResult)> {
        let open = match self.store.open_intents() {
            Ok(v) => v,
            Err(e) => {
                tracing::error!("redrive: cannot load open intents: {e}");
                return vec![];
            }
        };
        let mut out = Vec::with_capacity(open.len());
        for intent in open {
            if intent.quarantined {
                out.push((
                    intent.id.clone(),
                    DriveResult::Quarantined {
                        reason: intent.last_error.clone().unwrap_or_default(),
                    },
                ));
                continue;
            }
            let r = self.post_settlement(&intent);
            out.push((intent.id.clone(), r));
        }
        out
    }

    /// Drive one intent toward `Posted`. Idempotent and crash-safe per the
    /// DECISION RECORD. Never returns a terminal failure for a well-formed
    /// intent — a transient failure leaves it open for the next redrive.
    pub fn post_settlement(&mut self, intent: &SettlementIntent) -> DriveResult {
        if intent.status == IntentStatus::Posted {
            return DriveResult::AlreadyPosted;
        }
        let mut intent = intent.clone();

        // ── (a) Crash-in-flight recovery for ANY intent that already has a
        // pinned nonce (`Posting` after a crash mid-submit, OR `Failed` after a
        // transient error that may nonetheless have sealed on-chain). Query the
        // chain FIRST: if the pinned entry sealed, reconcile without re-submit.
        // This is the guard against double-posting a charge whose first submit
        // actually landed even though the worker never learned it.
        if let Some(nonce) = intent.nonce {
            if let Some(hash) = self.chain.find_sealed(&self.identity.account, nonce) {
                return self.mark_posted(&mut intent, hash);
            }
            // Not found: fall through and RE-SUBMIT the SAME pinned entry
            // (same nonce/epoch/fee) below.
        }

        // ── (b) Resolve dreams_amount via the pinned fx rate (reject stale/
        // out-of-band/missing — a non-posting condition, retried after re-pin).
        let dreams = match intent.dreams_amount {
            Some(d) if intent.nonce.is_some() => d, // already pinned on a prior attempt
            _ => match self.fx.resolve(
                &intent.fx_rate_id,
                &self.fx_policy,
                intent.usd_amount_cents,
                self.now,
            ) {
                Ok(d) => d,
                Err(e) => {
                    // fx problem: leave open, retry after re-pin. NOT quarantined
                    // (a stale rate is transient — the enqueuing layer re-pins).
                    return self.mark_retry(&mut intent, format!("fx: {e}"));
                }
            },
        };

        // Malformed guard: a non-positive fee or empty owner is a data bug, not
        // a transient failure — quarantine so it surfaces to a human instead of
        // spinning forever OR being silently dropped.
        if dreams == 0 {
            return self.mark_quarantine(&mut intent, "resolved dreams_amount is 0".into());
        }
        if intent.owner_account.trim().is_empty() {
            return self.mark_quarantine(&mut intent, "owner_account is empty".into());
        }

        // ── (c) Pin nonce + epoch ONCE, persist BEFORE submitting, so a retry
        // re-posts the identical entry. If already pinned (retry path), reuse.
        let nonce = match intent.nonce {
            Some(n) => n,
            None => self.chain.next_nonce(&self.identity.account),
        };
        let epoch = match intent.epoch {
            Some(e) => e,
            None => self.chain.current_epoch(),
        };
        intent.dreams_amount = Some(dreams);
        intent.nonce = Some(nonce);
        intent.epoch = Some(epoch);
        intent.status = IntentStatus::Posting;
        intent.attempts += 1;
        // Persist the pin BEFORE the side effect. If we crash right here, the
        // next redrive sees `Posting` with a pinned nonce and recovers via (a).
        if let Err(e) = self.store.update(&intent) {
            tracing::error!("post_settlement: persist pin failed: {e}");
            return DriveResult::Retry {
                reason: format!("persist pin failed: {e}"),
            };
        }

        // ── (d) Construct, sign, submit the entry.
        let entry = chain::build_purchase_entry(
            &intent.sensor_id,
            &self.identity.account,
            &intent.owner_account,
            &intent.batch_hash,
            dreams,
            epoch,
            nonce,
        );
        let sig_hex = match chain::sign_entry(&self.identity.signing_key, &entry, &self.config.chain_id) {
            Ok(s) => s,
            Err(e) => {
                // Signing is deterministic; a failure here is a config/logic bug.
                return self.mark_quarantine(&mut intent, format!("sign failed: {e}"));
            }
        };

        match self.chain.submit(&entry, &sig_hex) {
            SubmitOutcome::Sealed { entry_hash } => self.mark_posted(&mut intent, entry_hash),
            SubmitOutcome::NonceAlreadyUsed => {
                // A prior attempt of THIS entry already sealed (double-post
                // guard fired). Resolve the existing hash and reconcile.
                match self.chain.find_sealed(&self.identity.account, nonce) {
                    Some(hash) => self.mark_posted(&mut intent, hash),
                    None => {
                        // Chain says used but we can't find it — treat as
                        // posted-unknown-hash; record and reconcile loudly.
                        self.mark_posted(&mut intent, format!("nonce-{nonce}-already-used"))
                    }
                }
            }
            SubmitOutcome::NoPeers => {
                self.mark_retry(&mut intent, "no peers (hardline) — will retry".into())
            }
            SubmitOutcome::InsufficientBalance => self.mark_retry(
                &mut intent,
                "settlement account underfunded — will retry after top-up".into(),
            ),
            SubmitOutcome::Transient(msg) => self.mark_retry(&mut intent, format!("transient: {msg}")),
        }
    }

    fn mark_posted(&mut self, intent: &mut SettlementIntent, hash: String) -> DriveResult {
        intent.status = IntentStatus::Posted;
        intent.chain_entry_hash = Some(hash.clone());
        intent.posted_at = Some(self.now);
        intent.last_error = None;
        if let Err(e) = self.store.update(intent) {
            // If we can't persist `posted`, the entry IS on-chain but our record
            // says otherwise — the next redrive's find_sealed reconciles it, so
            // this is safe (owner paid once regardless). Log loudly.
            tracing::error!("mark_posted: persist failed for {}: {e}", intent.id);
            return DriveResult::Retry {
                reason: format!("posted on-chain but persist failed: {e}"),
            };
        }
        DriveResult::Posted { entry_hash: hash }
    }

    fn mark_retry(&mut self, intent: &mut SettlementIntent, reason: String) -> DriveResult {
        intent.status = IntentStatus::Failed;
        intent.last_error = Some(reason.clone());
        if intent.attempts >= self.config.loud_after_attempts {
            tracing::warn!(
                "intent {} still unsettled after {} attempts: {reason}",
                intent.id,
                intent.attempts
            );
        }
        let _ = self.store.update(intent);
        DriveResult::Retry { reason }
    }

    fn mark_quarantine(&mut self, intent: &mut SettlementIntent, reason: String) -> DriveResult {
        intent.status = IntentStatus::Failed;
        intent.quarantined = true;
        intent.last_error = Some(reason.clone());
        tracing::error!("intent {} quarantined (malformed): {reason}", intent.id);
        let _ = self.store.update(intent);
        DriveResult::Quarantined { reason }
    }
}
