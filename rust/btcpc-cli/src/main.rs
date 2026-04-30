mod api;
mod chain;
mod contract;
mod inference;
mod key;
mod tx;
mod wallet;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process;

#[derive(Parser)]
#[command(
    name = "btcpc",
    version = "1.0.0",
    author = "Shin Devlin <shindevlin@proton.me>",
    about = "BTCPC chain CLI — interact with a btcpc-node",
    long_about = None
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Show account balance
    Balance {
        /// Account name
        account: String,
        /// Token (optional, defaults to BTCPC)
        #[arg(long)]
        token: Option<String>,
    },

    /// Show staked amount for an account
    Stake {
        /// Account name
        account: String,
    },

    /// Show block info (latest if epoch omitted)
    Block {
        /// Epoch number (omit for latest)
        epoch: Option<u64>,
    },

    /// Show current epoch
    Epoch,

    /// Show account details
    Account {
        /// Account name
        name: String,
    },

    /// Node health check
    Health,

    /// Transfer BTCPC between accounts
    Transfer {
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: String,
        /// Amount in dreams (u64)
        #[arg(long)]
        amount: u64,
        #[arg(long)]
        memo: Option<String>,
        #[arg(long)]
        key_file: Option<PathBuf>,
    },

    /// Add stake
    #[command(name = "stake-add")]
    StakeAdd {
        #[arg(long)]
        account: String,
        /// Amount in dreams (u64)
        #[arg(long)]
        amount: u64,
        #[arg(long)]
        key_file: Option<PathBuf>,
    },

    /// Remove stake
    #[command(name = "stake-remove")]
    StakeRemove {
        #[arg(long)]
        account: String,
        /// Amount in dreams (u64)
        #[arg(long)]
        amount: u64,
        #[arg(long)]
        key_file: Option<PathBuf>,
    },

    /// Create a new account
    #[command(name = "account-create")]
    AccountCreate {
        #[arg(long)]
        account: String,
        /// Hex-encoded public key
        #[arg(long)]
        pubkey: Option<String>,
    },

    /// Contract subcommands
    Contract {
        #[command(subcommand)]
        action: ContractCommands,
    },

    /// Key management
    Key {
        #[command(subcommand)]
        action: KeyCommands,
    },

    /// Wallet management (mnemonic-backed, multi-chain)
    Wallet {
        #[command(subcommand)]
        action: WalletCommands,
    },

    /// Inference marketplace
    Inference {
        #[command(subcommand)]
        action: InferenceCommands,
    },
}

#[derive(Subcommand)]
enum ContractCommands {
    /// Deploy a contract from a .wasm file
    Deploy {
        #[arg(long)]
        deployer: String,
        /// Path to .wasm file
        #[arg(long)]
        wasm: String,
        #[arg(long)]
        init_method: Option<String>,
        /// JSON-encoded init arguments
        #[arg(long)]
        init_args: Option<String>,
    },

    /// Call a contract method (state-changing)
    Call {
        #[arg(long)]
        contract: String,
        #[arg(long)]
        method: String,
        #[arg(long)]
        signer: String,
        /// JSON-encoded arguments
        #[arg(long)]
        args: Option<String>,
        /// Deposit in dreams (u64)
        #[arg(long)]
        deposit: Option<u64>,
    },

    /// View a contract method (read-only)
    View {
        #[arg(long)]
        contract: String,
        #[arg(long)]
        method: String,
        /// JSON-encoded arguments
        #[arg(long)]
        args: Option<String>,
    },
}

#[derive(Subcommand)]
enum KeyCommands {
    /// Generate a new keypair and save to a file.
    Generate {
        /// Output path (default: ~/.btcpc/key.json)
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Show the public key for a key file.
    Show {
        /// Key file path (default: ~/.btcpc/key.json)
        #[arg(long)]
        key_file: Option<PathBuf>,
    },
    /// Register this key's public key on-chain for an account.
    Register {
        #[arg(long)]
        account: String,
        /// Key file path (default: ~/.btcpc/key.json)
        #[arg(long)]
        key_file: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum WalletCommands {
    /// Generate a new BIP39 mnemonic, create the BTCPC account, and publish all
    /// derived chain addresses on-chain atomically.  Prints the mnemonic — back it up.
    Create {
        /// BTCPC account name to register.
        #[arg(long)]
        account: String,
        /// Where to save the wallet file (default: ~/.btcpc/wallet.json).
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Restore a wallet from an existing mnemonic and show its derived addresses.
    Show {
        /// Wallet file path (default: ~/.btcpc/wallet.json).
        #[arg(long)]
        wallet_file: Option<PathBuf>,
    },
    /// Re-publish derived chain addresses for a wallet already on-chain.
    /// Requires the mnemonic to derive the signing key (never stored on disk).
    Publish {
        /// Wallet file path (default: ~/.btcpc/wallet.json).
        #[arg(long)]
        wallet_file: Option<PathBuf>,
        /// BIP39 mnemonic phrase (or set BTCPC_MNEMONIC env var).
        #[arg(long)]
        mnemonic: String,
    },
}

#[derive(Subcommand)]
enum InferenceCommands {
    /// Post a new inference job to the marketplace
    Post {
        #[arg(long)]
        account: String,
        #[arg(long)]
        model: String,
        /// Input text or path to input file (prefix with @)
        #[arg(long)]
        input: String,
        /// Maximum fee in dreams (or BTCPC as decimal string e.g. "0.5")
        #[arg(long)]
        max_fee: u64,
        /// Minimum node reputation score required (0 = any)
        #[arg(long, default_value = "0")]
        min_rep: u64,
        /// Inference mode: solo, ensemble, or pipeline
        #[arg(long, default_value = "solo")]
        mode: String,
        /// Bid window in epochs (default 2)
        #[arg(long, default_value = "2")]
        bid_window: u64,
        /// Deadline epoch (job cancelled if not done by then)
        #[arg(long)]
        deadline: u64,
        #[arg(long)]
        key_file: Option<std::path::PathBuf>,
    },
    /// List inference jobs
    Jobs {
        /// Filter by status: posted, awarded, completed, verified, disputed, paid
        #[arg(long)]
        status: Option<String>,
        /// Filter by model name
        #[arg(long)]
        model: Option<String>,
    },
    /// Show full details for a single job
    Job {
        /// Job ID
        id: String,
    },
    /// Bid on an open job (node operators)
    Bid {
        #[arg(long)]
        job_id: String,
        #[arg(long)]
        account: String,
        /// Fee to accept in dreams
        #[arg(long)]
        fee: u64,
        /// Role: worker or verifier
        #[arg(long, default_value = "worker")]
        role: String,
        #[arg(long)]
        key_file: Option<std::path::PathBuf>,
    },
    /// Submit completed job result (node operators)
    Complete {
        #[arg(long)]
        job_id: String,
        #[arg(long)]
        account: String,
        #[arg(long)]
        result_hash: String,
        #[arg(long)]
        latency_ms: u64,
        #[arg(long)]
        key_file: Option<std::path::PathBuf>,
    },
    /// Cancel a job you posted
    Cancel {
        #[arg(long)]
        job_id: String,
        #[arg(long)]
        account: String,
        #[arg(long)]
        key_file: Option<std::path::PathBuf>,
    },
    /// Show reputation for a node
    Reputation {
        /// Node account name (defaults to local BTCPC_ACCOUNT)
        node: Option<String>,
    },
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Balance { account, token } => {
            chain::cmd_balance(&account, token.as_deref())?;
        }
        Commands::Stake { account } => {
            chain::cmd_stake(&account)?;
        }
        Commands::Block { epoch } => {
            chain::cmd_block(epoch)?;
        }
        Commands::Epoch => {
            chain::cmd_epoch()?;
        }
        Commands::Account { name } => {
            chain::cmd_account(&name)?;
        }
        Commands::Health => {
            chain::cmd_health()?;
        }
        Commands::Transfer {
            from,
            to,
            amount,
            memo,
            key_file,
        } => {
            tx::cmd_transfer(&from, &to, amount, memo.as_deref(), key_file.as_deref())?;
        }
        Commands::StakeAdd {
            account,
            amount,
            key_file,
        } => {
            tx::cmd_stake_add(&account, amount, key_file.as_deref())?;
        }
        Commands::StakeRemove {
            account,
            amount,
            key_file,
        } => {
            tx::cmd_stake_remove(&account, amount, key_file.as_deref())?;
        }
        Commands::AccountCreate { account, pubkey } => {
            tx::cmd_account_create(&account, pubkey.as_deref())?;
        }
        Commands::Contract { action } => match action {
            ContractCommands::Deploy {
                deployer,
                wasm,
                init_method,
                init_args,
            } => {
                contract::cmd_contract_deploy(
                    &deployer,
                    &wasm,
                    init_method.as_deref(),
                    init_args.as_deref(),
                )?;
            }
            ContractCommands::Call {
                contract,
                method,
                signer,
                args,
                deposit,
            } => {
                contract::cmd_contract_call(
                    &contract,
                    &method,
                    &signer,
                    args.as_deref(),
                    deposit,
                )?;
            }
            ContractCommands::View {
                contract,
                method,
                args,
            } => {
                contract::cmd_contract_view(&contract, &method, args.as_deref())?;
            }
        },
        Commands::Inference { action } => match action {
            InferenceCommands::Post { account, model, input, max_fee, min_rep, mode, bid_window, deadline, key_file } => {
                inference::cmd_post(&account, &model, &input, max_fee, min_rep, &mode, bid_window, deadline, key_file.as_deref())?;
            }
            InferenceCommands::Jobs { status, model } => {
                inference::cmd_jobs(status.as_deref(), model.as_deref())?;
            }
            InferenceCommands::Job { id } => {
                inference::cmd_job(&id)?;
            }
            InferenceCommands::Bid { job_id, account, fee, role, key_file } => {
                inference::cmd_bid(&job_id, &account, fee, &role, key_file.as_deref())?;
            }
            InferenceCommands::Complete { job_id, account, result_hash, latency_ms, key_file } => {
                inference::cmd_complete(&job_id, &account, &result_hash, latency_ms, key_file.as_deref())?;
            }
            InferenceCommands::Cancel { job_id, account, key_file } => {
                inference::cmd_cancel(&job_id, &account, key_file.as_deref())?;
            }
            InferenceCommands::Reputation { node } => {
                inference::cmd_reputation(node.as_deref())?;
            }
        },
        Commands::Key { action } => match action {
            KeyCommands::Generate { output } => {
                key::cmd_key_generate(output.as_deref())?;
            }
            KeyCommands::Show { key_file } => {
                key::cmd_key_show(key_file.as_deref())?;
            }
            KeyCommands::Register { account, key_file } => {
                key::cmd_key_register(&account, key_file.as_deref())?;
            }
        },
        Commands::Wallet { action } => match action {
            WalletCommands::Create { account, output } => {
                wallet::cmd_wallet_create(&account, output.as_deref())?;
            }
            WalletCommands::Show { wallet_file } => {
                wallet::cmd_wallet_show(wallet_file.as_deref())?;
            }
            WalletCommands::Publish { wallet_file, mnemonic } => {
                wallet::cmd_wallet_publish(wallet_file.as_deref(), &mnemonic)?;
            }
        },
    }

    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("{}: {:#}", colored::Colorize::red("error"), e);
        process::exit(1);
    }
}
