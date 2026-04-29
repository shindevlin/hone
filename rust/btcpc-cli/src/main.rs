mod api;
mod chain;
mod contract;
mod tx;

use anyhow::Result;
use clap::{Parser, Subcommand};
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
        /// Amount in BTCPC (e.g. 1.5)
        #[arg(long)]
        amount: f64,
        #[arg(long)]
        memo: Option<String>,
        /// Hex-encoded signature
        #[arg(long)]
        sig: Option<String>,
    },

    /// Add stake
    #[command(name = "stake-add")]
    StakeAdd {
        #[arg(long)]
        account: String,
        /// Amount in BTCPC
        #[arg(long)]
        amount: f64,
        #[arg(long)]
        sig: Option<String>,
    },

    /// Remove stake
    #[command(name = "stake-remove")]
    StakeRemove {
        #[arg(long)]
        account: String,
        /// Amount in BTCPC
        #[arg(long)]
        amount: f64,
        #[arg(long)]
        sig: Option<String>,
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
        /// Deposit in BTCPC
        #[arg(long)]
        deposit: Option<f64>,
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
            sig,
        } => {
            tx::cmd_transfer(&from, &to, amount, memo.as_deref(), sig.as_deref())?;
        }
        Commands::StakeAdd {
            account,
            amount,
            sig,
        } => {
            tx::cmd_stake_add(&account, amount, sig.as_deref())?;
        }
        Commands::StakeRemove {
            account,
            amount,
            sig,
        } => {
            tx::cmd_stake_remove(&account, amount, sig.as_deref())?;
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
    }

    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("{}: {:#}", colored::Colorize::red("error"), e);
        process::exit(1);
    }
}
