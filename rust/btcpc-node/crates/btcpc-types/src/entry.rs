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
        keys: std::collections::HashMap<String, String>,  // role -> compressed pubkey hex
        epoch: Epoch,
    },
    AccountUpdateKey {
        account: AccountId,
        role: String,  // "owner" | "active" | "posting" | "memo" | "hide" | "seek"
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
        nonce: u64,
        signed_by: AccountId,
    },
    Unstake {
        account: AccountId,
        amount: Dreams,
        epoch: Epoch,
        nonce: u64,
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

    // ── Inference Marketplace ─────────────────────────────────────────────────
    /// Requester posts a job; max_fee held in escrow until completion or cancel.
    InferenceJobPost {
        job_id: String,
        requester: AccountId,
        model: String,
        /// "solo" | "ensemble" | "pipeline"
        mode: String,
        /// SHA-256 hex of the actual input (input stored off-chain or in request body).
        input_hash: String,
        /// Maximum fee in dreams. Held in escrow. Actual fee ≤ max_fee.
        max_fee: Dreams,
        /// Minimum node reputation score required (0 = any node).
        min_reputation: u64,
        /// Bid window in epochs before auto-award (default 2).
        bid_window_epochs: u64,
        /// Job expires (auto-cancelled) if not completed by this epoch.
        deadline_epoch: u64,
        epoch: Epoch,
        nonce: u64,
        signed_by: AccountId,
    },
    /// A node bids to perform work on a posted job.
    InferenceJobBid {
        job_id: String,
        bidder: AccountId,
        /// Fee the bidder will accept (must be ≤ job max_fee).
        fee: Dreams,
        /// "worker" | "verifier" | "reviewer"
        role: String,
        epoch: Epoch,
        nonce: u64,
        signed_by: AccountId,
    },
    /// Job awarded to the winning bidder after bid window closes (node-emitted).
    InferenceJobAward {
        job_id: String,
        winner: AccountId,
        role: String,
        fee: Dreams,
        epoch: Epoch,
    },
    /// Worker submits proof of completion with result hash.
    InferenceJobComplete {
        job_id: String,
        worker: AccountId,
        result_hash: String,
        latency_ms: u64,
        epoch: Epoch,
        signed_by: AccountId,
    },
    /// System verifier confirms or disputes a completed job.
    /// "approved" → payment flows; "disputed" → enters dispute window.
    InferenceJobVerify {
        job_id: String,
        verifier: AccountId,
        /// "approved" | "disputed"
        verdict: String,
        reason: Option<String>,
        epoch: Epoch,
        signed_by: AccountId,
    },
    /// Worker contests a verifier's "disputed" verdict.
    /// Must be submitted within CLAIM_WINDOW_EPOCHS of the dispute.
    /// If no claim is submitted in time, the worker receives no fee.
    InferenceJobClaim {
        job_id: String,
        claimant: AccountId,
        /// Optional hash of evidence supporting the claim.
        evidence_hash: Option<String>,
        epoch: Epoch,
        nonce: u64,
        signed_by: AccountId,
    },
    /// Human reviewer votes on a claimed dispute.
    /// Payment to reviewers only happens when this path is taken.
    InferenceReviewVote {
        job_id: String,
        reviewer: AccountId,
        /// true = worker did valid work; false = work not done / invalid.
        approved: bool,
        epoch: Epoch,
        signed_by: AccountId,
    },
    /// Fee distribution after job verification (node-emitted after complete).
    /// reviewer_payments is empty on the happy path (no dispute).
    InferenceJobPay {
        job_id: String,
        worker: AccountId,
        worker_amount: Dreams,
        /// (account, amount) pairs for each verifier that participated.
        verifier_payments: Vec<(AccountId, Dreams)>,
        /// (account, amount) pairs for human reviewers (non-empty only on disputed path).
        reviewer_payments: Vec<(AccountId, Dreams)>,
        recycle_amount: Dreams,
        refund_amount: Dreams,
        epoch: Epoch,
    },
    /// Job cancelled by requester or expired; escrow refunded.
    InferenceJobCancel {
        job_id: String,
        cancelled_by: AccountId,
        reason: String,
        epoch: Epoch,
        nonce: u64,
        signed_by: AccountId,
    },

    // ── Clock Reward ──────────────────────────────────────────────────────────
    /// Minimal per-epoch reward to clock nodes to keep the chain alive.
    ClockReward {
        node_id: AccountId,
        amount: Dreams,
        epoch: Epoch,
    },

    // ── Storage ───────────────────────────────────────────────────────────────
    BlobStore {
        cid: String,
        uploader: AccountId,
        size_bytes: u64,
        epoch: Epoch,
        fee: Dreams,
    },

    // ── Freeport: Commerce ────────────────────────────────────────────────────
    /// Create or update a storefront. Signed by posting key.
    StoreUpdate {
        seller: AccountId,
        store_id: String,
        name: Option<String>,
        description: Option<String>,
        onion_address: Option<String>,
        metadata: Option<serde_json::Value>,
        epoch: Epoch,
        signed_by: AccountId,
    },
    /// List a new product. Signed by posting key.
    ProductCreate {
        seller: AccountId,
        product_id: String,
        store_id: String,
        title: String,
        price: Dreams,
        token: String,
        /// "physical" | "digital" | "service"
        product_type: String,
        /// CID of encrypted payload for digital products (fulfilled automatically on order).
        delivery_cid: Option<String>,
        stock: Option<u64>,
        metadata: Option<serde_json::Value>,
        epoch: Epoch,
        signed_by: AccountId,
    },
    /// Update an existing product listing. Signed by posting key.
    ProductUpdate {
        seller: AccountId,
        product_id: String,
        price: Option<Dreams>,
        stock: Option<u64>,
        active: Option<bool>,
        metadata: Option<serde_json::Value>,
        epoch: Epoch,
        signed_by: AccountId,
    },
    /// Buyer places an order; price locked in escrow via active key.
    OrderPlace {
        order_id: String,
        buyer: AccountId,
        seller: AccountId,
        product_id: String,
        quantity: u64,
        total_price: Dreams,
        token: String,
        /// Buyer's public key for encrypting digital delivery.
        buyer_pubkey: Option<String>,
        epoch: Epoch,
        nonce: u64,
        signed_by: AccountId,
    },
    /// Seller fulfills order; digital delivery encrypted to buyer_pubkey. Signed by seek key.
    OrderFulfill {
        order_id: String,
        seller: AccountId,
        /// CID of delivery payload (encrypted to buyer's hide key for digital products).
        delivery_cid: Option<String>,
        tracking: Option<String>,
        epoch: Epoch,
        signed_by: AccountId,
    },
    /// Buyer or seller cancels order; escrow refunded to buyer.
    OrderCancel {
        order_id: String,
        cancelled_by: AccountId,
        reason: Option<String>,
        epoch: Epoch,
        nonce: u64,
        signed_by: AccountId,
    },
    /// Buyer opens a dispute after receiving order.
    OrderDispute {
        order_id: String,
        buyer: AccountId,
        reason: String,
        evidence_hash: Option<String>,
        epoch: Epoch,
        signed_by: AccountId,
    },
    /// Protocol releases escrow to seller after fulfillment + timeout or buyer confirmation.
    EscrowRelease {
        order_id: String,
        seller: AccountId,
        amount: Dreams,
        token: String,
        epoch: Epoch,
    },
    /// Seller sets a time-limited flash sale price. Signed by posting key.
    FlashSale {
        seller: AccountId,
        product_id: String,
        sale_price: Dreams,
        expires_epoch: Epoch,
        epoch: Epoch,
        signed_by: AccountId,
    },

    // ── Verasens: Sensors / IoT ───────────────────────────────────────────────
    /// Register a sensor on-chain. Signed by owner's posting key.
    SensorRegister {
        sensor_id: String,
        owner: AccountId,
        sensor_type: String,
        location: Option<String>,
        metadata: Option<serde_json::Value>,
        epoch: Epoch,
        signed_by: AccountId,
    },
    /// Register a device-level signing key for a sensor. Signed by owner's posting key.
    SensorKeyRegister {
        sensor_id: String,
        owner: AccountId,
        device_pubkey: String,
        epoch: Epoch,
        signed_by: AccountId,
    },
    /// Owner vouches for a sensor's data quality. Signed by posting key.
    SensorVouch {
        sensor_id: String,
        voucher: AccountId,
        epoch: Epoch,
        signed_by: AccountId,
    },
    /// Batch commit of sensor readings. Signed by device key.
    SensorDataCommit {
        sensor_id: String,
        owner: AccountId,
        /// SHA-256 of the batch JSON.
        batch_hash: String,
        reading_count: u64,
        epoch: Epoch,
        signed_by: AccountId,
    },
    /// Register a hardware IoT device key. Signed by owner's posting key.
    DeviceKeyRegister {
        device_id: String,
        owner: AccountId,
        device_pubkey: String,
        hardware_hash: Option<String>,
        epoch: Epoch,
        signed_by: AccountId,
    },
    /// Device stakes yield to earn storage/sensor rewards.
    DeviceYieldStake {
        device_id: String,
        owner: AccountId,
        amount: Dreams,
        epoch: Epoch,
        nonce: u64,
        signed_by: AccountId,
    },
    /// Device unstakes yield.
    DeviceYieldUnstake {
        device_id: String,
        owner: AccountId,
        amount: Dreams,
        epoch: Epoch,
        nonce: u64,
        signed_by: AccountId,
    },
    /// LoRa gateway heartbeat proving uptime.
    GatewayHeartbeat {
        gateway_id: String,
        owner: AccountId,
        epoch: Epoch,
        signed_by: AccountId,
    },
    /// Storage node heartbeat proving availability.
    StorageHeartbeat {
        node_id: AccountId,
        epoch: Epoch,
        bytes_proven: u64,
        signed_by: AccountId,
    },

    // ── btcpc-git: Decentralized Git ─────────────────────────────────────────
    /// Register a new git repository. If hide_key is set the repo is private;
    /// all objects are encrypted to that key before storage.
    GitRepoCreate {
        repo_id: String,
        owner: AccountId,
        name: String,
        /// "public" | "private"
        visibility: String,
        /// Owner's hide public key (hex). Required for private repos.
        hide_key: Option<String>,
        epoch: Epoch,
        signed_by: AccountId,
    },
    /// Update a branch or tag ref. Triggers storage nodes to prune
    /// objects no longer reachable from any live ref in this repo.
    GitRefUpdate {
        repo_id: String,
        owner: AccountId,
        /// e.g. "refs/heads/main" or "refs/tags/v1.0"
        ref_name: String,
        /// New commit hash (SHA-256 hex of the commit object).
        commit_hash: String,
        /// Previous commit hash — storage nodes use this to compute reachable diff.
        prev_hash: Option<String>,
        epoch: Epoch,
        signed_by: AccountId,
    },
    /// Grant read access to a private repo by sharing the repo's symmetric key
    /// encrypted to the grantee's hide key.
    GitAccessGrant {
        repo_id: String,
        grantor: AccountId,
        grantee: AccountId,
        /// Repo symmetric key encrypted to grantee's hide public key (hex).
        encrypted_key: String,
        epoch: Epoch,
        signed_by: AccountId,
    },
    /// Revoke a previously granted access. Storage nodes stop serving
    /// encrypted objects to the grantee.
    GitAccessRevoke {
        repo_id: String,
        grantor: AccountId,
        grantee: AccountId,
        epoch: Epoch,
        signed_by: AccountId,
    },
    /// Storage node declares it has pruned unreachable objects after a GitRefUpdate.
    /// Earns a small reward for confirmed GC work.
    GitPruneProof {
        repo_id: String,
        node_id: AccountId,
        /// Merkle root of pruned CIDs.
        pruned_root: String,
        bytes_freed: u64,
        epoch: Epoch,
        signed_by: AccountId,
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
            Self::InferenceJobPost { epoch, .. } => *epoch,
            Self::InferenceJobBid { epoch, .. } => *epoch,
            Self::InferenceJobAward { epoch, .. } => *epoch,
            Self::InferenceJobComplete { epoch, .. } => *epoch,
            Self::InferenceJobVerify { epoch, .. } => *epoch,
            Self::InferenceJobClaim { epoch, .. } => *epoch,
            Self::InferenceReviewVote { epoch, .. } => *epoch,
            Self::InferenceJobPay { epoch, .. } => *epoch,
            Self::InferenceJobCancel { epoch, .. } => *epoch,
            Self::ClockReward { epoch, .. } => *epoch,
            Self::BlobStore { epoch, .. } => *epoch,
            Self::StoreUpdate { epoch, .. } => *epoch,
            Self::ProductCreate { epoch, .. } => *epoch,
            Self::ProductUpdate { epoch, .. } => *epoch,
            Self::OrderPlace { epoch, .. } => *epoch,
            Self::OrderFulfill { epoch, .. } => *epoch,
            Self::OrderCancel { epoch, .. } => *epoch,
            Self::OrderDispute { epoch, .. } => *epoch,
            Self::EscrowRelease { epoch, .. } => *epoch,
            Self::FlashSale { epoch, .. } => *epoch,
            Self::SensorRegister { epoch, .. } => *epoch,
            Self::SensorKeyRegister { epoch, .. } => *epoch,
            Self::SensorVouch { epoch, .. } => *epoch,
            Self::SensorDataCommit { epoch, .. } => *epoch,
            Self::DeviceKeyRegister { epoch, .. } => *epoch,
            Self::DeviceYieldStake { epoch, .. } => *epoch,
            Self::DeviceYieldUnstake { epoch, .. } => *epoch,
            Self::GatewayHeartbeat { epoch, .. } => *epoch,
            Self::StorageHeartbeat { epoch, .. } => *epoch,
            Self::GitRepoCreate { epoch, .. } => *epoch,
            Self::GitRefUpdate { epoch, .. } => *epoch,
            Self::GitAccessGrant { epoch, .. } => *epoch,
            Self::GitAccessRevoke { epoch, .. } => *epoch,
            Self::GitPruneProof { epoch, .. } => *epoch,
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
