//! Inference marketplace daemon — auto-awards pending jobs, expires stale ones,
//! and triggers payment after verification.

use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};
use btcpc_types::{LedgerEntry, CLOCK_REWARD_DREAMS, epoch_duration_ms};

use crate::chain::Chain;
use crate::inference::{self, JobStatus};

pub async fn run(chain: Arc<Chain>) {
    let mut last_processed_epoch: u64 = 0;

    loop {
        let current_epoch = chain.current_epoch();
        if current_epoch > last_processed_epoch {
            last_processed_epoch = current_epoch;
            process_epoch(&chain, current_epoch);
        }
        // Check approximately every half-epoch (era 0: ~15s).
        let interval = epoch_duration_ms(current_epoch) / 2;
        tokio::time::sleep(Duration::from_millis(interval.max(5_000))).await;
    }
}

/// Grace period (epochs) before a completed-but-unverified job auto-settles.
const VERIFICATION_GRACE_EPOCHS: u64 = 20;

fn process_epoch(chain: &Chain, current_epoch: u64) {
    award_pending_jobs(chain, current_epoch);
    expire_stale_claims(chain, current_epoch);
    expire_stale_jobs(chain, current_epoch);
    pay_verified_jobs(chain, current_epoch);
    pay_resolved_disputes(chain, current_epoch);
    auto_settle_unverified(chain, current_epoch);
}

/// Award jobs whose bid window has closed.
fn award_pending_jobs(chain: &Chain, current_epoch: u64) {
    for job in inference::jobs_ready_to_award(chain, current_epoch) {
        let bids = inference::get_bids(chain, &job.job_id);

        // Award best worker bid.
        if let Some(worker_bid) = inference::select_best_bid(&bids, chain, "worker") {
            let award = LedgerEntry::InferenceJobAward {
                job_id: job.job_id.clone(),
                winner: worker_bid.bidder.clone(),
                role: "worker".to_owned(),
                fee: worker_bid.fee,
                epoch: current_epoch,
            };
            if let Err(e) = chain.apply_entry(&award) {
                warn!("failed to award worker for job {}: {}", job.job_id, e);
                continue;
            }

            // Award up to 3 verifier bids.
            let verifier_bids: Vec<_> = bids.iter()
                .filter(|b| b.role == "verifier")
                .take(3)
                .collect();
            for vbid in verifier_bids {
                let vaward = LedgerEntry::InferenceJobAward {
                    job_id: job.job_id.clone(),
                    winner: vbid.bidder.clone(),
                    role: "verifier".to_owned(),
                    fee: vbid.fee,
                    epoch: current_epoch,
                };
                if let Err(e) = chain.apply_entry(&vaward) {
                    warn!("failed to award verifier {} for job {}: {}", vbid.bidder, job.job_id, e);
                }
            }
            info!("inference job {} awarded to {}", job.job_id, worker_bid.bidder);
        } else {
            // No bids — cancel and refund.
            let cancel = LedgerEntry::InferenceJobCancel {
                job_id: job.job_id.clone(),
                cancelled_by: "system".to_owned(),
                reason: "no bids received".to_owned(),
                epoch: current_epoch,
                nonce: 0,
                signed_by: "system".to_owned(),
            };
            if let Err(e) = chain.apply_entry(&cancel) {
                warn!("failed to cancel no-bid job {}: {}", job.job_id, e);
            }
        }
    }
}

/// Transition Disputed jobs to NoFee when claim window expires with no claim.
fn expire_stale_claims(chain: &Chain, current_epoch: u64) {
    for job in inference::jobs_claim_expired(chain, current_epoch) {
        let verifiers = job.verifiers.clone();
        let pay = inference::build_pay_entry_nofee(&job, &verifiers, &[], current_epoch);
        if let Err(e) = chain.apply_entry(&pay) {
            warn!("nofee pay failed for job {}: {}", job.job_id, e);
        } else {
            info!("inference job {} expired without claim — worker gets no fee", job.job_id);
        }
    }
}

/// Cancel Posted/Awarded jobs that have passed their deadline.
fn expire_stale_jobs(chain: &Chain, current_epoch: u64) {
    for job in inference::jobs_past_deadline(chain, current_epoch) {
        let cancel = LedgerEntry::InferenceJobCancel {
            job_id: job.job_id.clone(),
            cancelled_by: "system".to_owned(),
            reason: "deadline exceeded".to_owned(),
            epoch: current_epoch,
            nonce: 0,
            signed_by: "system".to_owned(),
        };
        if let Err(e) = chain.apply_entry(&cancel) {
            warn!("failed to expire job {}: {}", job.job_id, e);
        }
    }
}

/// Pay Verified jobs (happy path: verifier approved, no dispute).
fn pay_verified_jobs(chain: &Chain, current_epoch: u64) {
    let verified_jobs: Vec<_> = chain.store.state_scan_prefix("infer_job:")
        .into_iter()
        .filter_map(|(_, v)| serde_json::from_slice::<inference::JobState>(&v).ok())
        .filter(|j| j.status == JobStatus::Verified)
        .collect();

    for job in verified_jobs {
        let verifiers = job.verifiers.clone();
        if let Some(pay) = inference::build_pay_entry_happy(&job, &verifiers, current_epoch) {
            if let Err(e) = chain.apply_entry(&pay) {
                warn!("happy-path pay failed for job {}: {}", job.job_id, e);
            } else {
                info!("inference job {} paid (verified)", job.job_id);
            }
        }
    }
}

/// Pay Reviewed jobs (dispute resolved by human reviewers).
fn pay_resolved_disputes(chain: &Chain, current_epoch: u64) {
    let resolved_jobs: Vec<_> = chain.store.state_scan_prefix("infer_job:")
        .into_iter()
        .filter_map(|(_, v)| serde_json::from_slice::<inference::JobState>(&v).ok())
        .filter(|j| matches!(j.status, JobStatus::Reviewed | JobStatus::Rejected))
        .collect();

    for job in resolved_jobs {
        let verifiers = job.verifiers.clone();
        let votes = inference::get_votes(chain, &job.job_id);
        let reviewers: Vec<String> = votes.iter().map(|v| v.reviewer.clone()).collect();

        let pay = if job.status == JobStatus::Reviewed {
            inference::build_pay_entry_disputed(&job, &verifiers, &reviewers, current_epoch)
        } else {
            // Rejected — worker gets nothing.
            Some(inference::build_pay_entry_nofee(&job, &verifiers, &reviewers, current_epoch))
        };

        if let Some(pay_entry) = pay {
            if let Err(e) = chain.apply_entry(&pay_entry) {
                warn!("dispute pay failed for job {}: {}", job.job_id, e);
            } else {
                info!("inference job {} paid (dispute resolved: {:?})", job.job_id, job.status);
            }
        }
    }
}

/// Auto-settle Completed jobs that received no verification within VERIFICATION_GRACE_EPOCHS.
/// Worker still gets paid; verifier pool is recycled.
fn auto_settle_unverified(chain: &Chain, current_epoch: u64) {
    let unverified: Vec<_> = chain.store.state_scan_prefix("infer_job:")
        .into_iter()
        .filter_map(|(_, v)| serde_json::from_slice::<inference::JobState>(&v).ok())
        .filter(|j| {
            j.status == inference::JobStatus::Completed
                && j.verifiers.is_empty()
                && j.latency_ms.is_some() // must have actually completed
        })
        .collect();

    for job in unverified {
        // Grace period: VERIFICATION_GRACE_EPOCHS after the deadline.
        if current_epoch < job.deadline_epoch + VERIFICATION_GRACE_EPOCHS {
            continue;
        }
        let verifiers = job.verifiers.clone();
        if let Some(pay) = inference::build_pay_entry_happy(&job, &verifiers, current_epoch) {
            if let Err(e) = chain.apply_entry(&pay) {
                warn!("auto-settle pay failed for job {}: {}", job.job_id, e);
            } else {
                info!("inference job {} auto-settled (no verifiers in grace period)", job.job_id);
            }
        }
    }
}

/// Distribute the total clock reward for an epoch equally among participating clock nodes.
/// Called from the clock seal task after quorum is reached.
#[allow(dead_code)]
pub fn distribute_clock_rewards(chain: &Chain, clock_nodes: &[String], epoch: u64) {
    if clock_nodes.is_empty() {
        return;
    }
    let per_node = CLOCK_REWARD_DREAMS / clock_nodes.len() as u64;
    for node in clock_nodes {
        let reward = LedgerEntry::ClockReward {
            node_id: node.clone(),
            amount: per_node,
            epoch,
        };
        if let Err(e) = chain.apply_entry(&reward) {
            warn!("clock reward failed for {}: {}", node, e);
        }
    }
}
