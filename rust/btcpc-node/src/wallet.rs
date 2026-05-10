//! BTCPC multi-chain wallet — one BIP39 mnemonic, all supported chains.
//!
//! Derivation:
//!   BTCPC role keys — SLIP-10 m/44'/6942'/role_index'/0'  ed25519
//!     role 0 = owner    (key rotation, governance — store cold)
//!     role 1 = active   (transfers, staking — sign with active key)
//!     role 2 = posting  (daily activity — can be on device)
//!     role 3 = memo     (encrypted messages / selective disclosure)
//!     role 4 = hide     (encrypt private content for your eyes only)
//!     role 5 = seek     (encrypted digital delivery to buyers)
//!   Bitcoin         — BIP44  m/44'/0'/0'/0/0     secp256k1
//!   Ethereum + EVM  — BIP44  m/44'/60'/0'/0/0    secp256k1 (Polygon/BSC/Avax/Arb/Op/Base)
//!   XRP             — BIP44  m/44'/144'/0'/0/0   secp256k1
//!   Cosmos/ATOM     — BIP44  m/44'/118'/0'/0/0   secp256k1
//!   Tron            — BIP44  m/44'/195'/0'/0/0   secp256k1
//!   Solana          — SLIP-10 m/44'/501'/0'/0'   ed25519
//!   TON             — SLIP-10 m/44'/607'/0'      ed25519
//!   Stellar/XLM     — SLIP-10 m/44'/148'/0'/0'   ed25519
//!   NEAR            — SLIP-10 m/44'/397'/0'/0'   ed25519
//!   Sui             — SLIP-10 m/44'/784'/0'/0'   ed25519
//!   Aptos           — SLIP-10 m/44'/637'/0'/0'   ed25519
//!
//! Hive: NOT derived from mnemonic (password-based keys). Linked later via
//!       signed challenge → HiveLink entry on-chain.

use std::path::Path;
use anyhow::{bail, Context, Result};
use bech32::{ToBase32, Variant as Bech32Variant};
use bip39::{Language, Mnemonic};
use blake2::{Blake2b512, Digest as Blake2Digest};
use ed25519_dalek::SigningKey as Ed25519SigningKey;
use hmac::{Hmac, Mac};
use rand::RngCore;
use ripemd::Ripemd160;
use secp256k1::{PublicKey as SecpPubKey, Secp256k1, SecretKey as SecpSecretKey, Scalar};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256, Sha512};
use sha3::{Keccak256, Sha3_256};
use tracing::{info, warn};
use btcpc_types::ChainProof;

type HmacSha512 = Hmac<Sha512>;
const H: u32 = 0x8000_0000;

/// sha256(chain + ":" + address + ":" + nonce) as lowercase hex.
/// This is the on-chain commitment — the actual address never hits the chain.
fn chain_commitment(chain: &str, address: &str, nonce: &str) -> String {
    let mut h = Sha256::new();
    h.update(chain);
    h.update(b":");
    h.update(address);
    h.update(b":");
    h.update(nonce);
    hex::encode(h.finalize())
}

// ── Key bundle ────────────────────────────────────────────────────────────────

/// BTCPC coin type in SLIP-44 (coin index 6942 = BTCPC).
const BTCPC_COIN: u32 = 6942;

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct WalletKeys {
    pub mnemonic: String,

    // BTCPC role keys — SLIP-10 m/44'/6942'/role'/0'
    // Owner: key rotation and governance. Keep this one cold.
    pub btcpc_owner_private_key:   String,
    pub btcpc_owner_public_key:    String,
    // Active: transfers and staking.
    pub btcpc_active_private_key:  String,
    pub btcpc_active_public_key:   String,
    // Posting: daily ops. Safe to keep on device.
    pub btcpc_private_key: String,  // ← kept as canonical "posting" for back-compat
    pub btcpc_public_key:  String,
    // Memo: encrypted messages and selective disclosure.
    pub btcpc_memo_private_key:    String,
    pub btcpc_memo_public_key:     String,
    // Hide: encrypt private content for your own eyes only.
    pub btcpc_hide_private_key:    String,
    pub btcpc_hide_public_key:     String,
    // Seek: encrypted digital delivery to buyers.
    pub btcpc_seek_private_key:    String,
    pub btcpc_seek_public_key:     String,

    // Bitcoin — secp256k1 BIP44 m/44'/0'/0'/0/0
    pub bitcoin_wif:    String,
    pub bitcoin_pubkey: String,

    // Ethereum + all EVM — secp256k1 BIP44 m/44'/60'/0'/0/0
    pub ethereum_address:     String,
    pub ethereum_private_key: String,

    // XRP — secp256k1 BIP44 m/44'/144'/0'/0/0
    pub xrp_address:     String,
    pub xrp_private_key: String,

    // Cosmos/ATOM — secp256k1 BIP44 m/44'/118'/0'/0/0
    pub cosmos_address:     String,
    pub cosmos_private_key: String,

    // Tron — secp256k1 BIP44 m/44'/195'/0'/0/0
    pub tron_address:     String,
    pub tron_private_key: String,

    // Solana — ed25519 SLIP-10 m/44'/501'/0'/0'
    pub solana_address:     String,
    pub solana_private_key: String,

    // TON — ed25519 SLIP-10 m/44'/607'/0'
    pub ton_public_key:  String,
    pub ton_private_key: String,

    // Stellar/XLM — ed25519 SLIP-10 m/44'/148'/0'/0'
    pub stellar_address:     String,
    pub stellar_private_key: String,

    // NEAR — ed25519 SLIP-10 m/44'/397'/0'/0'
    pub near_address:     String,
    pub near_private_key: String,

    // Sui — ed25519 SLIP-10 m/44'/784'/0'/0'
    pub sui_address:     String,
    pub sui_private_key: String,

    // Aptos — ed25519 SLIP-10 m/44'/637'/0'/0'
    pub aptos_address:     String,
    pub aptos_private_key: String,

    /// Per-chain random nonces used to compute on-chain commitments.
    /// commitment = sha256(chain + ":" + address + ":" + nonce)
    /// Keep wallet.key backed up — you need these nonces for selective disclosure.
    #[serde(default)]
    pub chain_nonces: std::collections::HashMap<String, String>,
}

// ── Public API ────────────────────────────────────────────────────────────────

pub fn init(data_dir: &Path) -> Result<WalletKeys> {
    let key_path = data_dir.join("wallet.key");

    // 1. Env override — reinstall / migration
    if let Ok(raw) = std::env::var("BTCPC_POSTING_KEY") {
        let raw = raw.trim().to_owned();
        match restore_from_input(&raw) {
            Ok(keys) => {
                save(&key_path, &keys)?;
                info!(btcpc = %keys.btcpc_public_key, "wallet: restored from BTCPC_POSTING_KEY");
                return Ok(keys);
            }
            Err(e) => warn!("wallet: BTCPC_POSTING_KEY could not be parsed ({}), ignoring", e),
        }
    }

    // 2. Load existing wallet.key
    if key_path.exists() {
        let json = std::fs::read_to_string(&key_path)
            .with_context(|| format!("failed to read {}", key_path.display()))?;

        // Try current format
        if let Ok(keys) = serde_json::from_str::<WalletKeys>(&json) {
            // Upgrade if new chains or role keys are missing (mnemonic is the source of truth)
            if keys.xrp_address.is_empty() || keys.sui_address.is_empty()
                || keys.btcpc_owner_public_key.is_empty()
            {
                let upgraded = restore_from_input(&keys.mnemonic)?;
                save(&key_path, &upgraded)?;
                warn!(btcpc = %upgraded.btcpc_public_key, "wallet: upgraded to full multi-chain + role key format");
                return Ok(upgraded);
            }
            info!(btcpc = %keys.btcpc_public_key, "wallet: loaded from wallet.key");
            return Ok(keys);
        }

        // Legacy single-chain format migration
        #[derive(Deserialize, Default)]
        struct Legacy {
            #[serde(default)] mnemonic:          String,
            #[serde(default)] private_key:        String,
            #[serde(default)] btcpc_private_key:  String,
        }
        if let Ok(leg) = serde_json::from_str::<Legacy>(&json) {
            let input = [&leg.mnemonic, &leg.private_key, &leg.btcpc_private_key]
                .iter().find(|s| !s.is_empty()).copied().cloned()
                .ok_or_else(|| anyhow::anyhow!("legacy wallet.key has no usable key field"))?;
            let keys = restore_from_input(&input).context("upgrade legacy wallet.key")?;
            save(&key_path, &keys)?;
            warn!(btcpc = %keys.btcpc_public_key, "wallet: migrated legacy format → multi-chain");
            return Ok(keys);
        }

        bail!("wallet.key is unreadable — move it aside and restart to generate a fresh wallet");
    }

    // 3. Generate fresh wallet
    std::fs::create_dir_all(data_dir)
        .with_context(|| format!("failed to create data dir {}", data_dir.display()))?;
    let keys = generate_fresh()?;
    save(&key_path, &keys)?;
    print_new_wallet(&keys);
    Ok(keys)
}

/// Copy the wallet to ~/.btcpc/{account}.wallet.key and ~/.btcpc/{account}.txt so
/// TUI/CLI can find it without knowing the node's data directory.
/// Called every startup — safe to repeat. User can encrypt the ~/.btcpc/ folder.
pub fn backup_to_home(account: &str, keys: &WalletKeys) {
    let home = match std::env::var("HOME") {
        Ok(h) => h,
        Err(_) => return,
    };
    let dir = std::path::PathBuf::from(&home).join(".btcpc");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        warn!("wallet: could not create ~/.btcpc: {}", e);
        return;
    }

    let json_dest = dir.join(format!("{}.wallet.key", account));
    match save(&json_dest, keys) {
        Ok(_)  => info!("wallet: backed up to {}", json_dest.display()),
        Err(e) => warn!("wallet: could not back up to {}: {}", json_dest.display(), e),
    }

    let txt_dest = dir.join(format!("{}.txt", account));
    let txt = format_txt_backup(account, keys);
    match std::fs::write(&txt_dest, &txt) {
        Ok(_)  => info!("wallet: human-readable backup at {}", txt_dest.display()),
        Err(e) => warn!("wallet: could not write {}: {}", txt_dest.display(), e),
    }
}

fn format_txt_backup(account: &str, k: &WalletKeys) -> String {
    format!("\
============================================================
  {}
============================================================

  MNEMONIC (12 words — regenerates everything below)
  {}

  ── BTCPC ROLE KEYS (ed25519) ──────────────────────────

  owner
    public   {}
    private  {}
  active
    public   {}
    private  {}
  posting
    public   {}
    private  {}
  memo
    public   {}
    private  {}
  hide
    public   {}
    private  {}
  seek
    public   {}
    private  {}

  ── EXTERNAL CHAIN KEYS ────────────────────────────────

  ETHEREUM (+ Polygon, BSC, Arbitrum, Optimism, Base, Avalanche)
    address  {}
    private  {}

  BITCOIN
    pubkey   {}
    WIF      {}

  SOLANA
    address  {}
    private  {}

  NEAR
    address  {}
    private  {}

  SUI
    address  {}
    private  {}

  APTOS
    address  {}
    private  {}

  COSMOS / ATOM
    address  {}
    private  {}

  XRP
    address  {}
    private  {}

  TRON
    address  {}
    private  {}

  TON
    pubkey   {}
    private  {}

  STELLAR / XLM
    address  {}
    private  {}

============================================================
  Keep this file safe. Encrypt this folder.
  Never share private keys or mnemonic with anyone.
============================================================
",
        account.to_uppercase(),
        k.mnemonic,
        k.btcpc_owner_public_key,   k.btcpc_owner_private_key,
        k.btcpc_active_public_key,  k.btcpc_active_private_key,
        k.btcpc_public_key,         k.btcpc_private_key,
        k.btcpc_memo_public_key,    k.btcpc_memo_private_key,
        k.btcpc_hide_public_key,    k.btcpc_hide_private_key,
        k.btcpc_seek_public_key,    k.btcpc_seek_private_key,
        k.ethereum_address,   k.ethereum_private_key,
        k.bitcoin_pubkey,     k.bitcoin_wif,
        k.solana_address,     k.solana_private_key,
        k.near_address,       k.near_private_key,
        k.sui_address,        k.sui_private_key,
        k.aptos_address,      k.aptos_private_key,
        k.cosmos_address,     k.cosmos_private_key,
        k.xrp_address,        k.xrp_private_key,
        k.tron_address,       k.tron_private_key,
        k.ton_public_key,     k.ton_private_key,
        k.stellar_address,    k.stellar_private_key,
    )
}

/// Submit AccountCreate to local chain with all public keys. No-op if account exists.
/// If BTCPC_ACCOUNT_PUBKEY env var is set, uses that as the posting key (install-script path).
pub fn register_account(
    chain: &crate::chain::Chain,
    account: &str,
    keys: &WalletKeys,
) -> Result<()> {
    use btcpc_types::LedgerEntry;

    if chain.store.get_account(account)?.is_some() {
        return Ok(());
    }

    // Use externally supplied public key if provided (user has existing account on another device)
    let posting_key = std::env::var("BTCPC_ACCOUNT_PUBKEY")
        .ok()
        .filter(|k| k.len() == 64)
        .unwrap_or_else(|| keys.btcpc_public_key.clone());

    // All 6 BTCPC role keys — all public, registered on-chain for verification.
    let mut km: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    km.insert("owner".to_string(),   keys.btcpc_owner_public_key.clone());
    km.insert("active".to_string(),  keys.btcpc_active_public_key.clone());
    km.insert("posting".to_string(), posting_key);
    km.insert("memo".to_string(),    keys.btcpc_memo_public_key.clone());
    km.insert("hide".to_string(),    keys.btcpc_hide_public_key.clone());
    km.insert("seek".to_string(),    keys.btcpc_seek_public_key.clone());

    // Build easy-mode chain proofs: commitment per chain, no plaintext addresses on-chain.
    // The nonces live in wallet.key — back it up for future selective disclosure.
    let chain_addrs = [
        ("bitcoin",  &keys.bitcoin_pubkey),
        ("ethereum", &keys.ethereum_address),
        ("xrp",      &keys.xrp_address),
        ("cosmos",   &keys.cosmos_address),
        ("tron",     &keys.tron_address),
        ("solana",   &keys.solana_address),
        ("ton",      &keys.ton_public_key),
        ("stellar",  &keys.stellar_address),
        ("near",     &keys.near_address),
        ("sui",      &keys.sui_address),
        ("aptos",    &keys.aptos_address),
    ];
    let chain_proofs: Vec<ChainProof> = chain_addrs.iter().filter_map(|(chain, addr)| {
        keys.chain_nonces.get(*chain).map(|nonce| ChainProof {
            chain:      chain.to_string(),
            commitment: chain_commitment(chain, addr, nonce),
            mode:       "easy".to_string(),
        })
    }).collect();

    chain.apply_entry(&LedgerEntry::AccountCreate {
        account:             account.to_string(),
        keys:                km,
        chain_proofs,
        epoch:               0,
        funded_by:           None,
        machine_fingerprint: None,
    })?;

    info!(
        account,
        owner    = %keys.btcpc_owner_public_key,
        posting  = %keys.btcpc_public_key,
        eth      = %keys.ethereum_address,
        sol      = %keys.solana_address,
        "wallet: account registered on-chain with 6 role keys + all chain commitments"
    );
    Ok(())
}

// ── Derivation ────────────────────────────────────────────────────────────────

fn restore_from_input(raw: &str) -> Result<WalletKeys> {
    let words = raw.split_whitespace().count();
    if words >= 12 {
        let m = Mnemonic::parse_in(Language::English, raw)
            .map_err(|e| anyhow::anyhow!("invalid mnemonic: {}", e))?;
        return derive_all(m);
    }
    if raw.len() == 64 {
        let b: [u8; 32] = hex::decode(raw).context("hex decode")?
            .try_into().map_err(|_| anyhow::anyhow!("must be 32 bytes"))?;
        let m = Mnemonic::from_entropy(&b)
            .map_err(|e| anyhow::anyhow!("entropy→mnemonic: {}", e))?;
        return derive_all(m);
    }
    bail!("expected 12-word mnemonic or 64-char hex private key");
}

fn generate_fresh() -> Result<WalletKeys> {
    let mut entropy = [0u8; 16]; // 128 bits = 12 words (industry standard)
    rand::rngs::OsRng.fill_bytes(&mut entropy);
    derive_all(Mnemonic::from_entropy(&entropy)
        .map_err(|e| anyhow::anyhow!("mnemonic gen: {}", e))?)
}

fn derive_all(mnemonic: Mnemonic) -> Result<WalletKeys> {
    let seed = mnemonic.to_seed("");

    // ── BTCPC role keys — SLIP-10 m/44'/6942'/role'/0' ───────────────────────
    // role 0=owner, 1=active, 2=posting, 3=memo, 4=hide, 5=seek
    let (owner_priv,   owner_pub)   = slip10(&seed, &[44|H, BTCPC_COIN|H, 0|H, 0|H])?;
    let (active_priv,  active_pub)  = slip10(&seed, &[44|H, BTCPC_COIN|H, 1|H, 0|H])?;
    let (post_priv,    post_pub)    = slip10(&seed, &[44|H, BTCPC_COIN|H, 2|H, 0|H])?;
    let (memo_priv,    memo_pub)    = slip10(&seed, &[44|H, BTCPC_COIN|H, 3|H, 0|H])?;
    let (hide_priv,    hide_pub)    = slip10(&seed, &[44|H, BTCPC_COIN|H, 4|H, 0|H])?;
    let (seek_priv,    seek_pub)    = slip10(&seed, &[44|H, BTCPC_COIN|H, 5|H, 0|H])?;

    // ── secp256k1 BIP44 ──────────────────────────────────────────────────────
    let (master, mchain) = bip32_master(&seed)?;


    let (btc_sk, _)  = bip44_secp(&master, &mchain, 0)?;
    let (eth_sk, _)  = bip44_secp(&master, &mchain, 60)?;
    let (xrp_sk, _)  = bip44_secp(&master, &mchain, 144)?;
    let (atm_sk, _)  = bip44_secp(&master, &mchain, 118)?;
    let (trx_sk, _)  = bip44_secp(&master, &mchain, 195)?;

    // ── ed25519 SLIP-10 ───────────────────────────────────────────────────────
    let (sol_priv, sol_pub)  = slip10(&seed, &[44|H, 501|H, 0|H, 0|H])?;
    let (ton_priv, ton_pub)  = slip10(&seed, &[44|H, 607|H, 0|H])?;
    let (xlm_priv, xlm_pub)  = slip10(&seed, &[44|H, 148|H, 0|H, 0|H])?;
    let (near_priv, near_pub)= slip10(&seed, &[44|H, 397|H, 0|H, 0|H])?;
    let (sui_priv, sui_pub)  = slip10(&seed, &[44|H, 784|H, 0|H, 0|H])?;
    let (apt_priv, apt_pub)  = slip10(&seed, &[44|H, 637|H, 0|H, 0|H])?;

    // Derive addresses.
    let btc_addr   = wif(&btc_sk);
    let btc_pub    = cpk_hex(&btc_sk);
    let eth_addr_s = eth_addr(&eth_sk);
    let xrp_addr_s = xrp_addr(&xrp_sk);
    let atm_addr_s = cosmos_addr(&atm_sk);
    let trx_addr_s = tron_addr(&trx_sk);
    let sol_addr_s = bs58::encode(&sol_pub).into_string();
    let ton_pub_s  = hex::encode(&ton_pub);
    let xlm_addr_s = stellar_addr(&xlm_pub);
    let near_addr_s= bs58::encode(&near_pub).into_string();
    let sui_addr_s = sui_addr(&sui_pub);
    let apt_addr_s = aptos_addr(&apt_pub);

    // Generate a fresh random nonce per external chain.
    // These nonces are the key to selective disclosure — keep wallet.key backed up.
    let mut nonce_buf = [0u8; 16];
    let mut rng = rand::rngs::OsRng;
    let mut nonce = || -> String {
        rng.fill_bytes(&mut nonce_buf);
        hex::encode(nonce_buf)
    };
    let mut chain_nonces = std::collections::HashMap::new();
    chain_nonces.insert("bitcoin".into(),  nonce());
    chain_nonces.insert("ethereum".into(), nonce());
    chain_nonces.insert("xrp".into(),      nonce());
    chain_nonces.insert("cosmos".into(),   nonce());
    chain_nonces.insert("tron".into(),     nonce());
    chain_nonces.insert("solana".into(),   nonce());
    chain_nonces.insert("ton".into(),      nonce());
    chain_nonces.insert("stellar".into(),  nonce());
    chain_nonces.insert("near".into(),     nonce());
    chain_nonces.insert("sui".into(),      nonce());
    chain_nonces.insert("aptos".into(),    nonce());

    Ok(WalletKeys {
        mnemonic: mnemonic.to_string(),

        btcpc_owner_private_key:  hex::encode(&owner_priv),
        btcpc_owner_public_key:   hex::encode(&owner_pub),
        btcpc_active_private_key: hex::encode(&active_priv),
        btcpc_active_public_key:  hex::encode(&active_pub),
        btcpc_private_key:        hex::encode(&post_priv),
        btcpc_public_key:         hex::encode(&post_pub),
        btcpc_memo_private_key:   hex::encode(&memo_priv),
        btcpc_memo_public_key:    hex::encode(&memo_pub),
        btcpc_hide_private_key:   hex::encode(&hide_priv),
        btcpc_hide_public_key:    hex::encode(&hide_pub),
        btcpc_seek_private_key:   hex::encode(&seek_priv),
        btcpc_seek_public_key:    hex::encode(&seek_pub),

        bitcoin_wif:    btc_addr.clone(),
        bitcoin_pubkey: btc_pub,

        ethereum_address:     eth_addr_s.clone(),
        ethereum_private_key: hex::encode(eth_sk.secret_bytes()),

        xrp_address:     xrp_addr_s.clone(),
        xrp_private_key: hex::encode(xrp_sk.secret_bytes()),

        cosmos_address:     atm_addr_s.clone(),
        cosmos_private_key: hex::encode(atm_sk.secret_bytes()),

        tron_address:     trx_addr_s.clone(),
        tron_private_key: hex::encode(trx_sk.secret_bytes()),

        solana_address:     sol_addr_s.clone(),
        solana_private_key: hex::encode(&sol_priv),

        ton_public_key:  ton_pub_s.clone(),
        ton_private_key: hex::encode(&ton_priv),

        stellar_address:     xlm_addr_s.clone(),
        stellar_private_key: hex::encode(&xlm_priv),

        near_address:     near_addr_s.clone(),
        near_private_key: hex::encode(&near_priv),

        sui_address:     sui_addr_s.clone(),
        sui_private_key: hex::encode(&sui_priv),

        aptos_address:     apt_addr_s.clone(),
        aptos_private_key: hex::encode(&apt_priv),

        chain_nonces: chain_nonces.clone(),
    })
}

// ── BIP32 secp256k1 ───────────────────────────────────────────────────────────

fn bip32_master(seed: &[u8]) -> Result<(SecpSecretKey, [u8; 32])> {
    let mut mac = HmacSha512::new_from_slice(b"Bitcoin seed").unwrap();
    mac.update(seed);
    let i = mac.finalize().into_bytes();
    Ok((
        SecpSecretKey::from_slice(&i[..32]).map_err(|e| anyhow::anyhow!("master key: {}", e))?,
        i[32..].try_into().unwrap(),
    ))
}

fn bip32_hard(sk: &SecpSecretKey, chain: &[u8; 32], idx: u32) -> Result<(SecpSecretKey, [u8; 32])> {
    let mut mac = HmacSha512::new_from_slice(chain).unwrap();
    mac.update(&[0x00]);
    mac.update(&sk.secret_bytes());
    mac.update(&(idx | H).to_be_bytes());
    let i = mac.finalize().into_bytes();
    let child = SecpSecretKey::from_slice(&i[..32])
        .map_err(|e| anyhow::anyhow!("hard child: {}", e))?
        .add_tweak(&Scalar::from_be_bytes(sk.secret_bytes()).map_err(|_| anyhow::anyhow!("scalar"))?)
        .map_err(|e| anyhow::anyhow!("tweak: {}", e))?;
    Ok((child, i[32..].try_into().unwrap()))
}

fn bip32_norm(sk: &SecpSecretKey, chain: &[u8; 32], idx: u32) -> Result<(SecpSecretKey, [u8; 32])> {
    let secp = Secp256k1::new();
    let mut mac = HmacSha512::new_from_slice(chain).unwrap();
    mac.update(&SecpPubKey::from_secret_key(&secp, sk).serialize());
    mac.update(&idx.to_be_bytes());
    let i = mac.finalize().into_bytes();
    let child = SecpSecretKey::from_slice(&i[..32])
        .map_err(|e| anyhow::anyhow!("norm child: {}", e))?
        .add_tweak(&Scalar::from_be_bytes(sk.secret_bytes()).map_err(|_| anyhow::anyhow!("scalar"))?)
        .map_err(|e| anyhow::anyhow!("tweak: {}", e))?;
    Ok((child, i[32..].try_into().unwrap()))
}

/// BIP44 m/44'/coin'/0'/0/0
fn bip44_secp(master: &SecpSecretKey, mchain: &[u8; 32], coin: u32) -> Result<(SecpSecretKey, [u8; 32])> {
    let (k1,c1) = bip32_hard(master, mchain, 44)?;
    let (k2,c2) = bip32_hard(&k1, &c1, coin)?;
    let (k3,c3) = bip32_hard(&k2, &c2, 0)?;
    let (k4,c4) = bip32_norm(&k3, &c3, 0)?;
    bip32_norm(&k4, &c4, 0)
}

// ── SLIP-10 ed25519 ───────────────────────────────────────────────────────────

fn slip10(seed: &[u8], path: &[u32]) -> Result<([u8; 32], [u8; 32])> {
    let mut mac = HmacSha512::new_from_slice(b"ed25519 seed").unwrap();
    mac.update(seed);
    let i = mac.finalize().into_bytes();
    let mut key:   [u8; 32] = i[..32].try_into().unwrap();
    let mut chain: [u8; 32] = i[32..].try_into().unwrap();
    for &idx in path {
        let mut mac = HmacSha512::new_from_slice(&chain).unwrap();
        mac.update(&[0x00]);
        mac.update(&key);
        mac.update(&(idx | H).to_be_bytes());
        let i = mac.finalize().into_bytes();
        key.copy_from_slice(&i[..32]);
        chain.copy_from_slice(&i[32..]);
    }
    let pub_key: [u8; 32] = Ed25519SigningKey::from_bytes(&key).verifying_key().to_bytes();
    Ok((key, pub_key))
}

// ── Address helpers ───────────────────────────────────────────────────────────

fn cpk_hex(sk: &SecpSecretKey) -> String {
    hex::encode(SecpPubKey::from_secret_key(&Secp256k1::new(), sk).serialize())
}

fn wif(sk: &SecpSecretKey) -> String {
    let mut p = vec![0x80u8];
    p.extend_from_slice(&sk.secret_bytes());
    p.push(0x01);
    let cs = Sha256::digest(Sha256::digest(&p));
    p.extend_from_slice(&cs[..4]);
    bs58::encode(p).into_string()
}

fn hash160(data: &[u8]) -> [u8; 20] {
    let sha = Sha256::digest(data);
    Ripemd160::digest(sha).into()
}

fn eth_addr(sk: &SecpSecretKey) -> String {
    let unc = SecpPubKey::from_secret_key(&Secp256k1::new(), sk).serialize_uncompressed();
    let h: [u8; 32] = Keccak256::digest(&unc[1..]).into();
    eip55(&h[12..])
}

fn eip55(addr: &[u8]) -> String {
    let hex_lower = hex::encode(addr);
    let hash: [u8; 32] = Keccak256::digest(hex_lower.as_bytes()).into();
    let mut out = String::from("0x");
    for (i, c) in hex_lower.chars().enumerate() {
        let nib = (hash[i/2] >> (if i%2==0 {4} else {0})) & 0x0f;
        if c.is_alphabetic() && nib >= 8 { out.extend(c.to_uppercase()); } else { out.push(c); }
    }
    out
}

fn xrp_addr(sk: &SecpSecretKey) -> String {
    let h160 = hash160(&SecpPubKey::from_secret_key(&Secp256k1::new(), sk).serialize());
    let mut payload = vec![0x00u8]; // XRP version byte
    payload.extend_from_slice(&h160);
    let cs = Sha256::digest(Sha256::digest(&payload));
    payload.extend_from_slice(&cs[..4]);
    bs58::encode(payload).with_alphabet(bs58::Alphabet::BITCOIN).into_string()
}

fn cosmos_addr(sk: &SecpSecretKey) -> String {
    let h160 = hash160(&SecpPubKey::from_secret_key(&Secp256k1::new(), sk).serialize());
    bech32::encode("cosmos", h160.to_base32(), Bech32Variant::Bech32).unwrap_or_default()
}

fn tron_addr(sk: &SecpSecretKey) -> String {
    let unc = SecpPubKey::from_secret_key(&Secp256k1::new(), sk).serialize_uncompressed();
    let h: [u8; 32] = Keccak256::digest(&unc[1..]).into();
    let mut payload = vec![0x41u8]; // Tron mainnet prefix
    payload.extend_from_slice(&h[12..]);
    let cs = Sha256::digest(Sha256::digest(&payload));
    payload.extend_from_slice(&cs[..4]);
    bs58::encode(payload).with_alphabet(bs58::Alphabet::BITCOIN).into_string()
}

fn stellar_addr(pub_key: &[u8; 32]) -> String {
    // StrKey encoding: version_byte=6<<3=0x30 (G), CRC16-XModem checksum LE
    let mut payload = vec![0x30u8]; // account ID version byte
    payload.extend_from_slice(pub_key);
    let crc = crc16_xmodem(&payload).to_le_bytes();
    payload.extend_from_slice(&crc);
    base32_encode(&payload)
}

fn crc16_xmodem(data: &[u8]) -> u16 {
    let mut crc: u16 = 0x0000;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
        }
    }
    crc
}

fn base32_encode(data: &[u8]) -> String {
    const ALPHA: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = Vec::new();
    let mut bits: u32 = 0;
    let mut count = 0u32;
    for &b in data {
        bits = (bits << 8) | b as u32;
        count += 8;
        while count >= 5 {
            count -= 5;
            out.push(ALPHA[((bits >> count) & 0x1f) as usize]);
        }
    }
    if count > 0 { out.push(ALPHA[((bits << (5 - count)) & 0x1f) as usize]); }
    String::from_utf8(out).unwrap()
}

fn sui_addr(pub_key: &[u8; 32]) -> String {
    // Blake2b-256 of [scheme_flag=0x00, ...pub_key]
    use blake2::Digest as _;
    let mut h = blake2::Blake2b::<blake2::digest::consts::U32>::new();
    h.update(&[0x00u8]);
    h.update(pub_key);
    format!("0x{}", hex::encode(h.finalize()))
}

fn aptos_addr(pub_key: &[u8; 32]) -> String {
    // SHA3-256 of pub_key + [scheme_flag=0x00]
    let mut h = Sha3_256::new();
    h.update(pub_key);
    h.update(&[0x00u8]);
    format!("0x{}", hex::encode(h.finalize()))
}

// ── Display ───────────────────────────────────────────────────────────────────

fn print_new_wallet(k: &WalletKeys) {
    println!("\n\
╔══════════════════════════════════════════════════════════════════════════════════╗\n\
║          NEW BTCPC WALLET — WRITE DOWN YOUR MNEMONIC NOW                       ║\n\
╠══════════════════════════════════════════════════════════════════════════════════╣\n\
║  MNEMONIC (12 words — master key for every chain below):                       ║\n\
║                                                                                  ║\n\
║  {:<80}║\n\
║                                                                                  ║\n\
╠══════════════════════════════════════════════════════════════════════════════════╣\n\
║  BTCPC OWNER KEY  (SLIP-10 m/44'/6942'/0'/0') — store cold, key rotation only  ║\n\
║    Public key  : {:<64}║\n\
║    Private key : {:<64}║\n\
╠══════════════════════════════════════════════════════════════════════════════════╣\n\
║  BTCPC ACTIVE KEY  (SLIP-10 m/44'/6942'/1'/0') — transfers and staking         ║\n\
║    Public key  : {:<64}║\n\
║    Private key : {:<64}║\n\
╠══════════════════════════════════════════════════════════════════════════════════╣\n\
║  BTCPC POSTING KEY  (SLIP-10 m/44'/6942'/2'/0') — daily activity on device     ║\n\
║    Public key  : {:<64}║\n\
║    Private key : {:<64}║\n\
╠══════════════════════════════════════════════════════════════════════════════════╣\n\
║  BTCPC MEMO KEY  (SLIP-10 m/44'/6942'/3'/0') — encrypted messages             ║\n\
║    Public key  : {:<64}║\n\
║    Private key : {:<64}║\n\
╠══════════════════════════════════════════════════════════════════════════════════╣\n\
║  BTCPC HIDE KEY  (SLIP-10 m/44'/6942'/4'/0') — private content encryption     ║\n\
║    Public key  : {:<64}║\n\
║    Private key : {:<64}║\n\
╠══════════════════════════════════════════════════════════════════════════════════╣\n\
║  BTCPC SEEK KEY  (SLIP-10 m/44'/6942'/5'/0') — encrypted delivery to buyers   ║\n\
║    Public key  : {:<64}║\n\
║    Private key : {:<64}║\n\
╠══════════════════════════════════════════════════════════════════════════════════╣\n\
║  BITCOIN  (BIP44 m/44'/0'/0'/0/0)                                               ║\n\
║    Public key  : {:<64}║\n\
║    WIF key     : {:<64}║\n\
╠══════════════════════════════════════════════════════════════════════════════════╣\n\
║  ETHEREUM / POLYGON / BSC / AVALANCHE / ARBITRUM / OPTIMISM / BASE              ║\n\
║  (BIP44 m/44'/60'/0'/0/0 — same address on all EVM chains)                      ║\n\
║    Address     : {:<64}║\n\
║    Private key : {:<64}║\n\
╠══════════════════════════════════════════════════════════════════════════════════╣\n\
║  XRP  (BIP44 m/44'/144'/0'/0/0)                                                 ║\n\
║    Address     : {:<64}║\n\
║    Private key : {:<64}║\n\
╠══════════════════════════════════════════════════════════════════════════════════╣\n\
║  COSMOS/ATOM  (BIP44 m/44'/118'/0'/0/0)                                         ║\n\
║    Address     : {:<64}║\n\
║    Private key : {:<64}║\n\
╠══════════════════════════════════════════════════════════════════════════════════╣\n\
║  TRON  (BIP44 m/44'/195'/0'/0/0)                                                ║\n\
║    Address     : {:<64}║\n\
║    Private key : {:<64}║\n\
╠══════════════════════════════════════════════════════════════════════════════════╣\n\
║  SOLANA  (SLIP-10 m/44'/501'/0'/0')                                             ║\n\
║    Address     : {:<64}║\n\
║    Private key : {:<64}║\n\
╠══════════════════════════════════════════════════════════════════════════════════╣\n\
║  TON  (SLIP-10 m/44'/607'/0')                                                   ║\n\
║    Public key  : {:<64}║\n\
║    Private key : {:<64}║\n\
╠══════════════════════════════════════════════════════════════════════════════════╣\n\
║  STELLAR/XLM  (SLIP-10 m/44'/148'/0'/0')                                        ║\n\
║    Address     : {:<64}║\n\
║    Private key : {:<64}║\n\
╠══════════════════════════════════════════════════════════════════════════════════╣\n\
║  NEAR  (SLIP-10 m/44'/397'/0'/0')                                               ║\n\
║    Address     : {:<64}║\n\
║    Private key : {:<64}║\n\
╠══════════════════════════════════════════════════════════════════════════════════╣\n\
║  SUI  (SLIP-10 m/44'/784'/0'/0')                                                ║\n\
║    Address     : {:<64}║\n\
║    Private key : {:<64}║\n\
╠══════════════════════════════════════════════════════════════════════════════════╣\n\
║  APTOS  (SLIP-10 m/44'/637'/0'/0')                                              ║\n\
║    Address     : {:<64}║\n\
║    Private key : {:<64}║\n\
╠══════════════════════════════════════════════════════════════════════════════════╣\n\
║  Saved to: ~/.btcpc/wallet.key (chmod 600)                                      ║\n\
║  All public keys registered on-chain — your identity spans every chain.         ║\n\
╚══════════════════════════════════════════════════════════════════════════════════╝\n",
        k.mnemonic,
        k.btcpc_owner_public_key,   k.btcpc_owner_private_key,
        k.btcpc_active_public_key,  k.btcpc_active_private_key,
        k.btcpc_public_key,         k.btcpc_private_key,
        k.btcpc_memo_public_key,    k.btcpc_memo_private_key,
        k.btcpc_hide_public_key,    k.btcpc_hide_private_key,
        k.btcpc_seek_public_key,    k.btcpc_seek_private_key,
        k.bitcoin_pubkey,     k.bitcoin_wif,
        k.ethereum_address,   k.ethereum_private_key,
        k.xrp_address,        k.xrp_private_key,
        k.cosmos_address,     k.cosmos_private_key,
        k.tron_address,       k.tron_private_key,
        k.solana_address,     k.solana_private_key,
        k.ton_public_key,     k.ton_private_key,
        k.stellar_address,    k.stellar_private_key,
        k.near_address,       k.near_private_key,
        k.sui_address,        k.sui_private_key,
        k.aptos_address,      k.aptos_private_key,
    );
}

// ── Persistence ───────────────────────────────────────────────────────────────

fn save(path: &Path, keys: &WalletKeys) -> Result<()> {
    std::fs::write(path, serde_json::to_string_pretty(keys)?)
        .with_context(|| format!("failed to write {}", path.display()))?;
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("chmod 600 {}", path.display()))?;
    }
    Ok(())
}

