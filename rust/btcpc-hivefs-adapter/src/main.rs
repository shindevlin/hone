//! btcpc-hivefs-adapter — Hive external replica writer and verifier.
//!
//! # Usage
//!
//! ## Phase 2 — Write
//!
//! ```text
//! btcpc-hivefs-adapter write \
//!   --cid bafybtcpc... \
//!   --file /path/to/chunk.bin \
//!   --kind chunk
//! ```
//!
//! Required env: HIVE_ACCOUNT, HIVE_POSTING_KEY, BTCPC_NODE_ID, BTCPC_POSTING_KEY
//! Optional env: BTCPC_API_URL (default http://localhost:4242)
//!               HIVE_API_URL  (default https://api.hive.blog)
//!
//! ## Phase 3 — Verify
//!
//! ```text
//! btcpc-hivefs-adapter verify \
//!   --node-id <storage-node-btcpc-account> \
//!   --cid bafybtcpc... \
//!   --hive-tx-id <40-char-hex> \
//!   --hive-block-num 12345678 \
//!   --op-index 0 \
//!   --epoch 42
//! ```
//!
//! Required env: HIVE_ACCOUNT, BTCPC_NODE_ID, BTCPC_VERIFIER_ID, BTCPC_VERIFIER_KEY
//! Optional env: BTCPC_API_URL, HIVE_API_URL

mod btcpc_client;
mod hive_client;
mod verifier;
mod writer;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "btcpc-hivefs-adapter",
    version,
    about = "BTCPC-FS Hive external replica writer and verifier"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Phase 2: write a BTCPC-FS blob to Hive and submit HiveReplicaCommit.
    Write {
        /// BTCPC-FS CID of the blob being replicated.
        #[arg(long)]
        cid: String,

        /// Path to the local file to replicate (chunk, manifest, or full blob).
        #[arg(long)]
        file: String,

        /// Replica kind: full | chunk | parity | manifest
        #[arg(long, value_parser = ["full", "chunk", "parity", "manifest"])]
        kind: String,
    },

    /// Phase 3: verify a Hive replica and submit HiveReplicaVerify.
    Verify {
        /// BTCPC account ID of the storage node whose replica is being verified.
        #[arg(long)]
        node_id: String,

        /// BTCPC-FS CID of the replicated blob.
        #[arg(long)]
        cid: String,

        /// Hive transaction ID (40-char hex) of the custom_json broadcast.
        #[arg(long)]
        hive_tx_id: String,

        /// Hive block number the transaction was included in.
        #[arg(long)]
        hive_block_num: u64,

        /// Index of the operation within the Hive transaction (usually 0).
        #[arg(long, default_value = "0")]
        op_index: u32,

        /// Current BTCPC epoch to compute the challenge hash for.
        #[arg(long)]
        epoch: u64,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Write { cid, file, kind } => {
            let cfg = writer::WriterConfig::from_env()?;
            writer::run_write(cfg, cid, file, kind).await?;
        }
        Command::Verify {
            node_id,
            cid,
            hive_tx_id,
            hive_block_num,
            op_index,
            epoch,
        } => {
            let cfg = verifier::VerifierConfig::from_env()?;
            verifier::run_verify(cfg, node_id, cid, hive_tx_id, hive_block_num, op_index, epoch)
                .await?;
        }
    }

    Ok(())
}
