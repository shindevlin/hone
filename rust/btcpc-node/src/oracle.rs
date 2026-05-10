//! Oracle price feeds — commit-reveal aggregation with outlier reputation.
//!
//! Flow:
//!   1. Anyone creates a feed with OracleFeedCreate (defines asset pair + update interval).
//!   2. Registered reporters submit OracleReport entries within the commit window.
//!      Reports use a commit-reveal scheme: commit = sha256(value|nonce|reporter).
//!   3. At the deadline epoch, anyone triggers OracleFeedFinalize which:
//!      - Reveals all reports whose pre-images match the commit.
//!      - Computes the median of valid reports.
//!      - Penalises outliers (|report − median| > 5%) by decrementing reputation.
//!      - Records the finalised price in CF_META.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use tracing::info;

use btcpc_types::{LedgerEntry, NATIVE_TOKEN, RECYCLE_FUND_ACCOUNT};
use crate::chain::Chain;

const OUTLIER_THRESHOLD_BPS: u64 = 500; // 5%
const REPORTER_REWARD_DREAMS: u64 = 2_000;
const FEED_CREATION_FEE_DREAMS: u64 = 100_000; // 0.001 BTCPC
const MIN_REPORTS: usize = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OracleFeed {
    pub feed_id: String,
    pub creator: String,
    pub asset_pair: String, // e.g. "BTCPC/USD"
    pub update_interval_epochs: u64,
    pub created_epoch: u64,
    pub last_price: Option<u64>, // micro-USD (6 decimals)
    pub last_updated_epoch: Option<u64>,
    pub report_deadline: u64,
    pub reports: Vec<OracleCommit>,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OracleCommit {
    pub reporter: String,
    pub commit_hash: String, // sha256(value_u64_le | nonce | reporter)
    pub reveal_value: Option<u64>,
    pub reveal_nonce: Option<String>,
    pub epoch: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReporterReputation {
    pub reporter: String,
    pub score: i64, // starts at 100, penalised for outliers
    pub reports_total: u64,
    pub outliers: u64,
}

fn feed_key(feed_id: &str) -> String { format!("oracle_feed:{}", feed_id) }
fn rep_key(reporter: &str) -> String { format!("oracle_rep:{}", reporter) }

pub fn apply_create(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    let LedgerEntry::OracleFeedCreate {
        feed_id, creator, asset_pair, update_interval_epochs, epoch, ..
    } = entry else { bail!("wrong entry type") };

    if chain.store.state_get(&feed_key(feed_id)).is_some() {
        bail!("feed '{}' already exists", feed_id);
    }

    chain.store.debit(creator, NATIVE_TOKEN, FEED_CREATION_FEE_DREAMS)?;
    chain.store.credit(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, FEED_CREATION_FEE_DREAMS)?;

    let feed = OracleFeed {
        feed_id: feed_id.clone(),
        creator: creator.clone(),
        asset_pair: asset_pair.clone(),
        update_interval_epochs: *update_interval_epochs,
        created_epoch: *epoch,
        last_price: None,
        last_updated_epoch: None,
        report_deadline: epoch + update_interval_epochs,
        reports: vec![],
        active: true,
    };
    chain.store.state_set(&feed_key(feed_id), &serde_json::to_vec(&feed)?)?;
    info!("[oracle] feed '{}' created for {} by '{}'", feed_id, asset_pair, creator);
    Ok(())
}

pub fn apply_report(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    let LedgerEntry::OracleReport {
        feed_id, reporter, commit_hash, epoch, ..
    } = entry else { bail!("wrong entry type") };

    let raw = chain.store.state_get(&feed_key(feed_id))
        .ok_or_else(|| anyhow::anyhow!("feed '{}' not found", feed_id))?;
    let mut feed: OracleFeed = serde_json::from_slice(&raw)?;

    anyhow::ensure!(feed.active, "feed '{}' is inactive", feed_id);
    anyhow::ensure!(*epoch <= feed.report_deadline, "report window closed for '{}'", feed_id);

    if feed.reports.iter().any(|r| &r.reporter == reporter) {
        bail!("reporter '{}' already submitted for this round", reporter);
    }

    feed.reports.push(OracleCommit {
        reporter: reporter.clone(),
        commit_hash: commit_hash.clone(),
        reveal_value: None,
        reveal_nonce: None,
        epoch: *epoch,
    });
    chain.store.state_set(&feed_key(feed_id), &serde_json::to_vec(&feed)?)?;
    Ok(())
}

pub fn apply_finalize(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    let LedgerEntry::OracleFeedFinalize {
        feed_id, reveals, epoch, ..
    } = entry else { bail!("wrong entry type") };

    let raw = chain.store.state_get(&feed_key(feed_id))
        .ok_or_else(|| anyhow::anyhow!("feed '{}' not found", feed_id))?;
    let mut feed: OracleFeed = serde_json::from_slice(&raw)?;

    anyhow::ensure!(*epoch >= feed.report_deadline, "finalization window not open yet");
    anyhow::ensure!(feed.reports.len() >= MIN_REPORTS, "insufficient reports: need {}", MIN_REPORTS);

    // Verify reveals against commits.
    let mut valid_values: Vec<u64> = vec![];
    let reveal_map: std::collections::HashMap<&str, (&u64, &str)> = reveals.iter()
        .map(|r| (r.reporter.as_str(), (&r.value, r.nonce.as_str())))
        .collect();

    for commit in &mut feed.reports {
        if let Some((value, nonce)) = reveal_map.get(commit.reporter.as_str()) {
            let expected = {
                use sha2::{Digest, Sha256};
                let mut data = value.to_le_bytes().to_vec();
                data.extend_from_slice(nonce.as_bytes());
                data.extend_from_slice(commit.reporter.as_bytes());
                hex::encode(Sha256::digest(&data))
            };
            if expected == commit.commit_hash {
                commit.reveal_value = Some(**value);
                commit.reveal_nonce = Some(nonce.to_string());
                valid_values.push(**value);
            }
        }
    }

    if valid_values.len() < MIN_REPORTS {
        // Not enough reveals — advance deadline without updating price.
        feed.report_deadline = epoch + feed.update_interval_epochs;
        feed.reports.clear();
        chain.store.state_set(&feed_key(feed_id), &serde_json::to_vec(&feed)?)?;
        return Ok(());
    }

    valid_values.sort_unstable();
    let median = valid_values[valid_values.len() / 2];

    // Score reporters: reward accurate ones, penalise outliers.
    for commit in &feed.reports {
        let Some(val) = commit.reveal_value else { continue };
        let deviation = if val > median {
            (val - median) * 10_000 / median.max(1)
        } else {
            (median - val) * 10_000 / median.max(1)
        };

        let mut rep: ReporterReputation = chain.store.state_get(&rep_key(&commit.reporter))
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or(ReporterReputation {
                reporter: commit.reporter.clone(),
                score: 100,
                reports_total: 0,
                outliers: 0,
            });
        rep.reports_total += 1;

        if deviation > OUTLIER_THRESHOLD_BPS {
            rep.score = (rep.score - 5).max(0);
            rep.outliers += 1;
        } else {
            rep.score = (rep.score + 1).min(100);
            // Pay accurate reporters.
            let _ = chain.store.credit(&commit.reporter, NATIVE_TOKEN, REPORTER_REWARD_DREAMS);
        }
        let _ = chain.store.state_set(&rep_key(&commit.reporter), &serde_json::to_vec(&rep).unwrap_or_default());
    }

    feed.last_price = Some(median);
    feed.last_updated_epoch = Some(*epoch);
    feed.report_deadline = epoch + feed.update_interval_epochs;
    feed.reports.clear();
    chain.store.state_set(&feed_key(feed_id), &serde_json::to_vec(&feed)?)?;

    // Index latest price for quick lookup.
    chain.store.state_set(
        &format!("oracle_price:{}", feed.asset_pair),
        &serde_json::to_vec(&serde_json::json!({ "price": median, "epoch": epoch, "feed_id": feed_id })).unwrap_or_default(),
    )?;

    info!("[oracle] feed '{}' finalized — median={} ({})", feed_id, median, feed.asset_pair);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use btcpc_types::LedgerEntry;
    use tempfile::TempDir;

    fn make_chain() -> (crate::chain::Chain, TempDir) {
        let dir = tempfile::TempDir::new().unwrap();
        let store = crate::store::Store::open(dir.path()).unwrap();
        let chain = crate::chain::Chain::new(
            store, "test".to_string(), "btcpc-test".to_string(),
        );
        (chain, dir)
    }

    fn fund(chain: &crate::chain::Chain, account: &str, amount: u64) {
        chain.apply_entry(&LedgerEntry::AccountCreate {
            account: account.to_string(),
            keys: Default::default(),
            chain_proofs: vec![],
            epoch: 0,
            funded_by: None,
            machine_fingerprint: None,
        }).ok();
        chain.apply_entry(&LedgerEntry::GenesisAlloc {
            account: account.to_string(),
            amount,
            token: btcpc_types::NATIVE_TOKEN.to_string(),
        }).unwrap();
    }

    fn make_commit_hash(value: u64, nonce: &str, reporter: &str) -> String {
        use sha2::{Digest, Sha256};
        let mut data = value.to_le_bytes().to_vec();
        data.extend_from_slice(nonce.as_bytes());
        data.extend_from_slice(reporter.as_bytes());
        hex::encode(Sha256::digest(&data))
    }

    #[test]
    fn create_stores_feed() {
        let (chain, _dir) = make_chain();
        fund(&chain, "alice", FEED_CREATION_FEE_DREAMS * 10);

        let entry = LedgerEntry::OracleFeedCreate {
            feed_id: "feed1".to_string(),
            creator: "alice".to_string(),
            asset_pair: "BTCPC/USD".to_string(),
            update_interval_epochs: 5,
            epoch: 1,
            nonce: 1,
            signed_by: "alice".to_string(),
            signature: None,
        };
        apply_create(&chain, &entry).unwrap();

        let raw = chain.store.state_get(&feed_key("feed1")).unwrap();
        let feed: OracleFeed = serde_json::from_slice(&raw).unwrap();
        assert_eq!(feed.feed_id, "feed1");
        assert_eq!(feed.asset_pair, "BTCPC/USD");
        assert!(feed.active);
        assert_eq!(feed.reports.len(), 0);
    }

    #[test]
    fn report_stores_reading() {
        let (chain, _dir) = make_chain();
        fund(&chain, "alice", FEED_CREATION_FEE_DREAMS * 10);

        let create = LedgerEntry::OracleFeedCreate {
            feed_id: "feed2".to_string(),
            creator: "alice".to_string(),
            asset_pair: "BTCPC/USD".to_string(),
            update_interval_epochs: 10,
            epoch: 1,
            nonce: 1,
            signed_by: "alice".to_string(),
            signature: None,
        };
        apply_create(&chain, &create).unwrap();

        let report = LedgerEntry::OracleReport {
            feed_id: "feed2".to_string(),
            reporter: "bob".to_string(),
            commit_hash: make_commit_hash(100_000, "nonce1", "bob"),
            epoch: 2,
            nonce: 1,
            signed_by: "bob".to_string(),
            signature: None,
        };
        apply_report(&chain, &report).unwrap();

        let raw = chain.store.state_get(&feed_key("feed2")).unwrap();
        let feed: OracleFeed = serde_json::from_slice(&raw).unwrap();
        assert_eq!(feed.reports.len(), 1);
        assert_eq!(feed.reports[0].reporter, "bob");
    }

    #[test]
    fn finalize_updates_price() {
        let (chain, _dir) = make_chain();
        fund(&chain, "alice", FEED_CREATION_FEE_DREAMS * 10);
        // Also fund reporters for rewards
        fund(&chain, "r1", 0);
        fund(&chain, "r2", 0);
        fund(&chain, "r3", 0);

        // create feed: interval=5, deadline = epoch 1 + 5 = 6
        let create = LedgerEntry::OracleFeedCreate {
            feed_id: "feed3".to_string(),
            creator: "alice".to_string(),
            asset_pair: "BTCPC/USD".to_string(),
            update_interval_epochs: 5,
            epoch: 1,
            nonce: 1,
            signed_by: "alice".to_string(),
            signature: None,
        };
        apply_create(&chain, &create).unwrap();

        // Submit 3 reports (deadline = 6, report epoch ≤ 6)
        let reporters = [("r1", 100_000u64, "n1"), ("r2", 101_000u64, "n2"), ("r3", 99_000u64, "n3")];
        for (reporter, value, nonce) in &reporters {
            let r = LedgerEntry::OracleReport {
                feed_id: "feed3".to_string(),
                reporter: reporter.to_string(),
                commit_hash: make_commit_hash(*value, nonce, reporter),
                epoch: 5,
                nonce: 1,
                signed_by: reporter.to_string(),
                signature: None,
            };
            apply_report(&chain, &r).unwrap();
        }

        // Finalize at epoch ≥ deadline (6)
        let reveals: Vec<btcpc_types::OracleReveal> = reporters.iter()
            .map(|(reporter, value, nonce)| btcpc_types::OracleReveal {
                reporter: reporter.to_string(),
                value: *value,
                nonce: nonce.to_string(),
            })
            .collect();

        let finalize = LedgerEntry::OracleFeedFinalize {
            feed_id: "feed3".to_string(),
            finalizer: "alice".to_string(),
            reveals,
            epoch: 6,
            nonce: 1,
            signed_by: "alice".to_string(),
            signature: None,
        };
        apply_finalize(&chain, &finalize).unwrap();

        let raw = chain.store.state_get(&feed_key("feed3")).unwrap();
        let feed: OracleFeed = serde_json::from_slice(&raw).unwrap();
        assert!(feed.last_price.is_some(), "price should be set after finalize");
        assert_eq!(feed.last_updated_epoch, Some(6));
        assert!(feed.active, "feed should still be active after finalize");
    }

    #[test]
    fn report_after_deadline_fails() {
        let (chain, _dir) = make_chain();
        fund(&chain, "alice", FEED_CREATION_FEE_DREAMS * 10);

        // interval=3, deadline = epoch 1 + 3 = 4
        let create = LedgerEntry::OracleFeedCreate {
            feed_id: "feed4".to_string(),
            creator: "alice".to_string(),
            asset_pair: "BTCPC/USD".to_string(),
            update_interval_epochs: 3,
            epoch: 1,
            nonce: 1,
            signed_by: "alice".to_string(),
            signature: None,
        };
        apply_create(&chain, &create).unwrap();

        // Report at epoch > deadline (5 > 4)
        let report = LedgerEntry::OracleReport {
            feed_id: "feed4".to_string(),
            reporter: "bob".to_string(),
            commit_hash: make_commit_hash(100_000, "n1", "bob"),
            epoch: 5,
            nonce: 1,
            signed_by: "bob".to_string(),
            signature: None,
        };
        assert!(apply_report(&chain, &report).is_err(), "report after deadline should fail");
    }

    #[test]
    fn get_feed_returns_none_for_unknown() {
        let (chain, _dir) = make_chain();
        assert!(chain.store.state_get(&feed_key("unknown_feed")).is_none());
    }
}
