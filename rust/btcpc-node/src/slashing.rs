//! Slashing framework for misbehaving validators and clock nodes.
//!
//! Any staked clock node can file a SlashValidator entry with evidence.
//! The slashed node's stake is split: 50% to the reporter, 50% to recycle.
//! Slashed accounts can file a SlashAppeal within 100 epochs; a 5-member panel
//! votes with 66% threshold to overturn.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use tracing::info;

use btcpc_types::{LedgerEntry, NATIVE_TOKEN, RECYCLE_FUND_ACCOUNT};
use crate::chain::Chain;

const APPEAL_WINDOW_EPOCHS: u64 = 100;
const APPEAL_PANEL_SIZE: usize = 5;
const OVERTURN_THRESHOLD_BPS: u64 = 6_600; // 66%
const REPORTER_SHARE_BPS: u64 = 5_000;     // 50% of slashed stake
const MIN_SLASH_STAKE: u64 = 1_000_000;    // 0.01 BTCPC minimum to slash

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlashRecord {
    pub slash_id: String,
    pub reporter: String,
    pub accused: String,
    pub evidence: String,
    pub violation: String,
    pub slash_epoch: u64,
    pub appeal_deadline: u64,
    pub slashed_amount: u64,
    pub status: SlashStatus,
    pub appeal_votes: Vec<AppealVote>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SlashStatus {
    Active,
    Appealed,
    Overturned,
    Upheld,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppealVote {
    pub panelist: String,
    pub overturn: bool,
    pub epoch: u64,
}

fn slash_key(slash_id: &str) -> String { format!("slash:{}", slash_id) }

pub fn apply_slash(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    let LedgerEntry::SlashValidator {
        slash_id, reporter, accused, evidence, violation, epoch, ..
    } = entry else { bail!("wrong entry type") };

    // Reporter must be a staked clock node.
    let reporter_stake = chain.store.state_get(&format!("clock_stake:{}", reporter))
        .and_then(|b| serde_json::from_slice::<u64>(&b).ok())
        .unwrap_or(0);
    if reporter_stake == 0 {
        bail!("reporter '{}' has no clock stake", reporter);
    }

    // Accused must have stake to slash.
    let accused_stake = chain.get_stake(accused);
    if accused_stake < MIN_SLASH_STAKE {
        bail!("accused '{}' stake {} is below minimum {}", accused, accused_stake, MIN_SLASH_STAKE);
    }

    // Prevent duplicate slash IDs.
    if chain.store.state_get(&slash_key(slash_id)).is_some() {
        bail!("slash '{}' already exists", slash_id);
    }

    // Execute slash: deduct from accused, split reporter/recycle.
    let slash_amount = accused_stake / 10; // 10% slash
    chain.store.debit(accused, NATIVE_TOKEN, slash_amount)?;
    let reporter_cut = slash_amount * REPORTER_SHARE_BPS / 10_000;
    let recycle_cut  = slash_amount.saturating_sub(reporter_cut);
    chain.store.credit(reporter, NATIVE_TOKEN, reporter_cut)?;
    if recycle_cut > 0 {
        chain.store.credit(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, recycle_cut)?;
    }

    let record = SlashRecord {
        slash_id: slash_id.clone(),
        reporter: reporter.clone(),
        accused: accused.clone(),
        evidence: evidence.clone(),
        violation: violation.clone(),
        slash_epoch: *epoch,
        appeal_deadline: epoch + APPEAL_WINDOW_EPOCHS,
        slashed_amount: slash_amount,
        status: SlashStatus::Active,
        appeal_votes: vec![],
    };
    chain.store.state_set(&slash_key(slash_id), &serde_json::to_vec(&record)?)?;
    info!("[slash] {} slashed {} dreams from '{}' (reporter: '{}')",
        slash_id, slash_amount, accused, reporter);
    Ok(())
}

pub fn apply_appeal(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    let LedgerEntry::SlashAppeal {
        slash_id, panelist, overturn, epoch, ..
    } = entry else { bail!("wrong entry type") };

    let raw = chain.store.state_get(&slash_key(slash_id))
        .ok_or_else(|| anyhow::anyhow!("slash '{}' not found", slash_id))?;
    let mut record: SlashRecord = serde_json::from_slice(&raw)?;

    if record.status != SlashStatus::Active && record.status != SlashStatus::Appealed {
        bail!("slash '{}' is already finalized", slash_id);
    }
    if *epoch > record.appeal_deadline {
        bail!("appeal window closed for slash '{}'", slash_id);
    }
    if record.appeal_votes.iter().any(|v| &v.panelist == panelist) {
        bail!("panelist '{}' already voted on appeal '{}'", panelist, slash_id);
    }

    record.status = SlashStatus::Appealed;
    record.appeal_votes.push(AppealVote {
        panelist: panelist.clone(),
        overturn: *overturn,
        epoch: *epoch,
    });

    // Check if we have a panel quorum.
    if record.appeal_votes.len() >= APPEAL_PANEL_SIZE {
        let overturn_votes = record.appeal_votes.iter().filter(|v| v.overturn).count();
        let bps = overturn_votes as u64 * 10_000 / APPEAL_PANEL_SIZE as u64;
        if bps >= OVERTURN_THRESHOLD_BPS {
            // Overturn: return slashed amount to accused.
            let _ = chain.store.credit(&record.accused, NATIVE_TOKEN, record.slashed_amount);
            // Claw back reporter reward.
            let reporter_cut = record.slashed_amount * REPORTER_SHARE_BPS / 10_000;
            let _ = chain.store.debit(&record.reporter, NATIVE_TOKEN, reporter_cut);
            let _ = chain.store.credit(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, reporter_cut);
            record.status = SlashStatus::Overturned;
            info!("[slash] appeal {} overturned — refunded {} to '{}'",
                slash_id, record.slashed_amount, record.accused);
        } else {
            record.status = SlashStatus::Upheld;
            info!("[slash] appeal {} upheld — slash stands", slash_id);
        }
    }

    chain.store.state_set(&slash_key(slash_id), &serde_json::to_vec(&record)?)?;
    Ok(())
}
