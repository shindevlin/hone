use alloy::{
    network::EthereumWallet,
    primitives::{Address, U256},
    providers::{Provider, ProviderBuilder},
    signers::local::PrivateKeySigner,
    sol,
};
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use tracing::debug;

sol! {
    #[sol(rpc)]
    interface IBetChu {
        // --- Views ---
        function betPoolCount() external view returns (uint256);

        // Must match deployed BetChuBettingPool ABI exactly (19 fields in order)
        function betPools(uint256 poolId) external view returns (
            address owner,
            string description,
            uint256 creationTime,
            uint256 deadline,
            uint8 betType,
            uint8 bettingCurrency,
            bool acceptsBothCurrencies,
            uint256 exchangeRateAtCreation,
            uint256 sideAPool,
            uint256 sideBPool,
            bool isResolved,
            bool isCancelled,
            uint8 result,
            uint256 resolutionTime,
            uint256 exchangeRateAtSettlement,
            address counterBettor,
            bool needsCounter,
            uint256 minBetAmount,
            string betTerms
        );

        function getPoolBetsByAddress(uint256 poolId, address bettor)
            external view returns (uint256 sideAAmount, uint256 sideBAmount);

        // --- Writes ---
        // deadline is an absolute unix timestamp
        function createBetPool(
            string calldata description,
            uint256 deadline,
            uint8 betType,
            uint8 bettingCurrency,
            bool acceptsBothCurrencies,
            uint256 exchangeRateAtCreation
        ) external payable returns (uint256 poolId);

        function placeBet(uint256 poolId, uint8 side) external payable;

        // Oracle calls this once ESPN confirms game is final
        function reportScore(uint256 poolId, uint8 result) external;

        function cancelBet(uint256 poolId) external;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BetPool {
    pub pool_id: u32,
    pub owner: String,
    pub description: String,
    pub deadline: u64,
    pub side_a_pool: f64,
    pub side_b_pool: f64,
    pub is_resolved: bool,
    pub is_cancelled: bool,
    pub result: u8,
    pub betting_currency: u8,
    pub bet_type: u8,
}

pub struct Currency {
    pub id: u8,
    pub symbol: &'static str,
}

impl BetPool {
    pub fn currency(&self) -> Currency {
        match self.betting_currency {
            2 => Currency { id: 2, symbol: "USDC" },
            _ => Currency { id: 1, symbol: "ETH" },
        }
    }

    pub fn bet_type_name(&self) -> &'static str {
        match self.bet_type {
            1 => "MATCHED",
            _ => "POOL",
        }
    }

    pub fn hours_left(&self) -> f64 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let remaining = self.deadline.saturating_sub(now);
        remaining as f64 / 3600.0
    }

    pub fn is_active(&self) -> bool {
        !self.is_resolved && !self.is_cancelled
    }

    /// Deadline has passed but not yet resolved — ready for oracle check.
    pub fn needs_resolution(&self) -> bool {
        !self.is_resolved && !self.is_cancelled && self.hours_left() <= 0.0
    }

    pub fn total_pool(&self) -> f64 {
        self.side_a_pool + self.side_b_pool
    }
}

pub struct ContractService {
    rpc_url: String,
    contract_address: Address,
    wallet: Option<EthereumWallet>,
}

impl ContractService {
    pub fn new(rpc_url: String, contract_address: String, private_key: &str) -> Result<Self> {
        let addr = Address::from_str(&contract_address)
            .map_err(|e| anyhow!("Invalid contract address: {}", e))?;

        let wallet = if !private_key.is_empty() {
            let key = private_key.trim_start_matches("0x");
            let signer = PrivateKeySigner::from_str(key)
                .map_err(|e| anyhow!("Invalid private key: {}", e))?;
            Some(EthereumWallet::from(signer))
        } else {
            None
        };

        Ok(Self { rpc_url, contract_address: addr, wallet })
    }

    async fn read_provider(&self) -> Result<impl Provider> {
        let url = self.rpc_url.parse().map_err(|e| anyhow!("Invalid RPC URL: {}", e))?;
        Ok(ProviderBuilder::new().connect_http(url))
    }

    async fn write_provider(&self) -> Result<impl Provider> {
        let wallet = self
            .wallet
            .as_ref()
            .ok_or_else(|| anyhow!("No private key configured"))?;
        let url = self.rpc_url.parse().map_err(|e| anyhow!("Invalid RPC URL: {}", e))?;
        Ok(ProviderBuilder::new()
            .wallet(wallet.clone())
            .connect_http(url))
    }

    pub async fn get_pool_count(&self) -> Result<u32> {
        let provider = self.read_provider().await?;
        let contract = IBetChu::new(self.contract_address, provider);
        let count = contract.betPoolCount().call().await
            .map_err(|e| anyhow!("betPoolCount: {}", e))?;
        let n: u64 = count.try_into().unwrap_or(0u64);
        Ok(n as u32)
    }

    pub async fn get_pool(&self, pool_id: u32) -> Result<BetPool> {
        let provider = self.read_provider().await?;
        let contract = IBetChu::new(self.contract_address, provider);
        let p = contract.betPools(U256::from(pool_id)).call().await
            .map_err(|e| anyhow!("betPools({}): {}", pool_id, e))?;

        Ok(BetPool {
            pool_id,
            owner: format!("{:?}", p.owner),
            description: p.description,
            deadline: p.deadline.try_into().unwrap_or(0),
            side_a_pool: wei_to_eth(p.sideAPool),
            side_b_pool: wei_to_eth(p.sideBPool),
            is_resolved: p.isResolved,
            is_cancelled: p.isCancelled,
            result: p.result,
            betting_currency: p.bettingCurrency,
            bet_type: p.betType,
        })
    }

    pub async fn get_active_pools(&self, limit: u32) -> Result<Vec<BetPool>> {
        let count = self.get_pool_count().await?;
        let mut pools = Vec::new();
        let start = count.saturating_sub(limit * 3);

        for id in (start..count).rev() {
            if pools.len() >= limit as usize {
                break;
            }
            match self.get_pool(id).await {
                Ok(p) if p.is_active() => pools.push(p),
                Ok(_) => {}
                Err(e) => debug!("Pool {}: {}", id, e),
            }
        }
        Ok(pools)
    }

    pub async fn create_bet_pool(
        &self,
        description: &str,
        deadline: u64,          // absolute unix timestamp
        currency: u8,
        bet_type: u8,
        amount_wei: U256,
    ) -> Result<u32> {
        let provider = self.write_provider().await?;
        let contract = IBetChu::new(self.contract_address, provider);

        let tx = contract
            .createBetPool(
                description.to_string(),
                U256::from(deadline),
                bet_type,
                currency,
                false,          // acceptsBothCurrencies
                U256::ZERO,     // exchangeRateAtCreation (not used for ETH)
            )
            .value(amount_wei)
            .send()
            .await
            .map_err(|e| anyhow!("createBetPool: {}", e))?;

        tx.get_receipt().await.map_err(|e| anyhow!("receipt: {}", e))?;

        let new_count = self.get_pool_count().await?;
        Ok(new_count.saturating_sub(1))
    }

    pub async fn join_bet_pool(&self, pool_id: u32, side: u8, amount_wei: U256) -> Result<String> {
        let provider = self.write_provider().await?;
        let contract = IBetChu::new(self.contract_address, provider);

        let tx = contract
            .placeBet(U256::from(pool_id), side)
            .value(amount_wei)
            .send()
            .await
            .map_err(|e| anyhow!("placeBet: {}", e))?;

        let receipt = tx.get_receipt().await.map_err(|e| anyhow!("receipt: {}", e))?;
        Ok(format!("{:?}", receipt.transaction_hash))
    }

    /// Called by the oracle once ESPN confirms the game is final.
    /// result: 1 = Side A (away team) won, 2 = Side B (home team) won.
    pub async fn report_score(&self, pool_id: u32, result: u8) -> Result<String> {
        let provider = self.write_provider().await?;
        let contract = IBetChu::new(self.contract_address, provider);

        let tx = contract
            .reportScore(U256::from(pool_id), result)
            .send()
            .await
            .map_err(|e| anyhow!("reportScore({}): {}", pool_id, e))?;

        let receipt = tx.get_receipt().await.map_err(|e| anyhow!("receipt: {}", e))?;
        Ok(format!("{:?}", receipt.transaction_hash))
    }

    pub async fn get_user_bets(&self, pool_id: u32, address: &str) -> Result<(f64, f64)> {
        let provider = self.read_provider().await?;
        let contract = IBetChu::new(self.contract_address, provider);
        let addr = Address::from_str(address).map_err(|e| anyhow!("Invalid address: {}", e))?;

        let result = contract
            .getPoolBetsByAddress(U256::from(pool_id), addr)
            .call()
            .await
            .map_err(|e| anyhow!("getPoolBetsByAddress: {}", e))?;

        Ok((wei_to_eth(result.sideAAmount), wei_to_eth(result.sideBAmount)))
    }
}

pub fn wei_to_eth(wei: U256) -> f64 {
    let divisor = U256::from(10u64.pow(15));
    let milli = wei / divisor;
    let n: u64 = milli.try_into().unwrap_or(0u64);
    n as f64 / 1000.0
}

pub fn eth_to_wei(eth: f64) -> U256 {
    let wei = (eth * 1e18) as u128;
    U256::from(wei)
}
