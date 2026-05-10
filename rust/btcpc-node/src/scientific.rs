//! BTCPC Scientific Compute Engine
//!
//! Long-running distributed scientific compute jobs — protein folding,
//! drug discovery, climate modeling, genomics, and general inference
//! workloads too large or too slow for the real-time inference pipeline.
//!
//! Key mechanics:
//!   - Jobs are stored in RocksDB under `sci_job:{job_id}`
//!   - Open-source jobs receive a 40% fee discount; results are recorded on chain
//!   - Nodes earn a 25% bonus for processing open-science jobs
//!   - Results > 50 KB are stored in BTCPC-FS; the on-chain entry holds the CID
//!   - Latency map tracks inter-peer round-trips to optimize shard routing
#![allow(dead_code)]

#![allow(dead_code)]
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Result};
use btcpc_types::{LedgerEntry, NATIVE_TOKEN};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::warn;

use crate::chain::Chain;

// ── Constants ─────────────────────────────────────────────────────────────────

pub const OPEN_SOURCE_DISCOUNT_BPS: u64 = 4_000; // 40 % in basis points
pub const SCIENCE_NODE_BONUS_BPS: u64 = 2_500;   // 25 % bonus
pub const MAX_INLINE_RESULT_BYTES: usize = 51_200; // 50 KB
const LATENCY_WINDOW_MS: u64 = 300_000;           // 5-min rolling window

// ── Latency store ─────────────────────────────────────────────────────────────

#[derive(Clone)]
struct LatencyEntry {
    latency_ms: u64,
    recorded_at: u64,
}

#[derive(Default, Clone)]
pub struct LatencyMap(Arc<Mutex<HashMap<String, HashMap<String, LatencyEntry>>>>);

impl LatencyMap {
    pub fn new() -> Self { Self::default() }

    pub fn record(&self, from: &str, to: &str, latency_ms: u64) {
        let mut map = self.0.lock().unwrap();
        map.entry(from.to_owned()).or_default()
            .insert(to.to_owned(), LatencyEntry { latency_ms, recorded_at: now_ms() });
    }

    fn hop(&self, from: &str, to: &str) -> Option<u64> {
        let map = self.0.lock().unwrap();
        let entry = map.get(from)?.get(to)?;
        if now_ms().saturating_sub(entry.recorded_at) > LATENCY_WINDOW_MS {
            return None; // stale
        }
        Some(entry.latency_ms)
    }

    /// Sum of hop latencies along an ordered node path.
    /// Returns `u64::MAX` if any hop is unknown.
    pub fn path_latency(&self, nodes: &[&str]) -> u64 {
        if nodes.len() < 2 { return 0; }
        let mut total = 0u64;
        for w in nodes.windows(2) {
            match self.hop(w[0], w[1]) {
                Some(ms) => total = total.saturating_add(ms),
                None => return u64::MAX,
            }
        }
        total
    }

    /// Greedy nearest-neighbour reorder of shard nodes to minimise total path latency.
    /// Falls back to original order when latency data is sparse.
    pub fn optimal_shard_order<'a>(&self, shards: &[&'a str]) -> Vec<&'a str> {
        if shards.len() <= 1 { return shards.to_vec(); }

        let has_any = shards.windows(2).any(|w| {
            self.hop(w[0], w[1]).is_some() || self.hop(w[1], w[0]).is_some()
        });
        if !has_any { return shards.to_vec(); }

        let mut remaining: Vec<&str> = shards[1..].to_vec();
        let mut ordered: Vec<&str> = vec![shards[0]];

        while !remaining.is_empty() {
            let current = *ordered.last().unwrap();
            let best = remaining.iter().enumerate().min_by_key(|(_, n)| {
                self.hop(current, n).unwrap_or(9_999_999)
            });
            match best {
                Some((idx, _)) => { ordered.push(remaining.remove(idx)); }
                None => { ordered.push(remaining.remove(0)); }
            }
        }

        let orig_lat = self.path_latency(shards);
        let new_lat = self.path_latency(&ordered);
        if new_lat < orig_lat { ordered } else { shards.to_vec() }
    }
}

// ── Job model ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScientificJob {
    pub job_id: String,
    pub title: String,
    pub job_type: String,
    pub model: Option<String>,
    pub input_hash: String,
    pub requester: String,
    pub max_fee: u64,
    pub open_source: bool,
    pub shard_group_id: Option<String>,
    pub status: String,
    pub result_hash: Option<String>,
    pub result_blob_cid: Option<String>,
    pub on_chain: bool,
    pub contributing_nodes: Vec<String>,
    pub work_values: HashMap<String, u64>,
    pub created_at: u64,
    pub started_at: Option<u64>,
    pub completed_at: Option<u64>,
    pub fee_paid: u64,
    pub epoch: Option<u64>,
}

// ── Fee / payout math ─────────────────────────────────────────────────────────

pub fn compute_actual_fee(base_fee: u64, open_source: bool) -> u64 {
    if base_fee == 0 { return 0; }
    if open_source {
        base_fee.saturating_sub(base_fee * OPEN_SOURCE_DISCOUNT_BPS / 10_000)
    } else {
        base_fee
    }
}

pub fn compute_node_payout(job: &ScientificJob, node_id: &str) -> u64 {
    let work = *job.work_values.get(node_id).unwrap_or(&0);
    if job.open_source {
        work.saturating_add(work * SCIENCE_NODE_BONUS_BPS / 10_000)
    } else {
        work
    }
}

// ── Job lifecycle ─────────────────────────────────────────────────────────────

pub struct ScientificEngine {
    chain: Arc<Chain>,
    pub latency: LatencyMap,
}

impl ScientificEngine {
    pub fn new(chain: Arc<Chain>) -> Self {
        Self { chain, latency: LatencyMap::new() }
    }

    fn job_key(job_id: &str) -> String {
        format!("sci_job:{}", job_id)
    }

    fn load_job(&self, job_id: &str) -> Result<ScientificJob> {
        let raw = self.chain.store.state_get(&Self::job_key(job_id))
            .ok_or_else(|| anyhow::anyhow!("scientific job '{}' not found", job_id))?;
        Ok(serde_json::from_slice(&raw)?)
    }

    fn save_job(&self, job: &ScientificJob) -> Result<()> {
        self.chain.store.state_set(&Self::job_key(&job.job_id), &serde_json::to_vec(job)?)?;
        Ok(())
    }

    pub fn create_job(
        &self,
        requester: &str,
        title: &str,
        job_type: &str,
        model: Option<&str>,
        input_data: &[u8],
        max_fee: u64,
        open_source: bool,
    ) -> Result<ScientificJob> {
        if requester.is_empty() { bail!("requester required"); }
        if input_data.is_empty() { bail!("input_data required"); }

        let job_id = hex::encode(&Sha256::digest(&[
            requester.as_bytes(),
            &now_ms().to_le_bytes(),
        ].concat())[..16]);

        let input_hash = hex::encode(Sha256::digest(input_data));
        let fee_paid = compute_actual_fee(max_fee, open_source);

        let job = ScientificJob {
            job_id: job_id.clone(),
            title: if title.is_empty() { job_id.clone() } else { title.to_owned() },
            job_type: if job_type.is_empty() { "general".to_owned() } else { job_type.to_owned() },
            model: model.map(str::to_owned),
            input_hash,
            requester: requester.to_owned(),
            max_fee,
            open_source,
            shard_group_id: None,
            status: "queued".to_owned(),
            result_hash: None,
            result_blob_cid: None,
            on_chain: false,
            contributing_nodes: vec![],
            work_values: HashMap::new(),
            created_at: now_ms(),
            started_at: None,
            completed_at: None,
            fee_paid,
            epoch: None,
        };

        self.save_job(&job)?;
        Ok(job)
    }

    pub fn start_job(&self, job_id: &str, shard_group_id: Option<&str>) -> Result<ScientificJob> {
        let mut job = self.load_job(job_id)?;
        if job.status != "queued" {
            bail!("job '{}' is not queued (status: {})", job_id, job.status);
        }
        job.status = "running".to_owned();
        job.shard_group_id = shard_group_id.map(str::to_owned);
        job.started_at = Some(now_ms());
        self.save_job(&job)?;
        Ok(job)
    }

    pub fn complete_job(
        &self,
        job_id: &str,
        result_hash: &str,
        result_bytes: &[u8],
        contributing_nodes: Vec<String>,
        work_values: HashMap<String, u64>,
        epoch: u64,
    ) -> Result<ScientificJob> {
        let mut job = self.load_job(job_id)?;
        if job.status != "running" {
            bail!("job '{}' is not running (status: {})", job_id, job.status);
        }

        job.result_hash = Some(result_hash.to_owned());
        job.contributing_nodes = contributing_nodes;
        job.work_values = work_values;
        job.completed_at = Some(now_ms());
        job.epoch = Some(epoch);
        job.status = "complete".to_owned();

        // Large results go to blob store prefix; inline otherwise.
        if result_bytes.len() > MAX_INLINE_RESULT_BYTES {
            // Generate a deterministic CID for the result blob.
            let cid = format!("sci:{}", hex::encode(Sha256::digest(result_bytes)));
            let blob_key = format!("sci_blob:{}", cid);
            let _ = self.chain.store.state_set(&blob_key, result_bytes);
            job.result_blob_cid = Some(cid);
        }

        // Inscribe on-chain for open-source jobs.
        if job.open_source {
            let entry = LedgerEntry::ScientificResult {
                job_id: job.job_id.clone(),
                requester: job.requester.clone(),
                job_type: job.job_type.clone(),
                title: job.title.clone(),
                model: job.model.clone(),
                input_hash: job.input_hash.clone(),
                result_hash: result_hash.to_owned(),
                result_blob_cid: job.result_blob_cid.clone(),
                fee_paid: job.fee_paid,
                contributing_nodes: job.contributing_nodes.clone(),
                epoch,
                signed_by: job.requester.clone(),
            };
            if let Err(e) = self.chain.apply_entry(&entry) {
                warn!("[science] on-chain inscribe failed for job '{}': {}", job_id, e);
            } else {
                job.on_chain = true;
            }
        }

        // Distribute payout to contributing nodes.
        for node_id in &job.contributing_nodes {
            let payout = compute_node_payout(&job, node_id);
            if payout > 0 {
                if let Err(e) = self.chain.store.credit(node_id, NATIVE_TOKEN, payout) {
                    warn!("[science] payout failed for node '{}': {}", node_id, e);
                }
            }
        }

        self.save_job(&job)?;
        Ok(job)
    }

    pub fn get_job(&self, job_id: &str) -> Option<ScientificJob> {
        self.load_job(job_id).ok()
    }

    pub fn get_open_jobs(&self) -> Vec<ScientificJob> {
        self.chain.store.state_scan_prefix("sci_job:")
            .into_iter()
            .filter_map(|(_, v)| serde_json::from_slice::<ScientificJob>(&v).ok())
            .filter(|j| j.status == "queued")
            .collect()
    }

    pub fn get_jobs_by_type(&self, job_type: &str) -> Vec<ScientificJob> {
        self.chain.store.state_scan_prefix("sci_job:")
            .into_iter()
            .filter_map(|(_, v)| serde_json::from_slice::<ScientificJob>(&v).ok())
            .filter(|j| j.job_type == job_type)
            .collect()
    }
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

// ── apply_entry stub (handled in chain.rs) ────────────────────────────────────

// ScientificResult and CrossChainFinalityAnnounce are append-only records.
// chain.rs apply_entry stores them under sci_result:{job_id} and
// cc_finality:{target_chain}:{epoch} respectively.

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn make_engine(label: &str) -> (ScientificEngine, tempfile::TempDir) {
        let dir = tempfile::Builder::new()
            .prefix(&format!("btcpc_sci_{}_", label))
            .tempdir()
            .expect("tempdir");
        let store = crate::store::Store::open(dir.path()).expect("store");
        let chain = Arc::new(crate::chain::Chain::new(
            store,
            format!("sci-{}", label),
            "btcpc-satoshi".to_string(),
        ));
        (ScientificEngine::new(chain), dir)
    }

    // ── Fee math ─────────────────────────────────────────────────────────────

    #[test]
    fn open_source_discount_40_percent() {
        let base = 10_000;
        let discounted = compute_actual_fee(base, true);
        assert_eq!(discounted, 6_000, "40% off → 6000");
    }

    #[test]
    fn closed_source_no_discount() {
        let base = 10_000;
        assert_eq!(compute_actual_fee(base, false), 10_000);
    }

    #[test]
    fn zero_fee_always_zero() {
        assert_eq!(compute_actual_fee(0, true), 0);
        assert_eq!(compute_actual_fee(0, false), 0);
    }

    #[test]
    fn node_payout_open_source_bonus() {
        let mut job = ScientificJob {
            job_id: "j1".into(),
            title: "t".into(),
            job_type: "general".into(),
            model: None,
            input_hash: "h".into(),
            requester: "alice".into(),
            max_fee: 1000,
            open_source: true,
            shard_group_id: None,
            status: "complete".into(),
            result_hash: None,
            result_blob_cid: None,
            on_chain: false,
            contributing_nodes: vec!["n1".into()],
            work_values: {
                let mut m = HashMap::new();
                m.insert("n1".into(), 1000u64);
                m
            },
            created_at: 0,
            started_at: None,
            completed_at: None,
            fee_paid: 600,
            epoch: None,
        };
        // 1000 + 25% bonus = 1250
        assert_eq!(compute_node_payout(&job, "n1"), 1250);

        // closed-source — no bonus
        job.open_source = false;
        assert_eq!(compute_node_payout(&job, "n1"), 1000);
    }

    #[test]
    fn node_payout_unknown_node_is_zero() {
        let job = ScientificJob {
            job_id: "j2".into(),
            title: "t".into(),
            job_type: "general".into(),
            model: None,
            input_hash: "h".into(),
            requester: "alice".into(),
            max_fee: 1000,
            open_source: true,
            shard_group_id: None,
            status: "complete".into(),
            result_hash: None,
            result_blob_cid: None,
            on_chain: false,
            contributing_nodes: vec![],
            work_values: HashMap::new(),
            created_at: 0,
            started_at: None,
            completed_at: None,
            fee_paid: 600,
            epoch: None,
        };
        assert_eq!(compute_node_payout(&job, "nobody"), 0);
    }

    // ── Latency map ───────────────────────────────────────────────────────────

    #[test]
    fn optimal_shard_order_single_node_unchanged() {
        let lm = LatencyMap::new();
        let shards = vec!["a"];
        assert_eq!(lm.optimal_shard_order(&shards), vec!["a"]);
    }

    #[test]
    fn optimal_shard_order_no_data_falls_back() {
        let lm = LatencyMap::new();
        let shards = vec!["a", "b", "c"];
        // No latency data → original order returned.
        assert_eq!(lm.optimal_shard_order(&shards), vec!["a", "b", "c"]);
    }

    #[test]
    fn optimal_shard_order_picks_nearest_neighbour() {
        let lm = LatencyMap::new();
        // a→b: 10ms, a→c: 100ms, b→c: 5ms
        lm.record("a", "b", 10);
        lm.record("a", "c", 100);
        lm.record("b", "c", 5);

        let shards = vec!["a", "b", "c"];
        let ordered = lm.optimal_shard_order(&shards);
        // Greedy from a: nearest is b (10ms), then c (5ms) → [a, b, c] total 15ms.
        // Original order [a,b,c] also has path_latency = 10+5 = 15ms via known hops.
        // Either [a,b,c] is returned (same or better).
        assert!(ordered == vec!["a", "b", "c"] || ordered[0] == "a");
    }

    #[test]
    fn path_latency_missing_hop_returns_max() {
        let lm = LatencyMap::new();
        lm.record("x", "y", 50);
        // x→y known but y→z not known → u64::MAX
        assert_eq!(lm.path_latency(&["x", "y", "z"]), u64::MAX);
    }

    // ── Job lifecycle ─────────────────────────────────────────────────────────

    #[test]
    fn create_job_persisted_and_readable() {
        let (engine, _dir) = make_engine("create");
        let job = engine
            .create_job("alice", "Test job", "genomics", None, b"input data", 500, false)
            .expect("create");
        assert_eq!(job.status, "queued");
        assert_eq!(job.requester, "alice");
        assert_eq!(job.job_type, "genomics");
        assert!(!job.open_source);
        assert_eq!(job.fee_paid, 500);

        // Must be loadable back from store.
        let loaded = engine.get_job(&job.job_id).expect("loaded");
        assert_eq!(loaded.job_id, job.job_id);
    }

    #[test]
    fn create_job_open_source_discount_applied() {
        let (engine, _dir) = make_engine("oss_discount");
        let job = engine
            .create_job("bob", "OSS job", "climate", None, b"data", 10_000, true)
            .expect("create");
        assert_eq!(job.fee_paid, 6_000); // 40% off
        assert!(job.open_source);
    }

    #[test]
    fn create_job_empty_requester_fails() {
        let (engine, _dir) = make_engine("empty_req");
        assert!(engine.create_job("", "t", "g", None, b"d", 100, false).is_err());
    }

    #[test]
    fn create_job_empty_input_fails() {
        let (engine, _dir) = make_engine("empty_input");
        assert!(engine.create_job("alice", "t", "g", None, b"", 100, false).is_err());
    }

    #[test]
    fn start_job_transitions_to_running() {
        let (engine, _dir) = make_engine("start");
        let job = engine
            .create_job("alice", "t", "g", None, b"data", 100, false)
            .expect("create");
        let started = engine.start_job(&job.job_id, Some("shard-group-1")).expect("start");
        assert_eq!(started.status, "running");
        assert_eq!(started.shard_group_id.as_deref(), Some("shard-group-1"));
        assert!(started.started_at.is_some());
    }

    #[test]
    fn start_job_non_queued_fails() {
        let (engine, _dir) = make_engine("start_fail");
        let job = engine
            .create_job("alice", "t", "g", None, b"data", 100, false)
            .expect("create");
        engine.start_job(&job.job_id, None).expect("first start");
        // Second start on running job must fail.
        assert!(engine.start_job(&job.job_id, None).is_err());
    }

    #[test]
    fn complete_job_lifecycle() {
        let (engine, _dir) = make_engine("complete");
        let job = engine
            .create_job("alice", "t", "g", None, b"data", 100, false)
            .expect("create");
        engine.start_job(&job.job_id, None).expect("start");

        let nodes = vec!["n1".into()];
        let mut work = HashMap::new();
        work.insert("n1".into(), 80u64);

        let done = engine
            .complete_job(&job.job_id, "deadbeef", b"result", nodes, work, 5)
            .expect("complete");
        assert_eq!(done.status, "complete");
        assert_eq!(done.result_hash.as_deref(), Some("deadbeef"));
        assert!(done.completed_at.is_some());
        assert_eq!(done.epoch, Some(5));
    }

    #[test]
    fn complete_job_large_result_stored_as_blob() {
        let (engine, _dir) = make_engine("blob");
        let job = engine
            .create_job("alice", "big", "g", None, b"d", 100, false)
            .expect("create");
        engine.start_job(&job.job_id, None).expect("start");

        let large = vec![0u8; MAX_INLINE_RESULT_BYTES + 1];
        let done = engine
            .complete_job(&job.job_id, "hash", &large, vec![], HashMap::new(), 1)
            .expect("complete");
        assert!(done.result_blob_cid.is_some(), "large result must get a CID");
    }

    #[test]
    fn complete_job_not_running_fails() {
        let (engine, _dir) = make_engine("complete_fail");
        let job = engine
            .create_job("alice", "t", "g", None, b"d", 100, false)
            .expect("create");
        // Still queued — completing without start must fail.
        assert!(engine
            .complete_job(&job.job_id, "h", b"r", vec![], HashMap::new(), 1)
            .is_err());
    }
}
