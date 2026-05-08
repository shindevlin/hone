mod api;
mod chain;
mod contract;
mod inference;
mod key;
mod repo;
mod session;
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

    /// LinkGit repository management
    Repo {
        #[command(subcommand)]
        action: RepoCommands,
    },

    /// Sign in with a key file — saves session to ~/.btcpc/session.json
    Login {
        /// Account name to associate with this session
        #[arg(long)]
        account: String,
        /// Path to key file (default: ~/.btcpc/key.json)
        #[arg(long)]
        key_file: Option<PathBuf>,
        /// Node URL (default: http://localhost:4242)
        #[arg(long)]
        node_url: Option<String>,
    },

    /// Clear the saved session
    Logout,

    /// Show the currently active session
    Whoami,
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
    /// Write .btcpc/wallet.env in the current directory from your saved wallet.
    /// Run this inside a project repo to wire up BTCPC_ACCOUNT and BTCPC_API_KEY.
    Env {
        /// Wallet file to read account name from (default: ~/.btcpc/wallet.json).
        #[arg(long)]
        wallet_file: Option<PathBuf>,
        /// Override output path (default: .btcpc/wallet.env in CWD).
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Generate a random API key, register it on-chain, and write it to .btcpc/wallet.env.
    /// Requires your mnemonic to sign the on-chain AccountApiKeySet entry.
    #[command(name = "api-key-gen")]
    ApiKeyGen {
        /// BIP39 mnemonic phrase (or set BTCPC_MNEMONIC env var).
        #[arg(long, env = "BTCPC_MNEMONIC")]
        mnemonic: String,
        /// Wallet file (default: ~/.btcpc/wallet.json).
        #[arg(long)]
        wallet_file: Option<PathBuf>,
        /// Output path (default: .btcpc/wallet.env in CWD).
        #[arg(long)]
        output: Option<PathBuf>,
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

#[derive(Subcommand)]
enum RepoCommands {
    /// Register a new repository on-chain and print the git remote URL
    Create {
        /// Repository name
        name: String,
        /// Account (defaults to active session account)
        #[arg(long)]
        account: Option<String>,
        /// Make the repository private
        #[arg(long)]
        private: bool,
        /// Key file (defaults to session key)
        #[arg(long)]
        key_file: Option<PathBuf>,
    },
    /// Register a repo on-chain AND wire up git remote in the current directory
    Init {
        /// Repository name
        name: String,
        /// Account (defaults to active session account)
        #[arg(long)]
        account: Option<String>,
        /// Make the repository private
        #[arg(long)]
        private: bool,
        /// Key file (defaults to session key)
        #[arg(long)]
        key_file: Option<PathBuf>,
    },
    /// List repositories for an account
    List {
        /// Account to list repos for (defaults to active session account)
        owner: Option<String>,
    },
    /// Show details and refs for a repository
    Info {
        /// owner/repo  or  repo (uses session account as owner)
        repo: String,
    },
    /// Clone a repository
    Clone {
        /// owner/repo
        repo: String,
        /// Target directory (defaults to repo name)
        dir: Option<String>,
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
            WalletCommands::Env { wallet_file, output } => {
                wallet::cmd_wallet_env(wallet_file.as_deref(), output.as_deref())?;
            }
            WalletCommands::ApiKeyGen { mnemonic, wallet_file, output } => {
                wallet::cmd_wallet_api_key_gen(wallet_file.as_deref(), &mnemonic, output.as_deref())?;
            }
        },

        Commands::Repo { action } => match action {
            RepoCommands::Create { name, account, private, key_file } => {
                repo::cmd_repo_create(&name, account.as_deref(), private, key_file.as_deref())?;
            }
            RepoCommands::Init { name, account, private, key_file } => {
                repo::cmd_repo_init(&name, account.as_deref(), private, key_file.as_deref())?;
            }
            RepoCommands::List { owner } => {
                repo::cmd_repo_list(owner.as_deref())?;
            }
            RepoCommands::Info { repo } => {
                let (owner, name) = parse_repo_slug(&repo)?;
                repo::cmd_repo_info(&owner, &name)?;
            }
            RepoCommands::Clone { repo, dir } => {
                let (owner, name) = parse_repo_slug(&repo)?;
                repo::cmd_repo_clone(&owner, &name, dir.as_deref())?;
            }
        },

        Commands::Login { account, key_file, node_url } => {
            cmd_login(&account, key_file.as_deref(), node_url.as_deref())?;
        }
        Commands::Logout => {
            cmd_logout()?;
        }
        Commands::Whoami => {
            cmd_whoami()?;
        }
    }

    Ok(())
}

/// Parse "owner/repo" slug; if no slash, uses the active session account as owner.
fn parse_repo_slug(slug: &str) -> Result<(String, String)> {
    if let Some((owner, name)) = slug.split_once('/') {
        return Ok((owner.to_owned(), name.to_owned()));
    }
    let owner = session::load()
        .map(|s| s.account)
        .ok_or_else(|| anyhow::anyhow!("no owner in slug and no active session — use owner/repo or run `btcpc login`"))?;
    Ok((owner, slug.to_owned()))
}

fn cmd_login(account: &str, key_file: Option<&std::path::Path>, node_url: Option<&str>) -> Result<()> {
    use colored::Colorize;
    let key_path = session::resolve_key_file(key_file, None)?;
    // verify the key file is readable
    btcpc_sdk::KeyPair::from_file(&key_path)
        .map_err(|e| anyhow::anyhow!("cannot read key file {}: {}", key_path.display(), e))?;

    let sess = session::Session {
        account: account.to_owned(),
        key_file: key_path.clone(),
        node_url: node_url.unwrap_or("http://localhost:4242").to_owned(),
    };
    session::save(&sess)?;
    println!("{}", "Logged in.".green().bold());
    println!("{} {}", "Account:".bold(), sess.account);
    println!("{} {}", "Key file:".bold(), sess.key_file.display());
    println!("{} {}", "Node:".bold(), sess.node_url);
    Ok(())
}

fn cmd_logout() -> Result<()> {
    use colored::Colorize;
    session::clear()?;
    println!("{}", "Session cleared.".green().bold());
    Ok(())
}

fn cmd_whoami() -> Result<()> {
    use colored::Colorize;
    match session::load() {
        Some(s) => {
            println!("{} {}", "Account:".bold(), s.account);
            println!("{} {}", "Key file:".bold(), s.key_file.display());
            println!("{} {}", "Node:".bold(), s.node_url);
        }
        None => {
            println!("{}", "Not logged in.".yellow());
            println!("Run: btcpc login --account <name> [--key-file <path>] [--node-url <url>]");
        }
    }
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("{}: {:#}", colored::Colorize::red("error"), e);
        process::exit(1);
    }
}
