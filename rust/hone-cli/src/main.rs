mod agent;
mod api;
mod auction;
mod bridge;
mod chain;
mod contract;
mod ensemble;
mod finetune;
mod helpers;
mod inference;
mod key;
mod memory;
mod oracle;
mod private_auth;
mod repo;
mod science;
mod session;
mod sessions;
mod slash;
mod totp;
mod tx;
mod vrf;
mod wallet;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process;

#[derive(Parser)]
#[command(
    name = "hone",
    version = "1.0.0",
    author = "Shin Devlin <shindevlin@proton.me>",
    about = "HONE chain CLI — interact with a hone-node",
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
        /// Token (optional, defaults to HONE)
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

    /// Transfer HONE between accounts
    Transfer {
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: String,
        /// Amount in hunits (u64)
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
        /// Amount in hunits (u64)
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
        /// Amount in hunits (u64)
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
        /// Key file used to sign the account claim (default: session key or ~/.hone/key.json)
        #[arg(long)]
        key_file: Option<PathBuf>,
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

    /// Scientific compute marketplace
    Science {
        #[command(subcommand)]
        action: ScienceCommands,
    },

    /// On-chain key-value memory store for agents
    Memory {
        #[command(subcommand)]
        action: MemoryCommands,
    },

    /// RAG document index (embed + cosine search via Ollama)
    Rag {
        #[command(subcommand)]
        action: RagCommands,
    },

    /// Name auctions (bid on a short account name)
    Auction {
        #[command(subcommand)]
        action: AuctionCommands,
    },

    /// Freeport item auctions
    Freeport {
        #[command(subcommand)]
        action: FreeportCommands,
    },

    /// LoRA fine-tune job marketplace
    Finetune {
        #[command(subcommand)]
        action: FinetuneCommands,
    },

    /// Computer-use automation job marketplace
    #[command(name = "computer-use")]
    ComputerUse {
        #[command(subcommand)]
        action: ComputerUseCommands,
    },

    /// Snapshot save/load (HONE-FS backed)
    Snap {
        #[command(subcommand)]
        action: SnapCommands,
    },

    /// Amber Pill soulbound NFT (1.5× mining weight)
    #[command(name = "amber-pill")]
    AmberPill {
        #[command(subcommand)]
        action: AmberPillCommands,
    },

    /// Fee and mempool status
    Fee {
        #[command(subcommand)]
        action: FeeCommands,
    },

    /// P2P storefront registry
    #[command(name = "peer-commerce")]
    PeerCommerce {
        #[command(subcommand)]
        action: PeerCommerceCommands,
    },

    /// Gateway shortcode resolver
    #[command(name = "gateway")]
    Gateway {
        /// Shortcode to resolve
        shortcode: String,
    },

    /// Agent registry and task marketplace
    #[command(name = "agent")]
    Agent {
        #[command(subcommand)]
        action: AgentCommands,
    },

    /// LinkGit repository management
    Repo {
        #[command(subcommand)]
        action: RepoCommands,
    },

    /// Sign in with a key file — saves session to ~/.hone/session.json
    Login {
        /// Account name to associate with this session
        #[arg(long)]
        account: String,
        /// Path to key file (default: ~/.hone/key.json)
        #[arg(long)]
        key_file: Option<PathBuf>,
        /// Node URL (default: http://localhost:4242)
        #[arg(long)]
        node_url: Option<String>,
    },

    /// Ensemble inference coordinator (fan-out to N workers, majority vote)
    Ensemble {
        #[command(subcommand)]
        action: EnsembleCommands,
    },

    /// Slash misbehaving validators and file/vote on appeals
    Slash {
        #[command(subcommand)]
        action: SlashCommands,
    },

    /// wHONE cross-chain bridge (wrap, unwrap, fund, unlock)
    Bridge {
        #[command(subcommand)]
        action: BridgeCommands,
    },

    /// On-chain oracle price feeds (commit-reveal, median aggregation)
    Oracle {
        #[command(subcommand)]
        action: OracleCommands,
    },

    /// VRF beacon — deterministic randomness from clock-node commit-reveal
    Vrf {
        #[command(subcommand)]
        action: VrfCommands,
    },

    /// Session marketplace — buy/sell AI context windows on-chain
    #[command(name = "session-market")]
    SessionMarket {
        #[command(subcommand)]
        action: SessionMarketCommands,
    },

    /// Agent sessions — multi-turn on-chain agent conversations
    #[command(name = "agent-session")]
    AgentSession {
        #[command(subcommand)]
        action: AgentSessionCommands,
    },

    /// TOTP 2FA for account operations
    Totp {
        #[command(subcommand)]
        action: TotpCommands,
    },

    /// Private M-of-N authorization for high-value transfers
    #[command(name = "private-auth")]
    PrivateAuth {
        #[command(subcommand)]
        action: PrivateAuthCommands,
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
        /// Deposit in hunits (u64)
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
        /// Output path (default: ~/.hone/key.json)
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Show the public key for a key file.
    Show {
        /// Key file path (default: ~/.hone/key.json)
        #[arg(long)]
        key_file: Option<PathBuf>,
    },
    /// Register this key's public key on-chain for an account.
    Register {
        #[arg(long)]
        account: String,
        /// Key role to register (default: posting). Options: posting, owner, memo, active.
        #[arg(long, default_value = "posting")]
        role: String,
        /// Key file path (default: ~/.hone/key.json)
        #[arg(long)]
        key_file: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum WalletCommands {
    /// Generate a new BIP39 mnemonic, create the HONE account, and publish all
    /// derived chain addresses on-chain atomically.  Prints the mnemonic — back it up.
    Create {
        /// HONE account name to register.
        #[arg(long)]
        account: String,
        /// Where to save the wallet file (default: ~/.hone/wallet.json).
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Restore a wallet from an existing mnemonic and show its derived addresses.
    Show {
        /// Wallet file path (default: ~/.hone/wallet.json).
        #[arg(long)]
        wallet_file: Option<PathBuf>,
    },
    /// Re-publish derived chain addresses for a wallet already on-chain.
    /// Requires the mnemonic to derive the signing key (never stored on disk).
    Publish {
        /// Wallet file path (default: ~/.hone/wallet.json).
        #[arg(long)]
        wallet_file: Option<PathBuf>,
        /// BIP39 mnemonic phrase (or set HONE_MNEMONIC env var).
        #[arg(long)]
        mnemonic: String,
    },
    /// Write .hone/wallet.env in the current directory from your saved wallet.
    /// Run this inside a project repo to wire up HONE_ACCOUNT and HONE_API_KEY.
    Env {
        /// Wallet file to read account name from (default: ~/.hone/wallet.json).
        #[arg(long)]
        wallet_file: Option<PathBuf>,
        /// Override output path (default: .hone/wallet.env in CWD).
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Generate a random API key, register it on-chain, and write it to .hone/wallet.env.
    /// Requires your mnemonic to sign the on-chain AccountApiKeySet entry.
    #[command(name = "api-key-gen")]
    ApiKeyGen {
        /// BIP39 mnemonic phrase (or set HONE_MNEMONIC env var).
        #[arg(long, env = "HONE_MNEMONIC")]
        mnemonic: String,
        /// Wallet file (default: ~/.hone/wallet.json).
        #[arg(long)]
        wallet_file: Option<PathBuf>,
        /// Output path (default: .hone/wallet.env in CWD).
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
        /// Maximum fee in hunits (or HONE as decimal string e.g. "0.5")
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
        /// Fee to accept in hunits
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
        /// Node account name (defaults to local HONE_ACCOUNT)
        node: Option<String>,
    },
}

#[derive(Subcommand)]
enum ScienceCommands {
    /// Submit a new scientific compute job
    Create {
        #[arg(long)]
        account: String,
        /// Short job title
        #[arg(long)]
        title: String,
        /// Job type: genomics, climate, protein-folding, drug-discovery, general, …
        #[arg(long, default_value = "general")]
        job_type: String,
        /// Input data or @path/to/file
        #[arg(long)]
        input: String,
        /// Maximum fee in hunits
        #[arg(long)]
        max_fee: u64,
        /// Mark as open-source (40% fee discount; results inscribed on-chain)
        #[arg(long, default_value = "false")]
        open_source: bool,
        /// Optional model hint
        #[arg(long)]
        model: Option<String>,
    },
    /// List science jobs (optionally filter by type)
    Jobs {
        /// Filter by job type
        #[arg(long)]
        job_type: Option<String>,
    },
    /// Show details for a single science job
    Job {
        /// Job ID
        id: String,
    },
    /// Mark a queued job as running
    Start {
        /// Job ID
        job_id: String,
        /// Optional shard-group ID
        #[arg(long)]
        shard_group: Option<String>,
    },
    /// Submit results for a running job
    Complete {
        /// Job ID
        job_id: String,
        /// SHA-256 result hash (hex)
        #[arg(long)]
        result_hash: String,
        /// Result bytes or @path/to/file
        #[arg(long)]
        result: String,
        /// Contributing node account names (comma-separated)
        #[arg(long, value_delimiter = ',')]
        nodes: Vec<String>,
        /// Epoch at which the result was produced
        #[arg(long)]
        epoch: u64,
    },
}

#[derive(Subcommand)]
enum MemoryCommands {
    /// Set a key-value pair in on-chain memory
    Set {
        #[arg(long)] account: String,
        #[arg(long)] key: String,
        #[arg(long)] value: String,
        #[arg(long)] key_file: Option<std::path::PathBuf>,
    },
    /// Get a value from on-chain memory
    Get {
        #[arg(long)] account: String,
        #[arg(long)] key: String,
    },
    /// Delete a key from on-chain memory
    Del {
        #[arg(long)] account: String,
        #[arg(long)] key: String,
        #[arg(long)] key_file: Option<std::path::PathBuf>,
    },
    /// Scan all keys for an account (optional prefix filter)
    Scan {
        #[arg(long)] account: String,
        #[arg(long, default_value = "")] prefix: String,
    },
}

#[derive(Subcommand)]
enum RagCommands {
    /// Index a document (embeds via Ollama nomic-embed-text)
    Index {
        #[arg(long)] account: String,
        #[arg(long)] doc_id: String,
        /// Document text content
        #[arg(long)] content: String,
        #[arg(long)] key_file: Option<std::path::PathBuf>,
    },
    /// Query the RAG index (returns top-K docs + 6k context)
    Query {
        #[arg(long)] account: String,
        /// Query text
        #[arg(long)] q: String,
    },
    /// Delete a document from the index
    Delete {
        #[arg(long)] account: String,
        #[arg(long)] doc_id: String,
        #[arg(long)] key_file: Option<std::path::PathBuf>,
    },
}

#[derive(Subcommand)]
enum AuctionCommands {
    /// Open a name auction
    Open {
        #[arg(long)] account: String,
        /// Account name to auction
        #[arg(long)] name: String,
        /// Auction duration in epochs
        #[arg(long, default_value = "20")] duration: u64,
        #[arg(long)] key_file: Option<std::path::PathBuf>,
    },
    /// Place a bid on an auction
    Bid {
        #[arg(long)] account: String,
        #[arg(long)] auction_id: String,
        /// Bid amount in hunits
        #[arg(long)] amount: u64,
        #[arg(long)] key_file: Option<std::path::PathBuf>,
    },
    /// Settle an ended auction
    Settle {
        #[arg(long)] account: String,
        #[arg(long)] auction_id: String,
        #[arg(long)] key_file: Option<std::path::PathBuf>,
    },
    /// Cancel an auction you opened
    Cancel {
        #[arg(long)] account: String,
        #[arg(long)] auction_id: String,
        #[arg(long)] key_file: Option<std::path::PathBuf>,
    },
    /// Show auction details
    Get { auction_id: String },
}

#[derive(Subcommand)]
enum FreeportCommands {
    /// Open a freeport item auction
    Open {
        #[arg(long)] account: String,
        #[arg(long)] item_id: String,
        #[arg(long, default_value = "digital")] item_type: String,
        #[arg(long, default_value = "20")] duration: u64,
        #[arg(long)] key_file: Option<std::path::PathBuf>,
    },
    /// Bid on a freeport auction
    Bid {
        #[arg(long)] account: String,
        #[arg(long)] auction_id: String,
        #[arg(long)] amount: u64,
        #[arg(long)] key_file: Option<std::path::PathBuf>,
    },
    /// Settle a freeport auction
    Settle {
        #[arg(long)] account: String,
        #[arg(long)] auction_id: String,
        #[arg(long)] key_file: Option<std::path::PathBuf>,
    },
    /// Show freeport auction details
    Get { auction_id: String },
}

#[derive(Subcommand)]
enum FinetuneCommands {
    /// Post a LoRA fine-tune job
    Post {
        #[arg(long)] account: String,
        #[arg(long)] base_model: String,
        #[arg(long)] dataset_cid: String,
        #[arg(long, default_value = "8")] lora_rank: u32,
        #[arg(long)] max_fee: u64,
        #[arg(long)] key_file: Option<std::path::PathBuf>,
    },
    /// Mark a fine-tune job complete (node operators)
    Complete {
        #[arg(long)] worker: String,
        #[arg(long)] job_id: String,
        #[arg(long)] adapter_cid: String,
        #[arg(long)] key_file: Option<std::path::PathBuf>,
    },
    /// Show a fine-tune job
    Get { job_id: String },
    /// List open fine-tune jobs
    Jobs,
}

#[derive(Subcommand)]
enum ComputerUseCommands {
    /// Post a computer-use automation job
    Post {
        #[arg(long)] account: String,
        /// JSON task spec (e.g. '{"url":"...","goal":"..."}')
        #[arg(long)] task: String,
        #[arg(long)] max_fee: u64,
        #[arg(long)] key_file: Option<std::path::PathBuf>,
    },
    /// Show a computer-use job
    Get { job_id: String },
    /// List open computer-use jobs
    Jobs,
}

#[derive(Subcommand)]
enum SnapCommands {
    /// Save a snapshot (CID pointer with a human slug)
    Save {
        #[arg(long)] account: String,
        #[arg(long)] slug: String,
        #[arg(long)] cid: String,
        #[arg(long)] key_file: Option<std::path::PathBuf>,
    },
    /// Fetch a snapshot by account + slug
    Get {
        #[arg(long)] account: String,
        #[arg(long)] slug: String,
    },
    /// List all snapshots for an account
    List { account: String },
}

#[derive(Subcommand)]
enum AmberPillCommands {
    /// Mint your Amber Pill (one per hardware fingerprint; grants 1.5× mining weight)
    Mint {
        #[arg(long)] account: String,
        /// Hardware fingerprint hex (from hone-node /api/node/hardware or hone-node logs)
        #[arg(long)] fingerprint: String,
        #[arg(long)] key_file: Option<std::path::PathBuf>,
    },
    /// Check if an account holds an Amber Pill
    Get { account: String },
}

#[derive(Subcommand)]
enum FeeCommands {
    /// Show current base fee and suggested priority fee
    Estimate,
    /// Show mempool depth and congestion
    Mempool,
}

#[derive(Subcommand)]
enum PeerCommerceCommands {
    /// Register a product listing on-chain
    Register {
        #[arg(long)] account: String,
        #[arg(long)] product_cid: String,
        #[arg(long)] price: u64,
        #[arg(long, default_value = "")] description: String,
        #[arg(long)] key_file: Option<std::path::PathBuf>,
    },
    /// List all peer commerce listings
    List,
}

#[derive(Subcommand)]
enum AgentCommands {
    /// Register this account as an agent
    Register {
        #[arg(long)] account: String,
        #[arg(long)] name: String,
        #[arg(long)] description: String,
        /// Comma-separated tools e.g. web_search,chain_read
        #[arg(long, value_delimiter = ',')] tools: Vec<String>,
        #[arg(long, default_value = "qwen2.5:0.5b")] model: String,
        #[arg(long, default_value_t = 0)] min_fee: u64,
        #[arg(long)] key_file: Option<PathBuf>,
    },
    /// Remove this account from the agent registry
    Deregister {
        #[arg(long)] account: String,
        #[arg(long)] key_file: Option<PathBuf>,
    },
    /// List registered agents (optionally filter by tool)
    List {
        #[arg(long)] tool: Option<String>,
    },
    /// Get a specific agent's registration
    Get {
        account: String,
    },
    /// Deposit HONE into agent credit balance
    Deposit {
        #[arg(long)] account: String,
        #[arg(long)] amount: u64,
        #[arg(long)] key_file: Option<PathBuf>,
    },
    /// Withdraw unused agent credit
    Withdraw {
        #[arg(long)] account: String,
        #[arg(long)] amount: u64,
        #[arg(long)] key_file: Option<PathBuf>,
    },
    /// Show agent credit balance
    Balance {
        account: String,
    },
    /// Post a new task to the marketplace
    Post {
        #[arg(long)] task_id: String,
        #[arg(long)] requester: String,
        #[arg(long)] description: String,
        #[arg(long, value_delimiter = ',')] tools: Vec<String>,
        #[arg(long)] max_fee: u64,
        #[arg(long, default_value_t = 2)] min_verifiers: u32,
        #[arg(long, default_value_t = 3)] bid_window_epochs: u64,
        #[arg(long)] key_file: Option<PathBuf>,
    },
    /// Get a task by ID
    Task {
        task_id: String,
    },
    /// List tasks (filter by requester, agent, or status)
    Tasks {
        #[arg(long)] requester: Option<String>,
        #[arg(long)] agent: Option<String>,
        #[arg(long)] status: Option<String>,
    },
    /// Bid on a task
    Bid {
        #[arg(long)] task_id: String,
        #[arg(long)] agent: String,
        #[arg(long)] proposed_fee: u64,
        #[arg(long)] key_file: Option<PathBuf>,
    },
    /// Assign a task to an agent (requester only)
    Assign {
        #[arg(long)] task_id: String,
        #[arg(long)] agent: String,
        #[arg(long)] fee: u64,
        #[arg(long)] signed_by: String,
        #[arg(long)] key_file: Option<PathBuf>,
    },
    /// Submit a completed task result (agent workers)
    Submit {
        #[arg(long)] task_id: String,
        #[arg(long)] agent: String,
        /// SHA-256(task_id|output|agent)
        #[arg(long)] result_hash: String,
        /// Optional CID of full output stored on HONE-FS
        #[arg(long, default_value = "")] output_cid: String,
        #[arg(long)] key_file: Option<PathBuf>,
    },
    /// Commit a verifier hash (verifier nodes, phase 1 of commit-reveal)
    VerifierCommit {
        #[arg(long)] task_id: String,
        #[arg(long)] verifier: String,
        /// SHA-256(result_hash + salt)
        #[arg(long)] commit_hash: String,
        #[arg(long)] key_file: Option<PathBuf>,
    },
    /// Reveal a verifier result (verifier nodes, phase 2 of commit-reveal)
    VerifierReveal {
        #[arg(long)] task_id: String,
        #[arg(long)] verifier: String,
        #[arg(long)] result_hash: String,
        #[arg(long)] salt: String,
        #[arg(long)] key_file: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum EnsembleCommands {
    /// Post an ensemble job (fan-out to N workers)
    Post {
        #[arg(long)] requester: String,
        #[arg(long)] input_hash: String,
        #[arg(long)] max_fee: u64,
        #[arg(long, default_value_t = 3)] n_workers: u64,
        #[arg(long)] model: Option<String>,
        #[arg(long)] key_file: Option<PathBuf>,
    },
    /// Submit a worker vote for an ensemble job
    Vote {
        #[arg(long)] job_id: String,
        #[arg(long)] worker: String,
        #[arg(long)] output_hash: String,
        #[arg(long)] key_file: Option<PathBuf>,
    },
    /// Get an ensemble job by ID
    Get { job_id: String },
    /// List all ensemble jobs
    List,
}

#[derive(Subcommand)]
enum SlashCommands {
    /// Submit a slash report against a validator
    Submit {
        #[arg(long)] reporter: String,
        #[arg(long)] accused: String,
        #[arg(long)] violation: String,
        #[arg(long)] evidence: String,
        #[arg(long)] key_file: Option<PathBuf>,
    },
    /// Vote on a slash appeal
    Appeal {
        #[arg(long)] slash_id: String,
        #[arg(long)] panelist: String,
        /// Pass --overturn to vote to overturn the slash
        #[arg(long)] overturn: bool,
        #[arg(long)] key_file: Option<PathBuf>,
    },
    /// Get a slash record by ID
    Get { slash_id: String },
    /// List all slash records
    List,
}

#[derive(Subcommand)]
enum BridgeCommands {
    /// Custodian funds the bridge (deposit ETH/BTC, receive wHONE)
    Fund {
        #[arg(long)] bridge_id: String,
        #[arg(long)] custodian: String,
        #[arg(long)] amount_hunits: u64,
        #[arg(long)] external_tx_hash: String,
        #[arg(long, default_value = "ethereum")] chain: String,
        #[arg(long)] key_file: Option<PathBuf>,
    },
    /// Wrap HONE → wHONE (sends to external chain)
    Wrap {
        #[arg(long)] account: String,
        #[arg(long)] amount_hunits: u64,
        #[arg(long)] external_address: String,
        #[arg(long, default_value = "ethereum")] chain: String,
        #[arg(long)] key_file: Option<PathBuf>,
    },
    /// Unwrap wHONE → HONE (queues an unlock request)
    Unwrap {
        #[arg(long)] account: String,
        #[arg(long)] amount_hunits: u64,
        #[arg(long)] recipient_external: String,
        #[arg(long, default_value = "ethereum")] chain: String,
        #[arg(long)] key_file: Option<PathBuf>,
    },
    /// Custodian marks an unlock request as fulfilled
    Unlock {
        #[arg(long)] request_id: String,
        #[arg(long)] custodian: String,
        #[arg(long)] external_tx_hash: String,
        #[arg(long)] key_file: Option<PathBuf>,
    },
    /// Show wHONE supply and cap utilization
    Status,
    /// Show pending unlock queue
    Queue,
}

#[derive(Subcommand)]
enum OracleCommands {
    /// Create a new oracle price feed
    Create {
        #[arg(long)] creator: String,
        #[arg(long)] feed_id: String,
        #[arg(long)] description: String,
        #[arg(long)] asset_pair: String,
        #[arg(long, default_value_t = 3)] min_reporters: u32,
        #[arg(long)] key_file: Option<PathBuf>,
    },
    /// Submit a price report to a feed
    Report {
        #[arg(long)] feed_id: String,
        #[arg(long)] reporter: String,
        #[arg(long)] value: String,
        #[arg(long)] commit_hash: Option<String>,
        #[arg(long)] key_file: Option<PathBuf>,
    },
    /// Finalize a feed round (reveal phase for commit-reveal)
    Finalize {
        #[arg(long)] feed_id: String,
        #[arg(long)] finalizer: String,
        #[arg(long)] reveal_value: Option<String>,
        #[arg(long)] reveal_salt: Option<String>,
        #[arg(long)] key_file: Option<PathBuf>,
    },
    /// Get a feed by ID
    Get { feed_id: String },
    /// List all oracle feeds
    List,
    /// Get latest price for an asset pair (e.g. BTC-USD)
    Price { pair: String },
    /// Get reporter reputation
    Reputation { reporter: String },
}

#[derive(Subcommand)]
enum VrfCommands {
    /// Submit a VRF commitment (clock nodes only)
    Commit {
        #[arg(long)] clock_node: String,
        #[arg(long)] commit_hash: String,
        #[arg(long)] key_file: Option<PathBuf>,
    },
    /// Reveal a VRF commitment
    Reveal {
        #[arg(long)] clock_node: String,
        #[arg(long)] reveal_value: String,
        #[arg(long)] salt: String,
        #[arg(long)] key_file: Option<PathBuf>,
    },
    /// Get the current VRF beacon value
    Beacon,
    /// Get VRF round for a specific epoch
    Round { epoch: u64 },
}

#[derive(Subcommand)]
enum SessionMarketCommands {
    /// Create a session listing (sell AI context window)
    Create {
        #[arg(long)] provider: String,
        #[arg(long)] context_summary: String,
        #[arg(long)] price_per_turn: u64,
        #[arg(long, default_value_t = 50)] max_turns: u32,
        #[arg(long)] key_file: Option<PathBuf>,
    },
    /// Buy a session listing
    Buy {
        #[arg(long)] listing_id: String,
        #[arg(long)] buyer: String,
        #[arg(long)] key_file: Option<PathBuf>,
    },
    /// Cancel a session listing (provider only)
    Cancel {
        #[arg(long)] listing_id: String,
        #[arg(long)] provider: String,
        #[arg(long)] key_file: Option<PathBuf>,
    },
    /// List all active session listings
    Listings,
    /// Get a specific listing
    Get { listing_id: String },
}

#[derive(Subcommand)]
enum AgentSessionCommands {
    /// Open a multi-turn agent session
    Open {
        #[arg(long)] requester: String,
        #[arg(long)] agent: String,
        #[arg(long, default_value_t = 20)] max_turns: u32,
        #[arg(long, value_delimiter = ',')] tools: Vec<String>,
        #[arg(long)] key_file: Option<PathBuf>,
    },
    /// Close an agent session
    Close {
        #[arg(long)] session_id: String,
        #[arg(long)] account: String,
        #[arg(long)] key_file: Option<PathBuf>,
    },
    /// Get session details and turn history
    Get { session_id: String },
    /// Send a turn in an agent session
    Turn {
        #[arg(long)] session_id: String,
        #[arg(long)] sender: String,
        #[arg(long)] message: String,
        #[arg(long)] key_file: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum TotpCommands {
    /// Generate a TOTP secret and return an otpauth:// URI
    Setup {
        #[arg(long)] account: String,
        #[arg(long)] key_file: Option<PathBuf>,
    },
    /// Activate TOTP by verifying the first code
    Enable {
        #[arg(long)] account: String,
        #[arg(long)] code: String,
        #[arg(long)] key_file: Option<PathBuf>,
    },
    /// Check a TOTP code (read-only, no signature required)
    Verify {
        #[arg(long)] account: String,
        #[arg(long)] code: String,
    },
    /// Disable TOTP (requires active code)
    Disable {
        #[arg(long)] account: String,
        #[arg(long)] code: String,
        #[arg(long)] key_file: Option<PathBuf>,
    },
    /// Generate single-use backup codes
    BackupCodes {
        #[arg(long)] account: String,
        #[arg(long)] key_file: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum PrivateAuthCommands {
    /// Enroll an account in M-of-N private authorization
    Enroll {
        #[arg(long)] account: String,
        #[arg(long, value_delimiter = ',')] approvers: Vec<String>,
        #[arg(long)] threshold: u32,
        #[arg(long)] key_file: Option<PathBuf>,
    },
    /// Vote to approve or reject a pending high-value transaction
    Approve {
        #[arg(long)] tx_hash: String,
        #[arg(long)] approver: String,
        /// Omit to reject; pass --approved to approve
        #[arg(long)] approved: bool,
        #[arg(long)] key_file: Option<PathBuf>,
    },
    /// Get authorization status for a pending transaction
    Status { tx_hash: String },
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
        Commands::AccountCreate { account, pubkey, key_file } => {
            tx::cmd_account_create(&account, pubkey.as_deref(), key_file.as_deref())?;
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
            KeyCommands::Register { account, role, key_file } => {
                key::cmd_key_register(&account, Some(&role), key_file.as_deref())?;
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

        Commands::Science { action } => match action {
            ScienceCommands::Create { account, title, job_type, input, max_fee, open_source, model } => {
                science::cmd_science_create(
                    &account, &title, &job_type, &input, max_fee, open_source,
                    model.as_deref(),
                )?;
            }
            ScienceCommands::Jobs { job_type } => {
                science::cmd_science_jobs(job_type.as_deref())?;
            }
            ScienceCommands::Job { id } => {
                science::cmd_science_job(&id)?;
            }
            ScienceCommands::Start { job_id, shard_group } => {
                science::cmd_science_start(&job_id, shard_group.as_deref())?;
            }
            ScienceCommands::Complete { job_id, result_hash, result, nodes, epoch } => {
                science::cmd_science_complete(&job_id, &result_hash, &result, &nodes, epoch)?;
            }
        },

        Commands::Memory { action } => match action {
            MemoryCommands::Set { account, key, value, key_file } => {
                memory::cmd_memory_set(&account, &key, &value, key_file.as_deref())?;
            }
            MemoryCommands::Get { account, key } => {
                memory::cmd_memory_get(&account, &key)?;
            }
            MemoryCommands::Del { account, key, key_file } => {
                memory::cmd_memory_delete(&account, &key, key_file.as_deref())?;
            }
            MemoryCommands::Scan { account, prefix } => {
                memory::cmd_memory_scan(&account, &prefix)?;
            }
        },

        Commands::Rag { action } => match action {
            RagCommands::Index { account, doc_id, content, key_file } => {
                memory::cmd_rag_index(&account, &doc_id, &content, key_file.as_deref())?;
            }
            RagCommands::Query { account, q } => {
                memory::cmd_rag_query(&account, &q)?;
            }
            RagCommands::Delete { account, doc_id, key_file } => {
                memory::cmd_rag_delete(&account, &doc_id, key_file.as_deref())?;
            }
        },

        Commands::Auction { action } => match action {
            AuctionCommands::Open { account, name, duration, key_file } => {
                auction::cmd_name_open(&account, &name, duration, key_file.as_deref())?;
            }
            AuctionCommands::Bid { account, auction_id, amount, key_file } => {
                auction::cmd_name_bid(&account, &auction_id, amount, key_file.as_deref())?;
            }
            AuctionCommands::Settle { account, auction_id, key_file } => {
                auction::cmd_name_settle(&account, &auction_id, key_file.as_deref())?;
            }
            AuctionCommands::Cancel { account, auction_id, key_file } => {
                auction::cmd_name_cancel(&account, &auction_id, key_file.as_deref())?;
            }
            AuctionCommands::Get { auction_id } => {
                auction::cmd_auction_get(&auction_id)?;
            }
        },

        Commands::Freeport { action } => match action {
            FreeportCommands::Open { account, item_id, item_type, duration, key_file } => {
                auction::cmd_freeport_open(&account, &item_id, &item_type, duration, key_file.as_deref())?;
            }
            FreeportCommands::Bid { account, auction_id, amount, key_file } => {
                auction::cmd_freeport_bid(&account, &auction_id, amount, key_file.as_deref())?;
            }
            FreeportCommands::Settle { account, auction_id, key_file } => {
                auction::cmd_freeport_settle(&account, &auction_id, key_file.as_deref())?;
            }
            FreeportCommands::Get { auction_id } => {
                auction::cmd_freeport_get(&auction_id)?;
            }
        },

        Commands::Finetune { action } => match action {
            FinetuneCommands::Post { account, base_model, dataset_cid, lora_rank, max_fee, key_file } => {
                finetune::cmd_finetune_post(&account, &base_model, &dataset_cid, lora_rank, max_fee, key_file.as_deref())?;
            }
            FinetuneCommands::Complete { worker, job_id, adapter_cid, key_file } => {
                finetune::cmd_finetune_complete(&worker, &job_id, &adapter_cid, key_file.as_deref())?;
            }
            FinetuneCommands::Get { job_id } => {
                finetune::cmd_finetune_get(&job_id)?;
            }
            FinetuneCommands::Jobs => {
                finetune::cmd_finetune_jobs()?;
            }
        },

        Commands::ComputerUse { action } => match action {
            ComputerUseCommands::Post { account, task, max_fee, key_file } => {
                finetune::cmd_cu_post(&account, &task, max_fee, key_file.as_deref())?;
            }
            ComputerUseCommands::Get { job_id } => {
                finetune::cmd_cu_get(&job_id)?;
            }
            ComputerUseCommands::Jobs => {
                finetune::cmd_cu_jobs()?;
            }
        },

        Commands::Snap { action } => match action {
            SnapCommands::Save { account, slug, cid, key_file } => {
                finetune::cmd_snap_save(&account, &slug, &cid, key_file.as_deref())?;
            }
            SnapCommands::Get { account, slug } => {
                finetune::cmd_snap_get(&account, &slug)?;
            }
            SnapCommands::List { account } => {
                finetune::cmd_snap_list(&account)?;
            }
        },

        Commands::AmberPill { action } => match action {
            AmberPillCommands::Mint { account, fingerprint, key_file } => {
                finetune::cmd_amber_pill_mint(&account, &fingerprint, key_file.as_deref())?;
            }
            AmberPillCommands::Get { account } => {
                finetune::cmd_amber_pill_get(&account)?;
            }
        },

        Commands::Fee { action } => match action {
            FeeCommands::Estimate => finetune::cmd_fee_estimate()?,
            FeeCommands::Mempool => finetune::cmd_mempool_status()?,
        },

        Commands::PeerCommerce { action } => match action {
            PeerCommerceCommands::Register { account, product_cid, price, description, key_file } => {
                finetune::cmd_peer_register(&account, &product_cid, price, &description, key_file.as_deref())?;
            }
            PeerCommerceCommands::List => {
                finetune::cmd_peer_list()?;
            }
        },

        Commands::Gateway { shortcode } => {
            finetune::cmd_gateway_resolve(&shortcode)?;
        }

        Commands::Agent { action } => match action {
            AgentCommands::Register { account, name, description, tools, model, min_fee, key_file } => {
                agent::cmd_agent_register(&account, &name, &description, tools, &model, min_fee, key_file.as_deref())?;
            }
            AgentCommands::Deregister { account, key_file } => {
                agent::cmd_agent_deregister(&account, key_file.as_deref())?;
            }
            AgentCommands::List { tool } => {
                agent::cmd_agent_registry_list(tool)?;
            }
            AgentCommands::Get { account } => {
                agent::cmd_agent_registry_get(&account)?;
            }
            AgentCommands::Deposit { account, amount, key_file } => {
                agent::cmd_agent_credit_deposit(&account, amount, key_file.as_deref())?;
            }
            AgentCommands::Withdraw { account, amount, key_file } => {
                agent::cmd_agent_credit_withdraw(&account, amount, key_file.as_deref())?;
            }
            AgentCommands::Balance { account } => {
                agent::cmd_agent_credit_balance(&account)?;
            }
            AgentCommands::Post { task_id, requester, description, tools, max_fee, min_verifiers, bid_window_epochs, key_file } => {
                agent::cmd_agent_task_post(&task_id, &requester, &description, tools, max_fee, min_verifiers, bid_window_epochs, key_file.as_deref())?;
            }
            AgentCommands::Task { task_id } => {
                agent::cmd_agent_task_get(&task_id)?;
            }
            AgentCommands::Tasks { requester, agent, status } => {
                agent::cmd_agent_tasks_list(requester, agent, status)?;
            }
            AgentCommands::Bid { task_id, agent, proposed_fee, key_file } => {
                agent::cmd_agent_task_bid(&task_id, &agent, proposed_fee, key_file.as_deref())?;
            }
            AgentCommands::Assign { task_id, agent, fee, signed_by, key_file } => {
                agent::cmd_agent_task_assign(&task_id, &agent, fee, &signed_by, key_file.as_deref())?;
            }
            AgentCommands::Submit { task_id, agent, result_hash, output_cid, key_file } => {
                agent::cmd_agent_task_submit(&task_id, &agent, &result_hash, &output_cid, key_file.as_deref())?;
            }
            AgentCommands::VerifierCommit { task_id, verifier, commit_hash, key_file } => {
                agent::cmd_agent_verifier_commit(&task_id, &verifier, &commit_hash, key_file.as_deref())?;
            }
            AgentCommands::VerifierReveal { task_id, verifier, result_hash, salt, key_file } => {
                agent::cmd_agent_verifier_reveal(&task_id, &verifier, &result_hash, &salt, key_file.as_deref())?;
            }
        },

        Commands::Ensemble { action } => match action {
            EnsembleCommands::Post { requester, input_hash, max_fee, n_workers, model, key_file } => {
                ensemble::cmd_ensemble_post(&requester, &input_hash, max_fee, n_workers, model, key_file.as_deref())?;
            }
            EnsembleCommands::Vote { job_id, worker, output_hash, key_file } => {
                ensemble::cmd_ensemble_vote(&job_id, &worker, &output_hash, key_file.as_deref())?;
            }
            EnsembleCommands::Get { job_id } => { ensemble::cmd_ensemble_get(&job_id)?; }
            EnsembleCommands::List => { ensemble::cmd_ensemble_list()?; }
        },

        Commands::Slash { action } => match action {
            SlashCommands::Submit { reporter, accused, violation, evidence, key_file } => {
                slash::cmd_slash_submit(&reporter, &accused, &violation, &evidence, key_file.as_deref())?;
            }
            SlashCommands::Appeal { slash_id, panelist, overturn, key_file } => {
                slash::cmd_slash_appeal(&slash_id, &panelist, overturn, key_file.as_deref())?;
            }
            SlashCommands::Get { slash_id } => { slash::cmd_slash_get(&slash_id)?; }
            SlashCommands::List => { slash::cmd_slash_list()?; }
        },

        Commands::Bridge { action } => match action {
            BridgeCommands::Fund { bridge_id, custodian, amount_hunits, external_tx_hash, chain, key_file } => {
                bridge::cmd_bridge_fund(&bridge_id, &custodian, amount_hunits, &external_tx_hash, &chain, key_file.as_deref())?;
            }
            BridgeCommands::Wrap { account, amount_hunits, external_address, chain, key_file } => {
                bridge::cmd_bridge_wrap(&account, amount_hunits, &external_address, &chain, key_file.as_deref())?;
            }
            BridgeCommands::Unwrap { account, amount_hunits, recipient_external, chain, key_file } => {
                bridge::cmd_bridge_unwrap(&account, amount_hunits, &recipient_external, &chain, key_file.as_deref())?;
            }
            BridgeCommands::Unlock { request_id, custodian, external_tx_hash, key_file } => {
                bridge::cmd_bridge_unlock(&request_id, &custodian, &external_tx_hash, key_file.as_deref())?;
            }
            BridgeCommands::Status => { bridge::cmd_bridge_status()?; }
            BridgeCommands::Queue  => { bridge::cmd_bridge_queue()?; }
        },

        Commands::Oracle { action } => match action {
            OracleCommands::Create { creator, feed_id, description, asset_pair, min_reporters, key_file } => {
                oracle::cmd_oracle_create(&creator, &feed_id, &description, &asset_pair, min_reporters, key_file.as_deref())?;
            }
            OracleCommands::Report { feed_id, reporter, value, commit_hash, key_file } => {
                oracle::cmd_oracle_report(&feed_id, &reporter, &value, commit_hash, key_file.as_deref())?;
            }
            OracleCommands::Finalize { feed_id, finalizer, reveal_value, reveal_salt, key_file } => {
                oracle::cmd_oracle_finalize(&feed_id, &finalizer, reveal_value, reveal_salt, key_file.as_deref())?;
            }
            OracleCommands::Get { feed_id } => { oracle::cmd_oracle_get(&feed_id)?; }
            OracleCommands::List => { oracle::cmd_oracle_list()?; }
            OracleCommands::Price { pair } => { oracle::cmd_oracle_price(&pair)?; }
            OracleCommands::Reputation { reporter } => { oracle::cmd_oracle_reputation(&reporter)?; }
        },

        Commands::Vrf { action } => match action {
            VrfCommands::Commit { clock_node, commit_hash, key_file } => {
                vrf::cmd_vrf_commit(&clock_node, &commit_hash, key_file.as_deref())?;
            }
            VrfCommands::Reveal { clock_node, reveal_value, salt, key_file } => {
                vrf::cmd_vrf_reveal(&clock_node, &reveal_value, &salt, key_file.as_deref())?;
            }
            VrfCommands::Beacon => { vrf::cmd_vrf_beacon()?; }
            VrfCommands::Round { epoch } => { vrf::cmd_vrf_round(epoch)?; }
        },

        Commands::SessionMarket { action } => match action {
            SessionMarketCommands::Create { provider, context_summary, price_per_turn, max_turns, key_file } => {
                sessions::cmd_session_list_create(&provider, &context_summary, price_per_turn, max_turns, key_file.as_deref())?;
            }
            SessionMarketCommands::Buy { listing_id, buyer, key_file } => {
                sessions::cmd_session_buy(&listing_id, &buyer, key_file.as_deref())?;
            }
            SessionMarketCommands::Cancel { listing_id, provider, key_file } => {
                sessions::cmd_session_cancel(&listing_id, &provider, key_file.as_deref())?;
            }
            SessionMarketCommands::Listings => { sessions::cmd_session_listings()?; }
            SessionMarketCommands::Get { listing_id } => { sessions::cmd_session_listing_get(&listing_id)?; }
        },

        Commands::AgentSession { action } => match action {
            AgentSessionCommands::Open { requester, agent, max_turns, tools, key_file } => {
                sessions::cmd_agent_session_open(&requester, &agent, max_turns, tools, key_file.as_deref())?;
            }
            AgentSessionCommands::Close { session_id, account, key_file } => {
                sessions::cmd_agent_session_close(&session_id, &account, key_file.as_deref())?;
            }
            AgentSessionCommands::Get { session_id } => { sessions::cmd_agent_session_get(&session_id)?; }
            AgentSessionCommands::Turn { session_id, sender, message, key_file } => {
                sessions::cmd_agent_session_turn(&session_id, &sender, &message, key_file.as_deref())?;
            }
        },

        Commands::Totp { action } => match action {
            TotpCommands::Setup { account, key_file } => {
                totp::cmd_totp_setup(&account, key_file.as_deref())?;
            }
            TotpCommands::Enable { account, code, key_file } => {
                totp::cmd_totp_enable(&account, &code, key_file.as_deref())?;
            }
            TotpCommands::Verify { account, code } => {
                totp::cmd_totp_verify(&account, &code)?;
            }
            TotpCommands::Disable { account, code, key_file } => {
                totp::cmd_totp_disable(&account, &code, key_file.as_deref())?;
            }
            TotpCommands::BackupCodes { account, key_file } => {
                totp::cmd_totp_backup_codes(&account, key_file.as_deref())?;
            }
        },

        Commands::PrivateAuth { action } => match action {
            PrivateAuthCommands::Enroll { account, approvers, threshold, key_file } => {
                private_auth::cmd_private_auth_enroll(&account, approvers, threshold, key_file.as_deref())?;
            }
            PrivateAuthCommands::Approve { tx_hash, approver, approved, key_file } => {
                private_auth::cmd_private_auth_approve(&tx_hash, &approver, approved, key_file.as_deref())?;
            }
            PrivateAuthCommands::Status { tx_hash } => {
                private_auth::cmd_private_auth_status(&tx_hash)?;
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
        .ok_or_else(|| anyhow::anyhow!("no owner in slug and no active session — use owner/repo or run `hone login`"))?;
    Ok((owner, slug.to_owned()))
}

fn cmd_login(account: &str, key_file: Option<&std::path::Path>, node_url: Option<&str>) -> Result<()> {
    use colored::Colorize;
    let key_path = session::resolve_key_file(key_file, None)?;
    // verify the key file is readable
    hone_sdk::KeyPair::from_file(&key_path)
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
            println!("Run: hone login --account <name> [--key-file <path>] [--node-url <url>]");
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
