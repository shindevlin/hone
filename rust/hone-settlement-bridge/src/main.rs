//! `hone-settlement-bridge` binary.
//!
//! Runs the settlement worker's `redrive()` loop over a durable
//! `JsonFileStore`, re-driving every non-`Posted` intent until the sensor owner
//! is paid on-chain. The settlement-account key is read from env ONLY
//! (`HONE_SETTLEMENT_ACCOUNT`, `HONE_SETTLEMENT_SIGNING_KEY`) and never
//! persisted.
//!
//! The chain submission call is behind `ChainSubmit`. Wiring it to the real
//! `apply_and_broadcast` (`rust/hone-node/src/api.rs`) is a thin adapter,
//! pending resolution of the hone-node build ICE — so this binary intentionally
//! refuses to run against a real chain until that adapter is provided, rather
//! than silently posting through a mock. It prints the reconciliation report
//! and exits, which is safe and honest; the loop wiring is ready for the real
//! adapter.

use hone_settlement_bridge::{reconcile, JsonFileStore, SigningIdentity};

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let store_path = std::env::var("HONE_SETTLEMENT_STORE")
        .unwrap_or_else(|_| "settlement_intents.json".to_string());
    let store = JsonFileStore::open(&store_path)?;

    // Load (and validate) the settlement identity from env up front so a
    // misconfiguration fails loudly rather than at first post.
    match SigningIdentity::from_env() {
        Ok(id) => {
            tracing::info!(
                "settlement bridge: identity loaded for account '{}', store '{}'",
                id.account,
                store_path
            );
        }
        Err(e) => {
            tracing::warn!(
                "settlement identity not configured ({e}); reporting only. Set \
                 HONE_SETTLEMENT_ACCOUNT and HONE_SETTLEMENT_SIGNING_KEY to run the worker."
            );
        }
    }

    let r = reconcile(&store)?;
    tracing::info!(
        "reconciliation: {} intents ({} posted, {} open, {} quarantined); \
         posted {} hunits for {} USD-cents; open {} USD-cents",
        r.total_intents,
        r.posted,
        r.open,
        r.quarantined,
        r.hunits_posted,
        r.usd_cents_posted,
        r.usd_cents_open
    );

    tracing::info!(
        "chain adapter (apply_and_broadcast) not wired in this build (hone-node \
         build ICE); worker loop is ready — see crate docs. Exiting after report."
    );
    Ok(())
}
