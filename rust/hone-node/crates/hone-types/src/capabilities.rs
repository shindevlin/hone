//! Capability-based authorization (PROPOSED â€” not yet wired into `validate_and_apply`).
//!
//! Two layers (docs/KEY_AUTHORIZATION.md):
//!   1. `Capability` â€” the fixed vocabulary. Every ledger entry maps to exactly one
//!      via `required_capability`, an EXHAUSTIVE match (no `_` arm) so a new entry
//!      type cannot be added without assigning it a capability.
//!   2. `KeyType` + the registry (`grants`, `rank`, `rotatable_by`) â€” extensible:
//!      new key types are added by governance over the existing capability set, no
//!      fork. `authorize` applies the no-downgrade rule.
//!
//! Classification principle: when an entry's fund impact is ambiguous it is
//! classified `Value` (fail CLOSED â€” requires the active key), never `Operate`.
//! The policy table below is a FIRST PASS pending Shin/Josh audit before wiring.

use crate::LedgerEntry;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    /// Protocol-emitted; never carries a user signature (rewards, seals, finalize).
    System,
    /// Change keys / key policy / identity key set. Highest.
    Rotate,
    /// Chain governance: params, slashing adjudication, feed/operator admin.
    Govern,
    /// Move value â€” HONE, stake, tokens, name-NFTs, escrow, bridge, credits.
    Value,
    /// Everyday account operations (mine, storefront, git, register, set-primaryâ€¦).
    Operate,
    /// Fulfill/deliver + release escrow after fulfillment.
    Deliver,
    /// Liveness only â€” heartbeats, proofs, commits that prove a node runs.
    Service,
    /// Encrypted messaging / purchase intent.
    Message,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyType {
    Owner, Active, Posting, Memo, Hide, Seek, Fulfill, Service, Device,
}

impl KeyType {
    /// Registry default grants. (At runtime this comes from the on-chain key-type
    /// registry; these are the genesis-seeded rows.)
    pub fn grants(self) -> &'static [Capability] {
        use Capability::*;
        match self {
            KeyType::Owner   => &[Rotate, Govern, Value, Operate, Deliver, Service, Message],
            KeyType::Active  => &[Value, Operate, Deliver, Service, Message],
            KeyType::Posting => &[Operate, Deliver, Service, Message],
            KeyType::Memo    => &[Message],
            KeyType::Seek    => &[Deliver],
            KeyType::Fulfill => &[Deliver],
            KeyType::Service => &[Service],
            KeyType::Device  => &[Service],
            KeyType::Hide    => &[], // a decryption key, never an entry signer
        }
    }
    /// Authority rank (higher = more privileged); used for rotation checks.
    pub fn rank(self) -> u8 {
        match self {
            KeyType::Owner => 100, KeyType::Active => 80, KeyType::Posting => 60,
            KeyType::Memo | KeyType::Hide | KeyType::Seek => 40,
            KeyType::Fulfill => 30, KeyType::Service | KeyType::Device => 20,
        }
    }
    /// Which key type may rotate this one. `None` = founder multisig (owner).
    pub fn rotatable_by(self) -> Option<KeyType> {
        match self {
            KeyType::Owner => None, // founder 2-of-3 multisig
            KeyType::Active | KeyType::Posting => Some(KeyType::Owner),
            _ => Some(KeyType::Posting), // delegated keys: safely losable
        }
    }
}

/// A signature from `signer` authorizes an entry requiring `required` iff the
/// signer's key type grants that capability. No-downgrade is intrinsic: lower key
/// types simply do not list higher capabilities (posting never lists `Value`).
pub fn authorize(signer: KeyType, required: Capability) -> bool {
    // System entries are protocol-emitted and never user-authorized.
    required != Capability::System && signer.grants().contains(&required)
}

/// The capability each ledger entry requires. EXHAUSTIVE â€” no catch-all.
pub fn required_capability(entry: &LedgerEntry) -> Capability {
    use Capability::*;
    use LedgerEntry as E;
    match entry {
        // â”€â”€ Rotate: keys & identity key set â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
        E::AccountCreate { .. } | E::AccountUpdateKey { .. } | E::AccountTransfer { .. }
        | E::SetKeyPolicy { .. } | E::GrantExternalSigner { .. } | E::RevokeExternalSigner { .. }
        | E::WalletFamilyPublish { .. } | E::WalletFamilyAdd { .. }
        | E::DelegationGrant { .. } | E::DelegationRevoke { .. }
        | E::PrivateAuthEnroll { .. } => Rotate,

        // â”€â”€ Govern: chain governance / protocol admin â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
        E::ChainParameterSet { .. } | E::GovernancePropose { .. } | E::GovernanceVote { .. }
        | E::GovernanceFinalize { .. } | E::SlashValidator { .. } | E::SlashAppeal { .. }
        | E::ClockDoubleSignEvidence { .. } | E::SpamGateSet { .. } | E::SpamGateClear { .. }
        | E::OracleFeedCreate { .. } | E::OracleFeedFinalize { .. }
        | E::TestnetOperatorRegister { .. } | E::TestnetOperatorDeregister { .. }
        | E::PrivateAuthApprove { .. } => Govern,

        // â”€â”€ Value: anything that moves funds, stake, tokens, names, escrow â”€â”€â”€
        E::Transfer { .. } | E::Stake { .. } | E::Unstake { .. }
        | E::NodeRoleStake { .. } | E::NodeRoleUnstake { .. }
        | E::DeviceClaimStake { .. } | E::DeviceClaimUnstake { .. }
        | E::DeviceYieldStake { .. } | E::DeviceYieldUnstake { .. }
        | E::TokenApprove { .. } | E::TokenRevoke { .. } | E::TokenAccept { .. } | E::TokenReject { .. }
        | E::BridgeFund { .. } | E::BridgeWrap { .. } | E::BridgeUnwrap { .. } | E::BridgeUnlock { .. }
        | E::AgentCreditDeposit { .. } | E::AgentCreditWithdraw { .. }
        | E::ExternalSignerTransfer { .. } | E::SpamGatePayEvm { .. }
        | E::SensorDataPurchase { .. } | E::InferenceJobPay { .. } | E::InferenceJobPost { .. }
        | E::InferenceJobBid { .. } | E::InferenceJobAward { .. }
        | E::EnsembleJobPost { .. } | E::FineTuneJobPost { .. } | E::ComputerUseJobPost { .. }
        | E::AgentTaskPost { .. } | E::AgentTaskBid { .. } | E::AgentTaskSettle { .. }
        | E::TrackerSubscription { .. }
        | E::SessionListingBuy { .. }
        | E::NameAuctionBid { .. } | E::NameAuctionSettle { .. }
        | E::FreeportAuctionBid { .. } | E::FreeportAuctionSettle { .. }
        | E::IdentityNftTransfer { .. }
        | E::IdentityNftSetRecovery { .. } | E::IdentityNftLock { .. } | E::IdentityNftUnlock { .. }
        | E::IdentityNftRecover { .. } | E::IdentityNftSetBackstop { .. } => Value,

        // â”€â”€ Deliver: fulfillment + escrow resolution â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
        E::OrderFulfill { .. } | E::EscrowRelease { .. }
        | E::OrderCancel { .. } | E::OrderDispute { .. } => Deliver,

        // â”€â”€ Message: encrypted messaging / purchase intent â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
        E::OrderPlace { .. } | E::MemorySet { .. } | E::MemoryDelete { .. } => Message,

        // â”€â”€ Service: liveness, heartbeats, proofs, data commits â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
        E::MempoolHeartbeat { .. } | E::GatewayHeartbeat { .. } | E::StorageHeartbeat { .. }
        | E::ServiceHeartbeat { .. } | E::LinkGitServeHeartbeat { .. }
        | E::SensorReading { .. } | E::SensorDataCommit { .. }
        | E::HiveReplicaCommit { .. } | E::HiveReplicaVerify { .. }
        | E::RuntimeAttest { .. } | E::LivenessProof { .. } | E::EntropyWitness { .. }
        | E::HardwareClaim { .. } | E::PhoneStorageProof { .. } | E::BlobServeProof { .. }
        | E::LinkGitPruneProof { .. } | E::VrfCommit { .. } | E::VrfReveal { .. }
        | E::OracleReport { .. } | E::SnapshotSave { .. } | E::ClockNodeRegister { .. } => Service,

        // â”€â”€ System: protocol-emitted, never user-signed â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
        E::MineReward { .. } | E::EpochSeal { .. } | E::EpochFinalize { .. } | E::ClockReward { .. }
        | E::StorageReward { .. } | E::SensorReward { .. } | E::VerifierReward { .. }
        | E::ServiceReward { .. } | E::MempoolReward { .. } | E::RuntimeReward { .. }
        | E::LinkGitServeReward { .. } | E::LinkGitBuildReward { .. } | E::TrackerCoverageReward { .. }
        | E::TestnetReward { .. } | E::GatewayRewardSplit { .. } | E::GenesisAlloc { .. }
        | E::RuntimeSlash { .. } | E::CrossChainFinalityAnnounce { .. } | E::TonWalletActivated { .. } => System,

        // â”€â”€ Operate: everything else (daily operations) â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
        E::AccountSetPrimary { .. } | E::AccountApiKeySet { .. } | E::VerifyChainLink { .. }
        | E::NodeRoleOptIn { .. } | E::NodeRoleOptOut { .. }
        | E::Mine { .. } | E::PhoneMineSubmit { .. }
        | E::ContractDeploy { .. } | E::ContractCall { .. }
        | E::CoverageReport { .. }
        | E::InferenceJobComplete { .. } | E::InferenceVerifyClaim { .. } | E::InferenceJobCommit { .. }
        | E::InferenceJobVerify { .. } | E::InferenceJobClaim { .. } | E::InferenceReviewVote { .. }
        | E::InferenceJobCancel { .. }
        | E::MempoolOperatorRegister { .. } | E::BlobStore { .. }
        | E::StoreUpdate { .. } | E::ProductCreate { .. } | E::ProductUpdate { .. } | E::FlashSale { .. }
        | E::SensorRegister { .. } | E::SensorKeyRegister { .. } | E::SensorVouch { .. }
        | E::DeviceKeyRegister { .. } | E::DeviceYieldOptIn { .. } | E::DeviceYieldOptOut { .. }
        | E::RuntimeRegister { .. } | E::RuntimeDeploy { .. } | E::RuntimeUndeploy { .. }
        | E::RuntimeJobEnqueue { .. } | E::RuntimeClaim { .. } | E::RuntimeChallenge { .. }
        | E::LinkGitRepoCreate { .. } | E::LinkGitRefUpdate { .. }
        | E::LinkGitAccessGrant { .. } | E::LinkGitAccessRevoke { .. } | E::LinkGitStorageExtend { .. }
        | E::LinkGitIssueCreate { .. } | E::LinkGitIssueComment { .. } | E::LinkGitIssueClose { .. }
        | E::LinkGitIssueReopen { .. } | E::LinkGitPrCreate { .. } | E::LinkGitPrComment { .. }
        | E::LinkGitPrMerge { .. } | E::LinkGitPrClose { .. }
        | E::ProjectCreate { .. } | E::ProjectTask { .. }
        | E::TaskClaim { .. } | E::TaskSubmit { .. } | E::TaskApprove { .. }
        | E::TrackerSightingCommit { .. } | E::TrackerClaim { .. } | E::TrackerClaimRelease { .. }
        | E::TrackerAcousticProof { .. } | E::TrackerSightingData { .. } | E::TrackerHint { .. }
        | E::TrackerLostMode { .. } | E::TrackerFoundReport { .. } | E::TrackerFoundConfirm { .. }
        | E::ScientificResult { .. } | E::EnsembleVote { .. }
        | E::AgentRegister { .. } | E::AgentDeregister { .. }
        | E::AgentTaskAssign { .. } | E::AgentTaskSubmit { .. }
        | E::AgentTaskVerifierCommit { .. } | E::AgentTaskVerifierReveal { .. }
        | E::AgentSessionOpen { .. } | E::AgentSessionClose { .. }
        | E::SessionListingCreate { .. } | E::SessionListingCancel { .. }
        | E::NameAuctionOpen { .. } | E::NameAuctionCancel { .. }
        | E::FreeportAuctionOpen { .. }
        | E::TonActivationIntent { .. } | E::AmberPillMint { .. }
        | E::FineTuneJobComplete { .. } | E::ComputerUseJobComplete { .. }
        | E::IdentityNftMint { .. } | E::IdentityNftAccept { .. } | E::IdentityNftReject { .. }
        | E::IdentityNftSetPrimary { .. }
        | E::SoulboundIssue { .. } | E::SoulboundAccept { .. }
        | E::SoulboundReject { .. } | E::SoulboundRevoke { .. } => Operate,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Capability::*;

    #[test]
    fn posting_can_never_move_value_or_rotate() {
        assert!(!authorize(KeyType::Posting, Value));
        assert!(!authorize(KeyType::Posting, Rotate));
        assert!(!authorize(KeyType::Posting, Govern));
        assert!(authorize(KeyType::Posting, Operate));
        assert!(authorize(KeyType::Posting, Service));
    }

    #[test]
    fn value_needs_active_or_owner() {
        assert!(authorize(KeyType::Active, Value));
        assert!(authorize(KeyType::Owner, Value));
        for k in [KeyType::Posting, KeyType::Memo, KeyType::Seek, KeyType::Service, KeyType::Device] {
            assert!(!authorize(k, Value), "{k:?} must not move value");
        }
    }

    #[test]
    fn rotate_is_owner_only() {
        assert!(authorize(KeyType::Owner, Rotate));
        for k in [KeyType::Active, KeyType::Posting] {
            assert!(!authorize(k, Rotate));
        }
    }

    #[test]
    fn service_keys_are_liveness_only() {
        for k in [KeyType::Service, KeyType::Device] {
            assert!(authorize(k, Service));
            assert!(!authorize(k, Value));
            assert!(!authorize(k, Operate));
        }
    }

    #[test]
    fn system_entries_never_user_authorized() {
        for k in [KeyType::Owner, KeyType::Active, KeyType::Posting] {
            assert!(!authorize(k, System));
        }
    }

    #[test]
    fn identity_nft_transfer_is_value_others_are_operate() {
        use LedgerEntry as E;
        // A name is an asset: moving it needs Value (active/owner), never posting.
        assert_eq!(required_capability(&E::IdentityNftTransfer {
            name: "x".into(), from: "a".into(), to: "b".into(),
            epoch: 0, nonce: 0, signed_by: "a".into(), signature: None,
        }), Capability::Value);
        assert!(!authorize(KeyType::Posting, Capability::Value));
        // Minting / receipt / primary are everyday Operate actions.
        assert_eq!(required_capability(&E::IdentityNftMint {
            name: "x".into(), to: "a".into(),
            epoch: 0, nonce: 0, signed_by: "a".into(), signature: None,
        }), Capability::Operate);
        assert_eq!(required_capability(&E::IdentityNftSetPrimary {
            account: "a".into(), name: "x".into(),
            epoch: 0, nonce: 0, signed_by: "a".into(), signature: None,
        }), Capability::Operate);
    }
}
