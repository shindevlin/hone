//! Ledger entry types — the universal event format for all chain state mutations.
//! Every state change is an entry. Entries are included in blocks, replayed for sync.

use serde::{Deserialize, Serialize};
use crate::account::{AccountId, Dreams, Epoch};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LedgerEntry {
    // ── Account ──────────────────────────────────────────────────────────────
    AccountCreate {
        account: AccountId,
        public_key: Option<String>,
        epoch: Epoch,
    },
    AccountUpdateKey {
        account: AccountId,
        new_public_key: String,
        epoch: Epoch,
        signed_by: AccountId,
    },

    // ── Transfers ────────────────────────────────────────────────────────────
    Transfer {
        from: AccountId,
        to: AccountId,
        amount: Dreams,
        token: String,
        memo: Option<String>,
        epoch: Epoch,
        signed_by: AccountId,
        nonce: u64,
    },

    // ── Staking ──────────────────────────────────────────────────────────────
    Stake {
        account: AccountId,
        amount: Dreams,
        epoch: Epoch,
        signed_by: AccountId,
    },
    Unstake {
        account: AccountId,
        amount: Dreams,
        epoch: Epoch,
        signed_by: AccountId,
    },

    // ── Mining ───────────────────────────────────────────────────────────────
    Mine {
        miner: AccountId,
        epoch: Epoch,
        work_value: u64,
        block_hash: String,
    },
    MineReward {
        miner: AccountId,
        amount: Dreams,
        epoch: Epoch,
    },

    // ── Epoch / Clock ─────────────────────────────────────────────────────────
    EpochSeal {
        node_id: AccountId,
        epoch: Epoch,
        timestamp: u64,
        seal_hash: String,
        signature: Option<String>,
    },
    EpochFinalize {
        epoch: Epoch,
        sealed_by: Vec<AccountId>,
        state_root: String,
        timestamp: u64,
    },

    // ── Smart Contracts ───────────────────────────────────────────────────────
    ContractDeploy {
        deployer: AccountId,
        contract_id: String,
        wasm_hash: String,
        epoch: Epoch,
        gas_used: u64,
    },
    ContractCall {
        caller: AccountId,
        contract_id: String,
        method: String,
        epoch: Epoch,
        gas_used: u64,
        success: bool,
        result_hash: Option<String>,
    },

    // ── Sensors ───────────────────────────────────────────────────────────────
    SensorReading {
        sensor_id: String,
        owner: AccountId,
        epoch: Epoch,
        value: f64,
        data_hash: String,
        metadata: Option<serde_json::Value>,
    },

    // ── Inference ─────────────────────────────────────────────────────────────
    InferenceJob {
        job_id: String,
        buyer: AccountId,
        miner: Option<AccountId>,
        model: String,
        fee: Dreams,
        tokens: u64,
        epoch: Epoch,
        status: String,
    },

    // ── Storage ───────────────────────────────────────────────────────────────
    BlobStore {
        cid: String,
        uploader: AccountId,
        size_bytes: u64,
        epoch: Epoch,
        fee: Dreams,
    },

    // ── Genesis ───────────────────────────────────────────────────────────────
    GenesisAlloc {
        account: AccountId,
        amount: Dreams,
        token: String,
    },
}

impl LedgerEntry {
    pub fn epoch(&self) -> Epoch {
        match self {
            Self::AccountCreate { epoch, .. } => *epoch,
            Self::AccountUpdateKey { epoch, .. } => *epoch,
            Self::Transfer { epoch, .. } => *epoch,
            Self::Stake { epoch, .. } => *epoch,
            Self::Unstake { epoch, .. } => *epoch,
            Self::Mine { epoch, .. } => *epoch,
            Self::MineReward { epoch, .. } => *epoch,
            Self::EpochSeal { epoch, .. } => *epoch,
            Self::EpochFinalize { epoch, .. } => *epoch,
            Self::ContractDeploy { epoch, .. } => *epoch,
            Self::ContractCall { epoch, .. } => *epoch,
            Self::SensorReading { epoch, .. } => *epoch,
            Self::InferenceJob { epoch, .. } => *epoch,
            Self::BlobStore { epoch, .. } => *epoch,
            Self::GenesisAlloc { .. } => 0,
        }
    }

    pub fn hash(&self) -> String {
        use sha2::Digest;
        let json = serde_json::to_string(self).unwrap_or_default();
        let mut h = sha2::Sha256::new();
        h.update(json.as_bytes());
        hex::encode(h.finalize())
    }
}
