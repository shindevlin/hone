//! Ledger entry types — the universal event format for all chain state mutations.
//! Every state change is an entry. Entries are included in blocks, replayed for sync.

use serde::{Deserialize, Serialize};
use crate::account::{AccountId, Dreams, Epoch};

/// A cryptographic commitment to an external-chain address.
///
/// The actual address is NEVER stored on-chain. Only `sha256(chain:address:nonce)`
/// is recorded, so the chain can prove ownership without deanonymizing the user.
///
/// To reveal the address to a specific party later, share (address, nonce) privately.
/// They verify: sha256(chain + ":" + address + ":" + nonce) == commitment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainProof {
    /// Chain name: "bitcoin", "ethereum", "solana", "near", "sui", etc.
    pub chain: String,
    /// sha256(chain + ":" + address + ":" + nonce) encoded as lowercase hex.
    pub commitment: String,
    /// "easy"  — self-asserted; all keys derived from one BIP-39 mnemonic by the node.
    /// "hard"  — cryptographically verified; user signed a challenge with an existing
    ///           external wallet (MetaMask, Ledger, Phantom, etc.). The signature is
    ///           stored in the VerifyChainLink entry for independent verification.
    pub mode: String,
}

/// Cross-chain 2FA factor attached to a protected transaction.
///
/// Each key slot can independently have a 2FA policy backed by a different
/// external-chain wallet. Including a `TwoFactor` proves the slot's 2FA
/// requirement is satisfied without revealing which on-chain address it maps to.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwoFactor {
    /// Chain the 2FA wallet lives on: "ethereum", "base", "solana", …
    pub chain: String,
    /// Hex-encoded signature over sha256(entry_hash + ":" + epoch) using the
    /// 2FA wallet's key. The node verifies via ecrecover / ed25519 as appropriate.
    pub signature: String,
}

/// Compact Merkle range proof used to respond to per-epoch storage challenges (T4-1, D10).
///
/// The chain verifies that `challenge_hash` matches what the clock derived from the previous
/// epoch's seal. Full Merkle path verification is performed by independent storage auditors.
#[derive(Debug, Clone, Serialize, Deserialize, borsh::BorshSerialize, borsh::BorshDeserialize, PartialEq)]
pub struct MerkleRangeProof {
    /// sha256("{prev_seal_hash}:{node_id}:{epoch}") — ties the proof to one specific challenge.
    pub challenge_hash: String,
    /// Byte-offset of the first byte in the challenged chunk.
    pub range_start: u64,
    /// Byte-offset one past the last byte in the challenged chunk.
    pub range_end: u64,
    /// Total number of chunks in the stored file.
    /// Used to compute effective_bytes = total_chunks × STORAGE_CHALLENGE_CHUNK_BYTES (T2-4).
    pub total_chunks: u64,
    /// sha256("chunk:" || range_start_be_8 || ":" || chunk_bytes) — hash of the challenged data.
    /// The chain cannot verify the raw bytes, but it verifies this hash is consistent
    /// with `proof_nodes` and `merkle_root`.
    pub leaf_hash: String,
    /// Merkle root of the storage node's full content tree at this epoch.
    pub merkle_root: String,
    /// Sibling hashes at each level from leaf to root (excluding root itself).
    /// Left/right position is derived from chunk index parity: index even → current is left.
    pub proof_nodes: Vec<String>,
}

/// Owner-level authority bundle for the 3-of-4 adaptive threshold.
/// Required when an action needs owner-level approval (key rotation, 2FA changes).
///
/// Threshold: owner key sign + any 2 of { owner_2fa, corroborant_key }.
/// With all factors present this is 3-of-4; absent factors lower the bar
/// proportionally (e.g. only owner key + owner_2fa = 2-of-3 when no corroborant).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnerAuth {
    /// 2FA sig from the wallet linked to the owner slot (if any).
    pub owner_2fa: Option<TwoFactor>,
    /// A corroborating BTCPC role: "active" | "posting" — signing key sig.
    pub corroborant_key: Option<String>,
    /// ed25519 hex sig from the corroborant key over the entry canonical message.
    pub corroborant_sig: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LedgerEntry {
    // ── Account ──────────────────────────────────────────────────────────────
    AccountCreate {
        account: AccountId,
        /// BTCPC protocol keys — public, required for transaction verification.
        /// Roles: "posting" (daily use), "owner" (key rotation), "memo" (encrypted msgs).
        keys: std::collections::BTreeMap<String, String>,
        /// Cross-chain address commitments — no plaintext addresses stored.
        /// Easy mode: populated automatically from BIP-39 derived wallet.
        /// Hard mode: added later via VerifyChainLink entries.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        chain_proofs: Vec<ChainProof>,
        epoch: Epoch,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        funded_by: Option<AccountId>,
    },
    AccountUpdateKey {
        account: AccountId,
        role: String,  // "owner" | "active" | "posting" | "memo" | "hide" | "seek"
        new_public_key: String,
        epoch: Epoch,
        signed_by: AccountId,
    },
    /// Declare another account as the primary identity for this account's wallet.
    /// Must be an existing account sharing the same posting key.
    /// Required before AccountTransfer — proves the owner won't go keyless.
    AccountSetPrimary {
        account: AccountId,   // the account being configured, e.g. "josh"
        primary: AccountId,   // the owner's other identity, e.g. "joshua"
        epoch: Epoch,
        signed_by: AccountId,
        signature: Option<String>,
    },
    /// Set a chain-wide governance parameter. Initially only authorized accounts can call this.
    /// key examples: "name_stake_enabled" ("true"/"false"), "name_stake_amount" (u64 as string)
    ChainParameterSet {
        key: String,
        value: String,
        signed_by: AccountId,
        epoch: Epoch,
        signature: Option<String>,
    },
    /// Transfer the identity (account name) to a new owner's full wallet set.
    /// Requires AccountSetPrimary to have been called first — the stored primary
    /// receives any balance automatically before keys are rotated.
    AccountTransfer {
        account: AccountId,
        new_keys: std::collections::BTreeMap<String, String>,  // role -> pubkey (posting, btc, eth, …)
        epoch: Epoch,
        signed_by: AccountId,
        nonce: u64,
        signature: Option<String>,
    },

    /// Prove ownership of an external-chain address via a signed challenge.
    ///
    /// Hard-mode chain linking: the user signs a canonical challenge with their existing
    /// wallet (MetaMask, Ledger, Phantom, etc.) and submits it here. The node verifies the
    /// signature, recovers the address, confirms it matches the commitment, then records
    /// the proof. The address itself is never stored — only the commitment.
    ///
    /// Anyone can independently verify the link by checking the stored signature against
    /// the stored challenge message — no trust required.
    VerifyChainLink {
        account: AccountId,
        /// Chain being proven: "ethereum", "bitcoin", "solana", etc.
        chain: String,
        /// sha256(chain + ":" + address + ":" + nonce) hex — submitted by user pre-computed.
        /// Node verifies this matches what it recovers from the signature.
        commitment: String,
        /// The exact message that was signed: "btcpc:link:{account}:{chain}:{nonce}"
        signed_message: String,
        /// Hex-encoded raw signature bytes.
        signature: String,
        /// Signature scheme used: "eth_personal_sign" | "sol_sign"
        sig_type: String,
        epoch: Epoch,
        signed_by: AccountId,
    },

    /// Configure the 2FA policy for a specific key slot on an account.
    ///
    /// Setting `twofactor_chain` to Some enables 2FA for the slot backed by the
    /// commitment already stored on-chain for that chain. Setting it to None clears
    /// the slot's 2FA policy. Owner slot changes always require `owner_auth`.
    /// Non-owner slot changes require a valid sig from the owner key.
    SetKeyPolicy {
        account: AccountId,
        /// Which slot: "owner" | "active" | "posting" | "memo" | "hide" | "seek"
        role: String,
        /// Chain whose stored commitment is the 2FA wallet. None = clear 2FA.
        twofactor_chain: Option<String>,
        /// Owner-threshold bundle (required for owner slot; optional corroboration otherwise).
        owner_auth: OwnerAuth,
        epoch: Epoch,
        signed_by: AccountId,
        /// ed25519 sig over canonical message from the owner key (or posting if no owner).
        signature: Option<String>,
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
        /// Optional 2FA factor for the active key slot (if the slot has a policy set).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        twofactor: Option<TwoFactor>,
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

    // ── Node Role Staking ─────────────────────────────────────────────────────
    /// Node operator enables backer yield sharing for a specific role.
    /// Backers earn `backer_share_bps / 10000` of the node's role rewards, pro-rata by stake.
    /// Node must be running the given role and must re-opt-in after any role changes.
    NodeRoleOptIn {
        node: AccountId,
        /// "clock" | "miner" | "storage" | "sensor" | "service" | "verifier" | "mempool"
        role: String,
        /// Share of epoch rewards paid to backers (e.g. 2000 = 20%). Capped at 5000 (50%).
        backer_share_bps: u16,
        /// Maximum simultaneous backers allowed (1–50).
        max_backers: u8,
        epoch: Epoch,
        nonce: u64,
        signed_by: AccountId,
    },
    /// Node operator disables backer yield sharing for a role.
    /// Existing backers keep their stake positions but earn nothing until the node opts back in.
    NodeRoleOptOut {
        node: AccountId,
        role: String,
        epoch: Epoch,
        nonce: u64,
        signed_by: AccountId,
    },
    /// Back a node's specific role — earn a proportional share of their epoch rewards.
    /// The node must have opted in via NodeRoleOptIn. shindevlin can back natoshisakamoto's
    /// clock role: `NodeRoleStake { node: "natoshisakamoto", role: "clock", staker: "shindevlin", ... }`.
    NodeRoleStake {
        node: AccountId,
        role: String,
        staker: AccountId,
        amount: Dreams,
        epoch: Epoch,
        nonce: u64,
        signed_by: AccountId,
    },
    /// Unstake from a node role position, returning the staked amount.
    NodeRoleUnstake {
        node: AccountId,
        role: String,
        staker: AccountId,
        amount: Dreams,
        epoch: Epoch,
        nonce: u64,
        signed_by: AccountId,
    },

    // ── Mining ───────────────────────────────────────────────────────────────
    Mine {
        miner: AccountId,
        epoch: Epoch,
        /// Model identifier (e.g. "qwen2.5:0.5b", "llama3:8b").
        model: String,
        /// Prompt tokens consumed during this epoch.
        input_tokens: u64,
        /// Tokens generated — primary proof-of-work quantity.
        output_tokens: u64,
        /// Tool / function calls executed.
        tool_calls: u64,
        /// Hardware tier: 0=phone, 1=cpu, 2=gpu-sm, 3=gpu-pro, 4=gpu-server.
        hw_tier: u8,
        /// SHA-256 hex of the inference output — a binding commitment to the result.
        /// LLM outputs are non-deterministic; this is NOT a reproducibility proof.
        /// Renamed from `compute_proof`; old field name accepted for backward compat.
        #[serde(alias = "compute_proof")]
        output_hash: String,
        /// Job ID this mining session served. Required outside the grace period.
        /// Must match an InferenceJobComplete entry with an approved InferenceJobVerify.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        job_id: Option<String>,
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
        node_id: AccountId,
        rewards_hash: String,
        quorum: u64,
        sealed_by: Vec<AccountId>,
        state_root: String,
        timestamp: u64,
    },

    /// Register as a clock node. Requires staking >= current CLOCK_MIN_STAKE.
    /// Stored in sled as `clock_reg:{account}`. Slashable via ClockDoubleSignEvidence.
    ClockNodeRegister {
        node_id: AccountId,
        /// Stake to lock — must meet CLOCK_MIN_STAKE (read from chain_param:clock_min_stake).
        stake: Dreams,
        epoch: Epoch,
        /// Ed25519 public key hex (64 chars). Required for double-sign slash verification.
        #[serde(default)]
        pubkey: Option<String>,
        signature: Option<String>,
    },
    /// Evidence that a clock node signed two conflicting seals in the same epoch.
    /// Submitter gets a bounty; offender is slashed per D7.
    /// Both seal signatures are verified against the offender's registered pubkey
    /// before any stake is touched — framing is cryptographically impossible.
    ClockDoubleSignEvidence {
        submitter: AccountId,
        offender: AccountId,
        epoch: Epoch,
        /// First conflicting seal.
        seal_hash_a: String,
        timestamp_a: u64,
        /// Offender's ed25519 signature (hex) over "seal:{epoch}:{seal_hash_a}:{offender}:{timestamp_a}".
        sig_a: String,
        /// Second conflicting seal.
        seal_hash_b: String,
        timestamp_b: u64,
        /// Offender's ed25519 signature (hex) over "seal:{epoch}:{seal_hash_b}:{offender}:{timestamp_b}".
        sig_b: String,
        signature: Option<String>,
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

    /// Crowdsourced cellular coverage measurement (dead-spot mapping vertical).
    /// `signal_dbm = None` means no signal at all (dead spot).
    /// Rewards are higher for dead-spot reports (rarer, more valuable to telcos).
    /// Corroboration bonus: 3+ independent devices in the same 100m grid cell × carrier
    /// in the same epoch earn COVERAGE_CORROBORATION_BONUS_BPS extra.
    CoverageReport {
        reporter: AccountId,
        /// WGS-84 latitude in degrees.
        lat: f64,
        /// WGS-84 longitude in degrees.
        lon: f64,
        /// Received signal strength in dBm. None = no signal (dead spot).
        signal_dbm: Option<i32>,
        /// ITU-T E.212 carrier identifier: "{MCC}-{MNC}" e.g. "272-01" = Vodafone IE.
        carrier_mcc_mnc: String,
        /// Radio access technology: "5G" | "LTE" | "EDGE" | "HSPA" | "NR" | "none".
        technology: String,
        /// SHA-256 hex of the raw modem log for the measurement period.
        data_hash: String,
        epoch: Epoch,
        signed_by: AccountId,
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
        /// If true, pin input+result payload to btcpc-fs permanently after InferenceJobPay.
        /// Payload is otherwise pruneable 100 epochs after payment (D1).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        persist_on_fs: Option<bool>,
        /// Fee in dreams for persistent storage on btcpc-fs. Ignored if persist_on_fs=false.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fs_fee: Option<u64>,
        /// Minimum verifier board size. 0 or absent = auto-scale from network size.
        /// Set higher for mission-critical work; requester pays proportionally more.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min_verifiers: Option<u64>,
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
    /// Verifier claims a completed job to inspect.
    /// Worker responds by encrypting (prompt + result) to the verifier's memo public key.
    InferenceVerifyClaim {
        job_id: String,
        verifier: AccountId,
        epoch: Epoch,
        signed_by: AccountId,
    },
    /// Verifier commits to a verdict before revealing it (T4-2 commit-reveal).
    /// commit_hash = sha256(verdict || "|" || salt) where salt is a random hex string.
    /// The reveal phase (InferenceJobVerify) is only accepted after the commit phase closes.
    InferenceJobCommit {
        job_id: String,
        verifier: AccountId,
        /// sha256(verdict || "|" || salt) — hides verdict until reveal phase.
        commit_hash: String,
        epoch: Epoch,
        signed_by: AccountId,
    },
    /// Verifier reveals their committed verdict (T4-2 commit-reveal).
    /// "approved" → payment flows; "rejected" → worker loses fee;
    /// "review_required" → enters human review window.
    /// When commit-reveal is active, reveal_salt must match the prior InferenceJobCommit.
    InferenceJobVerify {
        job_id: String,
        verifier: AccountId,
        /// "approved" | "rejected" | "review_required"
        verdict: String,
        /// output_tokens × hw_tier_weight × model_weight × complexity_factor (1–4).
        /// Used for the verifier's Layer B pool weight and miner's epoch score.
        value_score: u64,
        reason: Option<String>,
        /// Salt matching the InferenceJobCommit. Required when commit-reveal is active.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reveal_salt: Option<String>,
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
        /// (account, amount) pairs for consensus verifiers only.
        verifier_payments: Vec<(AccountId, Dreams)>,
        /// (account, amount) pairs for human reviewers (non-empty only on disputed path).
        reviewer_payments: Vec<(AccountId, Dreams)>,
        recycle_amount: Dreams,
        refund_amount: Dreams,
        /// (account, stake_slash) for verifiers who dissented from board consensus.
        /// Slashed from staked balance, credited to recycle fund.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        dissenter_slashes: Vec<(AccountId, Dreams)>,
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

    // ── Epoch Rewards ─────────────────────────────────────────────────────────
    /// Layer D base reward to clock nodes (always fires, era-scaled).
    ClockReward {
        node_id: AccountId,
        amount: Dreams,
        epoch: Epoch,
    },
    /// Layer B pool reward to storage nodes (proportional to bytes_proven × query bonus).
    StorageReward {
        node_id: AccountId,
        amount: Dreams,
        epoch: Epoch,
    },
    /// Layer B pool reward to sensor owners (type-aware sensor_score weighting).
    SensorReward {
        node_id: AccountId,
        amount: Dreams,
        epoch: Epoch,
    },
    /// Layer B pool reward to verifiers (proportional to value_score_total this epoch).
    VerifierReward {
        node_id: AccountId,
        amount: Dreams,
        epoch: Epoch,
    },
    /// Layer B pool reward to service hosts (proportional to container_hours this epoch).
    ServiceReward {
        node_id: AccountId,
        amount: Dreams,
        epoch: Epoch,
    },
    /// Layer B pool reward to mempool operators (inversely weighted by propagation latency).
    MempoolReward {
        operator: AccountId,
        amount: Dreams,
        epoch: Epoch,
    },

    // ── Mempool: staked relay node type ──────────────────────────────────────
    /// Register as a mempool operator. Operator stakes BTCPC; lowest-latency
    /// nodes earn the most.  Slashable for censorship, double-inclusion, or
    /// fee front-running (slashing conditions enforced by governance + evidence entries).
    MempoolOperatorRegister {
        operator: AccountId,
        /// Stake to lock — must meet MEMPOOL_MIN_STAKE.
        amount: Dreams,
        epoch: Epoch,
        nonce: u64,
        signed_by: AccountId,
    },
    /// Periodic heartbeat from a mempool operator proving active relay work.
    MempoolHeartbeat {
        operator: AccountId,
        epoch: Epoch,
        /// P99 propagation latency in ms measured over this epoch.
        propagation_latency_ms: u64,
        /// Number of entries relayed to peers this epoch.
        entries_relayed: u64,
        signed_by: AccountId,
    },

    // ── Device claim stake ────────────────────────────────────────────────────
    /// Stake BTCPC to claim ownership of a device identified by its hardware serial.
    /// First staker of a serial wins.  Previous owner must DeviceClaimUnstake before
    /// a new owner can claim.  Stake is slashable for fraudulent claims.
    DeviceClaimStake {
        /// Hardware-burned unique identifier (IMEI, CPU serial, TPM endorsement key).
        device_serial: String,
        owner: AccountId,
        amount: Dreams,
        epoch: Epoch,
        nonce: u64,
        signed_by: AccountId,
    },
    /// Release a device claim, returning stake.  Required before a new owner can claim.
    DeviceClaimUnstake {
        device_serial: String,
        owner: AccountId,
        epoch: Epoch,
        nonce: u64,
        signed_by: AccountId,
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
        /// Value model for scoring: "continuous" | "event" | "sampled" | "pulse"
        /// Declared by the device at commit time; registry enforces consistency.
        sensor_type: String,
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
    /// Uses device_serial (hardware-burned ID) to match DeviceClaimStake.
    DeviceYieldStake {
        device_serial: String,
        staker: AccountId,
        amount: Dreams,
        epoch: Epoch,
        nonce: u64,
        signed_by: AccountId,
    },
    /// Device unstakes yield.
    DeviceYieldUnstake {
        device_serial: String,
        staker: AccountId,
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
    /// Storage node heartbeat proving availability and optionally responding to the
    /// per-epoch Merkle range challenge (T4-1, D10).
    StorageHeartbeat {
        node_id: AccountId,
        epoch: Epoch,
        bytes_proven: u64,
        /// Active data access queries served this epoch (used for query bonus up to 2×).
        query_count: u64,
        /// Cryptographic response to the deterministic Merkle range challenge.
        /// Challenge = sha256("{prev_seal_hash}:{node_id}:{epoch}").
        /// Missing or mismatched proof earns STORAGE_PROOF_NO_PROOF_BPS (20%) of normal reward.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        challenge_response: Option<MerkleRangeProof>,
        signed_by: AccountId,
    },
    /// Service node heartbeat proving active container-hours (decentralized compute).
    ServiceHeartbeat {
        node_id: AccountId,
        epoch: Epoch,
        /// Active container-hours in this epoch.
        container_hours: u64,
        signed_by: AccountId,
    },
    /// Buyer purchases a sensor data batch from a sensor owner.
    SensorDataPurchase {
        sensor_id: String,
        buyer: AccountId,
        owner: AccountId,
        /// SHA-256 of the batch being purchased.
        batch_hash: String,
        /// Fee paid by buyer; split: owner majority + storage contract rate + recycle.
        fee: Dreams,
        epoch: Epoch,
        nonce: u64,
        signed_by: AccountId,
    },

    // ── LinkGit: Decentralized Git ───────────────────────────────────────────
    /// Register a new git repository. If hide_key is set the repo is private;
    /// all objects are encrypted to that key before storage.
    LinkGitRepoCreate {
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
    LinkGitRefUpdate {
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
    LinkGitAccessGrant {
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
    LinkGitAccessRevoke {
        repo_id: String,
        grantor: AccountId,
        grantee: AccountId,
        epoch: Epoch,
        signed_by: AccountId,
    },
    /// Storage node declares it has pruned unreachable objects after a LinkGitRefUpdate.
    /// Earns a small reward for confirmed GC work.
    LinkGitPruneProof {
        repo_id: String,
        node_id: AccountId,
        /// Merkle root of pruned CIDs.
        pruned_root: String,
        bytes_freed: u64,
        epoch: Epoch,
        signed_by: AccountId,
    },
    /// Owner pays to retain unreachable objects beyond default prune window.
    /// Without this, storage nodes prune orphaned objects after a LinkGitRefUpdate.
    LinkGitStorageExtend {
        repo_id: String,
        owner: AccountId,
        /// CIDs to keep alive.
        cids: Vec<String>,
        /// Keep until this epoch.
        keep_until_epoch: Epoch,
        fee: Dreams,
        epoch: Epoch,
        signed_by: AccountId,
    },

    // ── Testnet ───────────────────────────────────────────────────────────────
    /// Register a mainnet account as a testnet operator so clock nodes on mainnet
    /// can reward it from the testnet fund each epoch.
    TestnetOperatorRegister {
        /// Mainnet account that will receive rewards.
        mainnet_account: AccountId,
        /// Identifying label for the testnet node (for logging / dedup).
        testnet_node_id: String,
        /// Chain ID of the testnet being operated (e.g. "btcpc-satoshi").
        testnet_chain_id: String,
        epoch: Epoch,
        signed_by: AccountId,
    },
    /// Periodic mainnet reward to a registered testnet operator.
    /// Emitted by mainnet clock nodes each epoch from the testnet fund.
    TestnetReward {
        mainnet_account: AccountId,
        testnet_node_id: String,
        amount: Dreams,
        epoch: Epoch,
    },

    // ── BLE Tracker Sightings & Claims ───────────────────────────────────────
    /// Privacy-preserving batch commit of BLE tracker sightings from a Pi or Android node.
    /// No MAC addresses stored on-chain — only counts and a batch commitment hash.
    TrackerSightingCommit {
        /// "pi-{hostname}/ble" or "android-{accountId}/ble"
        observer_id: String,
        owner: AccountId,
        airtag_count: u32,
        android_fmd_count: u32,
        tile_count: u32,
        samsung_count: u32,
        other_count: u32,
        /// SHA-256 of the sorted per-sighting commitment hashes (stored locally).
        batch_hash: String,
        epoch: Epoch,
        signed_by: AccountId,
    },
    /// Claim ownership of a BLE tracker. Fee paid to treasury.
    /// The manufacturer serial never appears on-chain — only a commitment hash.
    TrackerClaim {
        /// H(manufacturer_serial || claimer_btcpc_pubkey || nonce) — serial stays off-chain.
        serial_commitment: String,
        /// "AirTag" | "AndroidFMD" | "Tile" | "Samsung" | "Unknown"
        tag_type: String,
        claimer: AccountId,
        fee: Dreams,
        epoch: Epoch,
        nonce: u64,
        signed_by: AccountId,
    },
    /// Release a tracker claim, unlocking the fee for a new claimer.
    TrackerClaimRelease {
        serial_commitment: String,
        claimer: AccountId,
        epoch: Epoch,
        nonce: u64,
        signed_by: AccountId,
    },
    /// Acoustic + BLE co-event proof — signed by the witnessing gateway node.
    /// Upgrades a TrackerClaim from "Registered" to "AcousticVerified".
    TrackerAcousticProof {
        serial_commitment: String,
        /// ID of the Pi or Android node that witnessed the co-event.
        witness_id: String,
        /// H(tag_type || rssi_i16 || acoustic_peak_hz_u32 || acoustic_peak_db_i16
        ///   || challenge_nonce || epoch) — signed locally, verified by any node.
        proof_hash: String,
        claimer: AccountId,
        epoch: Epoch,
        /// Signed by the witness gateway's posting key.
        signed_by: AccountId,
    },
    /// Pay a per-epoch fee to receive push-delivered sighting data for a claimed tracker.
    /// Fee is distributed to observer nodes that sight the tag each epoch.
    /// Requires Verified or AcousticVerified claim status to receive location data.
    TrackerSubscription {
        serial_commitment: String,
        claimer: AccountId,
        /// Dreams burned per epoch from claimer's balance.
        fee_per_epoch: Dreams,
        /// Subscription auto-expires at this epoch.
        expires_epoch: Epoch,
        epoch: Epoch,
        nonce: u64,
        signed_by: AccountId,
    },
    /// Encrypted sighting reference pushed by an observer node.
    /// The encrypted payload is stored in BTCPC-FS; only the CID is gossiped.
    /// Plaintext of the blob: { lat, lon, rssi, timestamp, accuracy_m, observer_id }
    /// Only the subscriber's memo private key can decrypt the blob.
    TrackerSightingData {
        serial_commitment: String,
        observer_id: String,
        /// BTCPC-FS content ID of the encrypted sighting blob.
        cid: String,
        /// SHA-256 of the plaintext (before encryption) so subscriber can verify
        /// integrity after decryption without re-fetching.
        plaintext_hash: String,
        epoch: Epoch,
        /// Signed by the observer's posting key.
        signed_by: AccountId,
    },
    /// Owner publishes current time-window commitment hint so observers can pre-link
    /// rotating MACs to the claim. Gossiped on btcpc/tracker-hints.
    TrackerHint {
        serial_commitment: String,
        /// H(epoch_window || observer_id || rotating_mac) for the current window.
        /// Observers that receive this can match their local sightings to this claim.
        window_commitment: String,
        epoch_window: u64,
        claimer: AccountId,
        epoch: Epoch,
        signed_by: AccountId,
    },
    /// Lost mode: owner sets a bounty and marks the tag as missing.
    TrackerLostMode {
        serial_commitment: String,
        claimer: AccountId,
        /// BTCPC escrowed on-chain, released to finder on confirmation.
        bounty_dreams: Dreams,
        expires_epoch: Epoch,
        /// Contact info encrypted to claimer's own memo key (off-chain recovery).
        contact_encrypted: Option<String>,
        epoch: Epoch,
        nonce: u64,
        signed_by: AccountId,
    },
    /// Finder reports a lost tag at a location (GPS hashed for privacy until confirmed).
    TrackerFoundReport {
        serial_commitment: String,
        finder: AccountId,
        /// H(lat || lon || finder_account || nonce) — revealed after confirmation.
        gps_commitment: String,
        /// Optional acoustic proof hash proving physical proximity.
        acoustic_proof_hash: Option<String>,
        epoch: Epoch,
        nonce: u64,
        signed_by: AccountId,
    },
    /// Owner confirms a finder's report, releasing the bounty escrow.
    TrackerFoundConfirm {
        serial_commitment: String,
        finder: AccountId,
        claimer: AccountId,
        epoch: Epoch,
        nonce: u64,
        signed_by: AccountId,
    },
    /// Per-epoch coverage reward minted to a BLE tracker observer node.
    TrackerCoverageReward {
        observer_id: AccountId,
        amount: Dreams,
        epoch: Epoch,
    },

    // ── Genesis ───────────────────────────────────────────────────────────────
    GenesisAlloc {
        account: AccountId,
        amount: Dreams,
        token: String,
    },

    // ── Scoped Delegation ────────────────────────────────────────────────────
    /// Grant a delegate key permission to act on behalf of the grantor
    /// for a specific set of capabilities until `expires_epoch`.
    /// Capabilities: "Transfer", "Stake", "Inference", "SensorSubmission",
    ///               "DeviceYield", "ServiceHeartbeat", "all"
    DelegationGrant {
        from: AccountId,
        to: AccountId,
        capabilities: Vec<String>,
        expires_epoch: Epoch,
        epoch: Epoch,
        nonce: u64,
        signed_by: AccountId,
    },
    /// Revoke a previously granted delegation immediately.
    DelegationRevoke {
        from: AccountId,
        to: AccountId,
        epoch: Epoch,
        nonce: u64,
        signed_by: AccountId,
    },

    // ── Device Yield Opt-In ──────────────────────────────────────────────────
    /// Device owner opts in to yield-sharing, allowing stakers up to `max_stakers`.
    /// Reward split on device earnings: 70% owner, 20% stakers (pro-rata), 10% recycle.
    DeviceYieldOptIn {
        device_serial: String,
        owner: AccountId,
        max_stakers: u8,
        epoch: Epoch,
        nonce: u64,
        signed_by: AccountId,
    },
    /// Device owner opts out of yield-sharing. Existing yield stakers keep positions
    /// but earn nothing until the owner opts back in.
    DeviceYieldOptOut {
        device_serial: String,
        owner: AccountId,
        epoch: Epoch,
        nonce: u64,
        signed_by: AccountId,
    },

    // ── Permissive Token Model ────────────────────────────────────────────────
    /// Set a BTCPC fee that any sender must pay to deliver an unapproved token.
    /// The fee is deducted from the sender's NATIVE_TOKEN balance and credited
    /// to the recipient before the token lands.  Use token="*" to gate all tokens.
    /// Pre-approved tokens (TokenApprove) bypass the gate entirely.
    SpamGateSet {
        account: AccountId,
        /// Token being gated ("*" = all custom tokens).
        token: String,
        fee: Dreams,
        /// Token the fee must be paid in (e.g. "BTCPC", "USDC", "USDT", "DAI").
        /// Set to "EVM" to require off-chain EVM payment instead (see evm_address / evm_chain_id).
        fee_token: String,
        /// Optional EVM wallet address for off-chain fee payment (e.g. USDC on Ethereum).
        /// When set, senders use SpamGatePayEvm instead of paying on-chain.
        evm_address: Option<String>,
        /// EVM chain ID where the off-chain fee should be paid (1=Ethereum, 137=Polygon, 8453=Base).
        evm_chain_id: Option<u64>,
        epoch: Epoch,
        nonce: u64,
        signed_by: AccountId,
    },
    /// Send a token to an EVM-gated recipient by attesting to an off-chain fee payment.
    /// Debits `amount` of `token` from the sender and creates a pending transfer record
    /// that includes the EVM tx hash.  Recipient verifies the EVM payment independently
    /// then calls TokenAccept to release, or TokenReject to refund.
    SpamGatePayEvm {
        from: AccountId,
        to: AccountId,
        token: String,
        amount: Dreams,
        /// EVM transaction hash proving the fee was paid to the recipient's EVM address.
        evm_tx_hash: String,
        evm_chain_id: u64,
        epoch: Epoch,
        nonce: u64,
        signed_by: AccountId,
    },
    /// Remove a previously set spam gate for a token (or "*").
    SpamGateClear {
        account: AccountId,
        token: String,
        epoch: Epoch,
        nonce: u64,
        signed_by: AccountId,
    },
    /// Pre-approve a token type and/or sender so transfers arrive immediately.
    /// Use "*" as a wildcard: token="*" means any token, from="*" means any sender.
    /// Examples:
    ///   token="USDC"  from="*"     — accept all USDC from anyone
    ///   token="*"     from="alice" — accept any token from alice
    ///   token="*"     from="*"     — fully open (no pending for this account)
    TokenApprove {
        account: AccountId,
        token: String,
        from: String,
        epoch: Epoch,
        nonce: u64,
        signed_by: AccountId,
    },
    /// Revoke a previously granted token approval.
    TokenRevoke {
        account: AccountId,
        token: String,
        from: String,
        epoch: Epoch,
        nonce: u64,
        signed_by: AccountId,
    },
    /// Recipient explicitly accepts a specific pending token transfer.
    TokenAccept {
        to: AccountId,
        token: String,
        from: AccountId,
        pending_nonce: u64,
        epoch: Epoch,
        nonce: u64,
        signed_by: AccountId,
    },
    /// Recipient rejects a pending token transfer — funds return to sender.
    TokenReject {
        to: AccountId,
        token: String,
        from: AccountId,
        pending_nonce: u64,
        epoch: Epoch,
        nonce: u64,
        signed_by: AccountId,
    },

    // ── Wallet Family ─────────────────────────────────────────────────────────
    /// Permanently link a set of BIP44-derived addresses across chains to a BTCPC
    /// account.  Addresses are additive-only — once published they cannot be removed.
    /// The `signature` field for each address proves key ownership (optional but
    /// strongly recommended): sign the canonical string
    /// `"btcpc-family:{account}:{chain}:{address}"` with the corresponding chain key.
    WalletFamilyPublish {
        /// BTCPC account that owns all derived wallets.
        account: AccountId,
        /// Initial set of chain addresses to link.
        chains: Vec<ChainAddress>,
        epoch: Epoch,
        nonce: u64,
        signed_by: AccountId,
    },
    /// Add more chain addresses to an existing wallet family.
    /// Signed by the same account as `WalletFamilyPublish`.
    WalletFamilyAdd {
        account: AccountId,
        chains: Vec<ChainAddress>,
        epoch: Epoch,
        nonce: u64,
        signed_by: AccountId,
    },

    // ── Chain Entropy Protocol ────────────────────────────────────────────────

    /// Prove liveness using a BTCPC key — the cheapest possible "I'm here" signal.
    ///
    /// No external chain involvement. The signer must hold one of the account's
    /// six BTCPC role keys (owner, active, posting, memo, hide, seek). Accepted
    /// entries reset `last_alive_epoch`, stopping half-life decay.
    ///
    /// This is the only user-initiated way to reset the entropy clock without
    /// performing another on-chain action (transfer, stake, mine, etc.).
    /// Cost: one signature. No token movement, no fee.
    LivenessProof {
        account: AccountId,
        epoch: Epoch,
        nonce: u64,
        /// Which key role is signing: "owner" | "active" | "posting" | "memo" | "hide" | "seek"
        key_role: String,
        signed_by: AccountId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },

    /// Reset the liveness clock from observed external-chain activity.
    ///
    /// Any BTCPC node that detects a transaction on a published wallet-family
    /// address (WalletFamilyPublish / WalletFamilyAdd) can submit this entry.
    /// The account owner doesn't need to be online — their published cross-chain
    /// addresses are the proof. Once accepted, `last_alive_epoch` resets and
    /// half-life decay is paused.
    ///
    /// The submitting node signs with their own posting key. Multiple nodes may
    /// independently submit witnesses for the same activity — all are idempotent
    /// (decay clock resets to max of current and witnessed epoch).
    EntropyWitness {
        /// BTCPC account whose liveness is being attested.
        /// Must have a published wallet-family entry for `chain`:`address`.
        account: AccountId,
        /// External chain: "ethereum", "bitcoin", "solana", "base", etc.
        chain: String,
        /// External address that was active (must match a published wallet-family entry).
        address: String,
        /// External chain transaction hash proving the observed activity.
        tx_hash: String,
        /// External chain block height at which the activity occurred.
        block_height: u64,
        epoch: Epoch,
        /// BTCPC node submitting this witness (must sign with their posting key).
        signed_by: AccountId,
    },
    /// Bind a hardware fingerprint to an account.
    /// One fingerprint can only be claimed by one account — chain rejects duplicates
    /// from a different account, preventing multiple profiles per physical machine.
    /// fingerprint = SHA-256(gpu_serial | machine_id | ...) — stable per hardware.
    HardwareClaim {
        account: AccountId,
        /// SHA-256 hex of stable hardware identifiers (GPU serial, machine-id, etc.)
        fingerprint: String,
        /// Human-readable summary of detected hardware (for display only, not enforced)
        hw_info: String,
        epoch: Epoch,
        signed_by: AccountId,
    },
}

/// A single chain address within a wallet family.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainAddress {
    /// Chain identifier: "evm", "bitcoin", "solana", "monero", "btcpc", etc.
    pub chain: String,
    /// Address on that chain (format is chain-specific).
    pub address: String,
    /// BIP44 derivation path used (e.g. "m/44'/60'/0'/0/0" for EVM).
    pub derivation_path: Option<String>,
    /// Signature proving ownership: sign `"btcpc-family:{account}:{chain}:{address}"`.
    pub signature: Option<String>,
}

impl LedgerEntry {
    pub fn epoch(&self) -> Epoch {
        match self {
            Self::AccountCreate { epoch, .. } => *epoch,
            Self::AccountUpdateKey { epoch, .. } => *epoch,
            Self::Transfer { epoch, .. } => *epoch,
            Self::Stake { epoch, .. } => *epoch,
            Self::Unstake { epoch, .. } => *epoch,
            Self::NodeRoleOptIn { epoch, .. } => *epoch,
            Self::NodeRoleOptOut { epoch, .. } => *epoch,
            Self::NodeRoleStake { epoch, .. } => *epoch,
            Self::NodeRoleUnstake { epoch, .. } => *epoch,
            Self::Mine { epoch, .. } => *epoch,
            Self::MineReward { epoch, .. } => *epoch,
            Self::EpochSeal { epoch, .. } => *epoch,
            Self::EpochFinalize { epoch, .. } => *epoch,
            Self::ContractDeploy { epoch, .. } => *epoch,
            Self::ContractCall { epoch, .. } => *epoch,
            Self::SensorReading { epoch, .. } => *epoch,
            Self::CoverageReport { epoch, .. } => *epoch,
            Self::InferenceJobPost { epoch, .. } => *epoch,
            Self::InferenceJobBid { epoch, .. } => *epoch,
            Self::InferenceJobAward { epoch, .. } => *epoch,
            Self::InferenceJobComplete { epoch, .. } => *epoch,
            Self::InferenceVerifyClaim { epoch, .. } => *epoch,
            Self::InferenceJobCommit { epoch, .. } => *epoch,
            Self::InferenceJobVerify { epoch, .. } => *epoch,
            Self::InferenceJobClaim { epoch, .. } => *epoch,
            Self::InferenceReviewVote { epoch, .. } => *epoch,
            Self::InferenceJobPay { epoch, .. } => *epoch,
            Self::InferenceJobCancel { epoch, .. } => *epoch,
            Self::ClockReward { epoch, .. } => *epoch,
            Self::StorageReward { epoch, .. } => *epoch,
            Self::SensorReward { epoch, .. } => *epoch,
            Self::VerifierReward { epoch, .. } => *epoch,
            Self::ServiceReward { epoch, .. } => *epoch,
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
            Self::ServiceHeartbeat { epoch, .. } => *epoch,
            Self::SensorDataPurchase { epoch, .. } => *epoch,
            Self::LinkGitRepoCreate { epoch, .. } => *epoch,
            Self::LinkGitRefUpdate { epoch, .. } => *epoch,
            Self::LinkGitAccessGrant { epoch, .. } => *epoch,
            Self::LinkGitAccessRevoke { epoch, .. } => *epoch,
            Self::LinkGitPruneProof { epoch, .. } => *epoch,
            Self::LinkGitStorageExtend { epoch, .. } => *epoch,
            Self::TestnetOperatorRegister { epoch, .. } => *epoch,
            Self::TestnetReward { epoch, .. } => *epoch,
            Self::MempoolReward { epoch, .. } => *epoch,
            Self::MempoolOperatorRegister { epoch, .. } => *epoch,
            Self::MempoolHeartbeat { epoch, .. } => *epoch,
            Self::DeviceClaimStake { epoch, .. } => *epoch,
            Self::DeviceClaimUnstake { epoch, .. } => *epoch,
            Self::TrackerSightingCommit { epoch, .. } => *epoch,
            Self::TrackerClaim { epoch, .. } => *epoch,
            Self::TrackerClaimRelease { epoch, .. } => *epoch,
            Self::TrackerAcousticProof { epoch, .. } => *epoch,
            Self::TrackerSubscription { epoch, .. } => *epoch,
            Self::TrackerSightingData { epoch, .. } => *epoch,
            Self::TrackerHint { epoch, .. } => *epoch,
            Self::TrackerLostMode { epoch, .. } => *epoch,
            Self::TrackerFoundReport { epoch, .. } => *epoch,
            Self::TrackerFoundConfirm { epoch, .. } => *epoch,
            Self::TrackerCoverageReward { epoch, .. } => *epoch,
            Self::DelegationGrant { epoch, .. } => *epoch,
            Self::DelegationRevoke { epoch, .. } => *epoch,
            Self::DeviceYieldOptIn { epoch, .. } => *epoch,
            Self::DeviceYieldOptOut { epoch, .. } => *epoch,
            Self::SpamGateSet { epoch, .. } => *epoch,
            Self::SpamGatePayEvm { epoch, .. } => *epoch,
            Self::SpamGateClear { epoch, .. } => *epoch,
            Self::TokenApprove { epoch, .. } => *epoch,
            Self::TokenRevoke { epoch, .. } => *epoch,
            Self::TokenAccept { epoch, .. } => *epoch,
            Self::TokenReject { epoch, .. } => *epoch,
            Self::WalletFamilyPublish { epoch, .. } => *epoch,
            Self::WalletFamilyAdd { epoch, .. } => *epoch,
            Self::AccountSetPrimary { epoch, .. } => *epoch,
            Self::AccountTransfer { epoch, .. } => *epoch,
            Self::ChainParameterSet { epoch, .. } => *epoch,
            Self::GenesisAlloc { .. } => 0,
            Self::VerifyChainLink { epoch, .. } => *epoch,
            Self::SetKeyPolicy { epoch, .. } => *epoch,
            Self::LivenessProof { epoch, .. } => *epoch,
            Self::EntropyWitness { epoch, .. } => *epoch,
            Self::HardwareClaim { epoch, .. } => *epoch,
            Self::ClockNodeRegister { epoch, .. } => *epoch,
            Self::ClockDoubleSignEvidence { epoch, .. } => *epoch,
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

/// Entry processing weight for the dynamic fee market (T3-1, Phase 5).
///
/// Fee = `base_fee_dreams × entry_weight(entry)`.
/// System entries always return 0 — they are never fee-charged.
pub fn entry_weight(entry: &LedgerEntry) -> u64 {
    use crate::emission::{
        ENTRY_WEIGHT_SYSTEM, ENTRY_WEIGHT_MICRO, ENTRY_WEIGHT_STANDARD,
        ENTRY_WEIGHT_HEAVY, ENTRY_WEIGHT_BULK, ENTRY_WEIGHT_REGISTRATION,
    };
    match entry {
        // ── System (0): generated by the protocol, never submitted by users ──
        LedgerEntry::EpochSeal { .. }
        | LedgerEntry::EpochFinalize { .. }
        | LedgerEntry::MineReward { .. }
        | LedgerEntry::ClockReward { .. }
        | LedgerEntry::StorageReward { .. }
        | LedgerEntry::SensorReward { .. }
        | LedgerEntry::VerifierReward { .. }
        | LedgerEntry::ServiceReward { .. }
        | LedgerEntry::MempoolReward { .. }
        | LedgerEntry::TrackerCoverageReward { .. }
        | LedgerEntry::GenesisAlloc { .. }
        | LedgerEntry::TestnetReward { .. } => ENTRY_WEIGHT_SYSTEM,

        // ── Micro (1): lightweight single-field state transitions ─────────────
        LedgerEntry::Transfer { .. }
        | LedgerEntry::LivenessProof { .. }
        | LedgerEntry::EntropyWitness { .. }
        | LedgerEntry::WalletFamilyPublish { .. }
        | LedgerEntry::WalletFamilyAdd { .. }
        | LedgerEntry::VerifyChainLink { .. }
        | LedgerEntry::SetKeyPolicy { .. }
        | LedgerEntry::DelegationGrant { .. }
        | LedgerEntry::DelegationRevoke { .. }
        | LedgerEntry::ChainParameterSet { .. }
        | LedgerEntry::AccountTransfer { .. } => ENTRY_WEIGHT_MICRO,

        // ── Standard (3): useful-work submissions + moderate state ────────────
        LedgerEntry::Stake { .. }
        | LedgerEntry::Unstake { .. }
        | LedgerEntry::NodeRoleStake { .. }
        | LedgerEntry::NodeRoleUnstake { .. }
        | LedgerEntry::Mine { .. }
        | LedgerEntry::SensorDataCommit { .. }
        | LedgerEntry::CoverageReport { .. }
        | LedgerEntry::GatewayHeartbeat { .. }
        | LedgerEntry::ServiceHeartbeat { .. }
        | LedgerEntry::MempoolHeartbeat { .. }
        | LedgerEntry::TrackerSightingCommit { .. }
        | LedgerEntry::TrackerClaim { .. }
        | LedgerEntry::TrackerClaimRelease { .. }
        | LedgerEntry::TrackerHint { .. }
        | LedgerEntry::TrackerFoundReport { .. }
        | LedgerEntry::TrackerFoundConfirm { .. }
        | LedgerEntry::DeviceClaimUnstake { .. }
        | LedgerEntry::DeviceYieldUnstake { .. }
        | LedgerEntry::DeviceYieldOptOut { .. }
        | LedgerEntry::SpamGateClear { .. }
        | LedgerEntry::TokenAccept { .. }
        | LedgerEntry::TokenReject { .. }
        | LedgerEntry::StoreUpdate { .. }
        | LedgerEntry::ProductUpdate { .. }
        | LedgerEntry::OrderFulfill { .. }
        | LedgerEntry::OrderCancel { .. } => ENTRY_WEIGHT_STANDARD,

        // ── Heavy (5): multi-party coordination + significant indexing ─────────
        LedgerEntry::StorageHeartbeat { .. }
        | LedgerEntry::InferenceJobPost { .. }
        | LedgerEntry::InferenceJobBid { .. }
        | LedgerEntry::InferenceJobAward { .. }
        | LedgerEntry::InferenceJobComplete { .. }
        | LedgerEntry::InferenceJobCommit { .. }
        | LedgerEntry::InferenceJobVerify { .. }
        | LedgerEntry::InferenceJobPay { .. }
        | LedgerEntry::InferenceJobCancel { .. }
        | LedgerEntry::InferenceJobClaim { .. }
        | LedgerEntry::InferenceReviewVote { .. }
        | LedgerEntry::InferenceVerifyClaim { .. }
        | LedgerEntry::TrackerAcousticProof { .. }
        | LedgerEntry::TrackerSubscription { .. }
        | LedgerEntry::TrackerSightingData { .. }
        | LedgerEntry::TrackerLostMode { .. }
        | LedgerEntry::OrderPlace { .. }
        | LedgerEntry::OrderDispute { .. }
        | LedgerEntry::EscrowRelease { .. }
        | LedgerEntry::SpamGatePayEvm { .. }
        | LedgerEntry::SensorDataPurchase { .. } => ENTRY_WEIGHT_HEAVY,

        // ── Bulk (10): large data operations ─────────────────────────────────
        LedgerEntry::BlobStore { .. }
        | LedgerEntry::LinkGitRefUpdate { .. }
        | LedgerEntry::LinkGitRepoCreate { .. }
        | LedgerEntry::LinkGitAccessGrant { .. }
        | LedgerEntry::LinkGitAccessRevoke { .. }
        | LedgerEntry::LinkGitPruneProof { .. }
        | LedgerEntry::LinkGitStorageExtend { .. } => ENTRY_WEIGHT_BULK,

        // ── Registration (20): one-time identity + capacity entries ───────────
        LedgerEntry::AccountCreate { .. }
        | LedgerEntry::AccountUpdateKey { .. }
        | LedgerEntry::AccountSetPrimary { .. }
        | LedgerEntry::ClockNodeRegister { .. }
        | LedgerEntry::ClockDoubleSignEvidence { .. }
        | LedgerEntry::SensorRegister { .. }
        | LedgerEntry::SensorKeyRegister { .. }
        | LedgerEntry::SensorVouch { .. }
        | LedgerEntry::SensorReading { .. }
        | LedgerEntry::DeviceKeyRegister { .. }
        | LedgerEntry::DeviceClaimStake { .. }
        | LedgerEntry::DeviceYieldStake { .. }
        | LedgerEntry::DeviceYieldOptIn { .. }
        | LedgerEntry::NodeRoleOptIn { .. }
        | LedgerEntry::NodeRoleOptOut { .. }
        | LedgerEntry::MempoolOperatorRegister { .. }
        | LedgerEntry::HardwareClaim { .. }
        | LedgerEntry::ContractDeploy { .. }
        | LedgerEntry::ContractCall { .. }
        | LedgerEntry::FlashSale { .. }
        | LedgerEntry::ProductCreate { .. }
        | LedgerEntry::TestnetOperatorRegister { .. }
        | LedgerEntry::SpamGateSet { .. }
        | LedgerEntry::TokenApprove { .. }
        | LedgerEntry::TokenRevoke { .. } => ENTRY_WEIGHT_REGISTRATION,
    }
}
