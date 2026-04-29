//! Testnet simulation daemon — generates fake activity on btcpc-satoshi.
//! Only spawned when BTCPC_CHAIN_ID=btcpc-satoshi.

use std::sync::Arc;
use tracing::{info, warn};
use btcpc_types::{LedgerEntry, NATIVE_TOKEN};

use crate::chain::Chain;

const SIM_ACCOUNTS: &[&str] = &[
    "satoshi-sim-1",
    "satoshi-sim-2",
    "satoshi-sim-3",
    "satoshi-sim-4",
    "satoshi-sim-5",
];

/// 10,000 BTCPC per sim account — enough for years of activity.
const SIM_FUND_DREAMS: u64 = 10_000 * 10_000_000_000;

pub async fn run(chain: Arc<Chain>) {
    seed_accounts(&chain);
    info!("testnet sim daemon started ({} accounts seeded)", SIM_ACCOUNTS.len());

    let mut tick: u64 = 0;
    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
        tick += 1;
        run_tick(&chain, tick, chain.current_epoch());
    }
}

fn seed_accounts(chain: &Chain) {
    for account in SIM_ACCOUNTS {
        if chain.store.get_account(account).ok().flatten().is_some() {
            continue;
        }
        let _ = chain.apply_entry(&LedgerEntry::AccountCreate {
            account: account.to_string(),
            public_key: None,
            epoch: 0,
        });
        let _ = chain.apply_entry(&LedgerEntry::GenesisAlloc {
            account: account.to_string(),
            amount: SIM_FUND_DREAMS,
            token: NATIVE_TOKEN.to_string(),
        });
        info!("sim: seeded account {} with {} dreams", account, SIM_FUND_DREAMS);
    }
}

fn run_tick(chain: &Chain, tick: u64, epoch: u64) {
    use sha2::Digest;

    let n = SIM_ACCOUNTS.len() as u64;
    let from = SIM_ACCOUNTS[(tick % n) as usize];
    let to = SIM_ACCOUNTS[((tick + 1) % n) as usize];

    // Transfer 0.1 BTCPC between rotating sim account pair.
    let transfer_amount: u64 = 1_000_000_000; // 0.1 BTCPC
    if chain.get_balance(from, NATIVE_TOKEN) >= transfer_amount * 10 {
        let entry = LedgerEntry::Transfer {
            from: from.to_string(),
            to: to.to_string(),
            amount: transfer_amount,
            token: NATIVE_TOKEN.to_string(),
            memo: Some(format!("sim-tick-{}", tick)),
            epoch,
            signed_by: from.to_string(),
            nonce: 0,
        };
        if let Err(e) = chain.apply_entry(&entry) {
            warn!("sim: transfer failed: {}", e);
        }
    }

    // Sensor reading — fake GNSS data.
    let data_hash = {
        let mut h = sha2::Sha256::new();
        h.update(tick.to_le_bytes());
        h.update(b"sensor");
        hex::encode(h.finalize())
    };
    let reading = LedgerEntry::SensorReading {
        sensor_id: format!("sim-gnss-{}", (tick % 3) + 1),
        owner: from.to_string(),
        epoch,
        value: (tick % 1000) as f64 * 0.001,
        data_hash: data_hash.clone(),
        metadata: Some(serde_json::json!({
            "lat": -33.8688 + (tick as f64 * 0.0001),
            "lon": 151.2093 + (tick as f64 * 0.0001),
            "sim": true,
        })),
    };
    if let Err(e) = chain.apply_entry(&reading) {
        warn!("sim: sensor reading failed: {}", e);
    }

    // Blob store every 3 ticks.
    if tick % 3 == 0 {
        let cid = {
            let mut h = sha2::Sha256::new();
            h.update(tick.to_le_bytes());
            h.update(b"blob");
            format!("Qm{}", &hex::encode(h.finalize())[..32])
        };
        let blob = LedgerEntry::BlobStore {
            cid,
            uploader: SIM_ACCOUNTS[(tick % n) as usize].to_string(),
            size_bytes: 1024 + (tick % 4096),
            epoch,
            fee: 0,
        };
        if let Err(e) = chain.apply_entry(&blob) {
            warn!("sim: blob store failed: {}", e);
        }
    }

    // Inference job post every 5 ticks.
    if tick % 5 == 0 {
        let requester = SIM_ACCOUNTS[(tick % n) as usize];
        let job_id = {
            let mut h = sha2::Sha256::new();
            h.update(tick.to_le_bytes());
            h.update(b"infer");
            hex::encode(h.finalize())[..16].to_owned()
        };
        let models = ["llama3-8b", "mistral-7b", "phi-3"];
        let job = LedgerEntry::InferenceJobPost {
            job_id,
            requester: requester.to_string(),
            model: models[((tick / 5) % 3) as usize].to_string(),
            mode: "solo".to_string(),
            input_hash: data_hash,
            max_fee: 1_000_000_000, // 0.1 BTCPC
            min_reputation: 0,
            bid_window_epochs: 2,
            deadline_epoch: epoch + 10,
            epoch,
            nonce: 0,
            signed_by: requester.to_string(),
        };
        if let Err(e) = chain.apply_entry(&job) {
            warn!("sim: inference post failed: {}", e);
        }
    }

    info!("sim tick {} (epoch {}): {}->{}", tick, epoch, from, to);
}
