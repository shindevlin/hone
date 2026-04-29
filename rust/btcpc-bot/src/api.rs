use anyhow::{anyhow, Result};
use reqwest::Client;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct BalanceResponse {
    pub balance: Option<f64>,
    pub dreams: Option<u64>,
    pub token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct StakeResponse {
    pub stake: Option<f64>,
    pub dreams: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct LatestResponse {
    pub epoch: Option<u64>,
    pub hash: Option<String>,
    pub current_epoch: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct BlockResponse {
    pub epoch: Option<u64>,
    pub hash: Option<String>,
    pub header: Option<serde_json::Value>,
    pub payload: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct HealthResponse {
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct FaucetResponse {
    pub success: Option<bool>,
    pub message: Option<String>,
    pub amount: Option<f64>,
    pub dreams: Option<u64>,
}

pub struct BtcpcClient {
    client: Client,
    base_url: String,
}

impl BtcpcClient {
    pub fn new(base_url: String) -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("Failed to build HTTP client"),
            base_url,
        }
    }

    pub async fn get_balance(&self, account: &str) -> Result<BalanceResponse> {
        let url = format!("{}/api/balance/{}", self.base_url, account);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| anyhow!("Node offline: {}", e))?;
        if !resp.status().is_success() {
            return Err(anyhow!("API error: {}", resp.status()));
        }
        resp.json::<BalanceResponse>()
            .await
            .map_err(|e| anyhow!("Parse error: {}", e))
    }

    pub async fn get_stake(&self, account: &str) -> Result<StakeResponse> {
        let url = format!("{}/api/stake/{}", self.base_url, account);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| anyhow!("Node offline: {}", e))?;
        if !resp.status().is_success() {
            return Err(anyhow!("API error: {}", resp.status()));
        }
        resp.json::<StakeResponse>()
            .await
            .map_err(|e| anyhow!("Parse error: {}", e))
    }

    pub async fn get_latest(&self) -> Result<LatestResponse> {
        let url = format!("{}/api/latest", self.base_url);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| anyhow!("Node offline: {}", e))?;
        if !resp.status().is_success() {
            return Err(anyhow!("API error: {}", resp.status()));
        }
        resp.json::<LatestResponse>()
            .await
            .map_err(|e| anyhow!("Parse error: {}", e))
    }

    pub async fn get_block(&self, epoch: u64) -> Result<BlockResponse> {
        let url = format!("{}/api/block/{}", self.base_url, epoch);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| anyhow!("Node offline: {}", e))?;
        if !resp.status().is_success() {
            return Err(anyhow!("API error: {}", resp.status()));
        }
        resp.json::<BlockResponse>()
            .await
            .map_err(|e| anyhow!("Parse error: {}", e))
    }

    pub async fn get_health(&self) -> Result<HealthResponse> {
        let url = format!("{}/health", self.base_url);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| anyhow!("Node offline: {}", e))?;
        if !resp.status().is_success() {
            return Err(anyhow!("API error: {}", resp.status()));
        }
        resp.json::<HealthResponse>()
            .await
            .map_err(|e| anyhow!("Parse error: {}", e))
    }

    pub async fn claim_faucet(&self, account: &str) -> Result<FaucetResponse> {
        let url = format!("{}/api/faucet/claim", self.base_url);
        let body = serde_json::json!({ "account": account });
        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow!("Node offline: {}", e))?;
        if !resp.status().is_success() {
            return Err(anyhow!("API error: {}", resp.status()));
        }
        resp.json::<FaucetResponse>()
            .await
            .map_err(|e| anyhow!("Parse error: {}", e))
    }
}

/// Convert dreams to BTCPC (1 BTCPC = 10^10 dreams)
pub fn dreams_to_btcpc(dreams: u64) -> f64 {
    dreams as f64 / 10_000_000_000.0
}

