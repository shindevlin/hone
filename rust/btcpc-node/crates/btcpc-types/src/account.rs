use serde::{Deserialize, Serialize};

pub type AccountId = String;
pub type TokenSymbol = String;
/// Balance in dreams (1 BTCPC = 10_000_000_000 dreams)
pub type Dreams = u64;
#[deprecated = "use Dreams"]
pub type Satoshis = Dreams;
pub type Epoch = u64;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AccountState {
    pub account_id: AccountId,
    pub created_epoch: Epoch,
    pub public_key: Option<String>,
    pub nonce: u64,
    pub stake: Satoshis,
    pub delegated_to: Option<AccountId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MinerStats {
    pub account_id: AccountId,
    pub total_blocks: u64,
    pub total_work: u64,
    pub last_block_epoch: Epoch,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpochMeta {
    pub epoch_number: Epoch,
    pub started_at: u64,
    pub ended_at: Option<u64>,
    pub block_hash: Option<String>,
    pub miner: Option<AccountId>,
    pub entry_count: u64,
    pub total_work: u64,
    pub sealed: bool,
    pub finalized: bool,
}
