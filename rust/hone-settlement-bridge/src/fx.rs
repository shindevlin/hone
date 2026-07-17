//! USD -> HONE fx rate pinning and anti-manipulation.
//!
//! Design (PLATFORM_PRD.md 1.4 DECISION RECORD, "FX-RATE PINNING /
//! ANTI-MANIPULATION RULE"):
//! - The rate is PINNED at intent creation (captured as an `FxRate` behind an
//!   `FxRateId`), never re-derived at post time.
//! - Staleness is REJECTED: if `now - as_of > max_age`, the intent is not
//!   posted at the stale rate; it must be re-pinned.
//! - A sanity band rejects an out-of-plausible-bounds rate so a single
//!   compromised/fat-fingered oracle value cannot mint an absurd payout.
//! - A missing / zero / negative rate is a hard, non-posting error.
//!
//! The oracle itself (a live price feed) is OUT of scope; this module defines
//! the interface and the pinning rules. Tests inject a fixed rate and a stale
//! rate to exercise both paths.

use crate::intent::FxRateId;
use hone_types::HUNITS_PER_HONE;
use serde::{Deserialize, Serialize};

/// Errors resolving `usd_amount_cents` -> hunits through a pinned rate.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FxError {
    #[error("fx rate '{0}' not found (pinned rate missing)")]
    NotFound(String),
    #[error("fx rate '{id}' is stale: age {age_secs}s exceeds max_age {max_age_secs}s")]
    Stale {
        id: String,
        age_secs: u64,
        max_age_secs: u64,
    },
    #[error("fx rate '{id}' out of sanity band: {hunits_per_usd_cent} hunits/cent not in [{min}, {max}]")]
    OutOfBand {
        id: String,
        hunits_per_usd_cent: u128,
        min: u128,
        max: u128,
    },
    #[error("fx rate '{0}' is non-positive (zero/invalid) — refusing to post")]
    NonPositive(String),
}

/// A pinned USD->HONE rate. Expressed as `hunits_per_usd_cent` (integer, to
/// stay deterministic — no floats in a value that decides payouts). E.g. if
/// 1 HONE = $0.50, then $0.01 = 0.02 HONE = 200_000_000 hunits/cent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FxRate {
    pub id: FxRateId,
    /// Integer hunits paid per one USD cent at this pinned rate.
    pub hunits_per_usd_cent: u128,
    /// Unix seconds when this rate was captured.
    pub as_of: u64,
    /// Human-readable source (oracle name / feed id), for audit.
    pub source: String,
}

/// Sanity band + staleness policy applied to a pinned rate at post time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FxPolicy {
    /// Maximum age (seconds) a pinned rate may have at post time.
    pub max_age_secs: u64,
    /// Lower bound on `hunits_per_usd_cent` (reject below).
    pub min_hunits_per_usd_cent: u128,
    /// Upper bound on `hunits_per_usd_cent` (reject above).
    pub max_hunits_per_usd_cent: u128,
}

impl Default for FxPolicy {
    fn default() -> Self {
        // Plausible band: HONE priced anywhere from ~$0.001 to ~$1000 per HONE.
        // hunits/cent = HUNITS_PER_HONE * (cents_per_HONE)^-1 ... expressed
        // directly as bounds. At $0.001/HONE: $0.01 buys 10 HONE =
        // 100_000_000_000 hunits/cent. At $1000/HONE: $0.01 buys 1e-5 HONE =
        // 100_000 hunits/cent. Band chosen wide but finite so an absurd oracle
        // value (0 or u128::MAX) is rejected.
        let hunits = HUNITS_PER_HONE as u128;
        FxPolicy {
            max_age_secs: 15 * 60, // 15 minutes
            min_hunits_per_usd_cent: hunits / 100_000, // ~ $1000/HONE floor
            max_hunits_per_usd_cent: hunits * 1_000,   // ~ $0.001/HONE ceiling
        }
    }
}

/// Resolve `usd_amount_cents` into hunits through a pinned rate, applying the
/// staleness + sanity policy. `now` is unix seconds (injected for
/// determinism/testability). Returns the hunits payout or an `FxError` that
/// the worker treats as a non-posting condition.
pub fn resolve_hunits(
    rate: &FxRate,
    policy: &FxPolicy,
    usd_amount_cents: u64,
    now: u64,
) -> Result<u64, FxError> {
    if rate.hunits_per_usd_cent == 0 {
        return Err(FxError::NonPositive(rate.id.0.clone()));
    }
    // Staleness: reject a rate older than max_age. `saturating_sub` so a rate
    // pinned slightly in the future (clock skew) reads as age 0, not underflow.
    let age = now.saturating_sub(rate.as_of);
    if age > policy.max_age_secs {
        return Err(FxError::Stale {
            id: rate.id.0.clone(),
            age_secs: age,
            max_age_secs: policy.max_age_secs,
        });
    }
    // Sanity band.
    if rate.hunits_per_usd_cent < policy.min_hunits_per_usd_cent
        || rate.hunits_per_usd_cent > policy.max_hunits_per_usd_cent
    {
        return Err(FxError::OutOfBand {
            id: rate.id.0.clone(),
            hunits_per_usd_cent: rate.hunits_per_usd_cent,
            min: policy.min_hunits_per_usd_cent,
            max: policy.max_hunits_per_usd_cent,
        });
    }
    // Convert. u128 math to avoid overflow, then checked-narrow to u64 (a
    // payout that does not fit u64 hunits is itself out-of-band and rejected).
    let hunits = rate.hunits_per_usd_cent * usd_amount_cents as u128;
    u64::try_from(hunits).map_err(|_| FxError::OutOfBand {
        id: rate.id.0.clone(),
        hunits_per_usd_cent: rate.hunits_per_usd_cent,
        min: policy.min_hunits_per_usd_cent,
        max: policy.max_hunits_per_usd_cent,
    })
}

/// A trivial in-memory pinned-rate registry. In production the pinned rate
/// travels on the intent's `fx_rate_id` and is looked up from the B2B service
/// store; here we model that lookup behind a simple map so the worker can
/// resolve a rate by id.
#[derive(Debug, Default, Clone)]
pub struct FxRateBook {
    rates: std::collections::HashMap<String, FxRate>,
}

impl FxRateBook {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn pin(&mut self, rate: FxRate) {
        self.rates.insert(rate.id.0.clone(), rate);
    }
    pub fn get(&self, id: &FxRateId) -> Option<&FxRate> {
        self.rates.get(&id.0)
    }
    /// Resolve through a pinned rate by id, or `NotFound`.
    pub fn resolve(
        &self,
        id: &FxRateId,
        policy: &FxPolicy,
        usd_amount_cents: u64,
        now: u64,
    ) -> Result<u64, FxError> {
        match self.get(id) {
            Some(rate) => resolve_hunits(rate, policy, usd_amount_cents, now),
            None => Err(FxError::NotFound(id.0.clone())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rate(hpc: u128, as_of: u64) -> FxRate {
        FxRate {
            id: FxRateId("r1".into()),
            hunits_per_usd_cent: hpc,
            as_of,
            source: "test-oracle".into(),
        }
    }

    #[test]
    fn resolves_within_band_and_fresh() {
        // 1 HONE = $0.50 => $0.01 = 0.02 HONE = 200_000_000 hunits/cent.
        let hpc = (HUNITS_PER_HONE as u128) * 2 / 100; // 200_000_000
        let r = rate(hpc, 1000);
        // 250 cents ($2.50) => 250 * 200_000_000 = 50_000_000_000 hunits (5 HONE)
        let got = resolve_hunits(&r, &FxPolicy::default(), 250, 1000).unwrap();
        assert_eq!(got, 50_000_000_000);
    }

    #[test]
    fn stale_rate_is_rejected() {
        let hpc = (HUNITS_PER_HONE as u128) * 2 / 100;
        let r = rate(hpc, 0);
        let policy = FxPolicy::default();
        // now far beyond max_age
        let err = resolve_hunits(&r, &policy, 100, policy.max_age_secs + 1).unwrap_err();
        assert!(matches!(err, FxError::Stale { .. }), "got {err:?}");
    }

    #[test]
    fn zero_rate_is_rejected() {
        let r = rate(0, 1000);
        let err = resolve_hunits(&r, &FxPolicy::default(), 100, 1000).unwrap_err();
        assert!(matches!(err, FxError::NonPositive(_)), "got {err:?}");
    }

    #[test]
    fn out_of_band_rate_is_rejected() {
        // Absurdly high hunits/cent (above ceiling).
        let policy = FxPolicy::default();
        let r = rate(policy.max_hunits_per_usd_cent + 1, 1000);
        let err = resolve_hunits(&r, &policy, 1, 1000).unwrap_err();
        assert!(matches!(err, FxError::OutOfBand { .. }), "got {err:?}");
    }

    #[test]
    fn missing_pinned_rate_is_not_found() {
        let book = FxRateBook::new();
        let err = book
            .resolve(&FxRateId("nope".into()), &FxPolicy::default(), 100, 1000)
            .unwrap_err();
        assert!(matches!(err, FxError::NotFound(_)), "got {err:?}");
    }
}
