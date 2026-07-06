//! `hone wallet` — recoverable wallet creation and unlock.
//!
//! Every wallet created here writes an encrypted, recoverable keystore
//! (`<account>.keystore.json`, Argon2id + AES-256-GCM) into a local vault, AND
//! displays the recovery phrase once. This is the fix for the gap that made
//! accounts unrecoverable (see docs/GENESIS_JULY4_2026.md): you can no longer
//! create a wallet that leaves no recoverable file.
//!
//! Password handling: the keystore password is read from the HONE_WALLET_PASSWORD
//! env var (never a CLI arg — args leak into process lists and shell history).
//! The password is never printed, and never written anywhere except as the
//! Argon2id-derived encryption of the mnemonic.
//!
//! Subcommands:
//!   new      --account NAME --vault DIR
//!   import   --account NAME --vault DIR         (mnemonic via HONE_WALLET_MNEMONIC)
//!   unlock   --keystore FILE                    (verifies password; prints PUBLIC keys)
//!   pubkeys  --keystore FILE                    (public keys for a genesis entry)

use anyhow::{anyhow, bail, Context, Result};
use hone_sdk::keystore::Keystore;
use hone_sdk::Wallet;
use std::path::{Path, PathBuf};

pub fn run(args: &[String]) -> Result<i32> {
    // `args` is everything after "wallet", so the subcommand is at index 0.
    match args.get(0).map(String::as_str) {
        Some("new") => cmd_new(&args[1..]),
        Some("import") => cmd_import(&args[1..]),
        Some("unlock") => cmd_unlock(&args[1..]),
        Some("pubkeys") => cmd_pubkeys(&args[1..]),
        Some("index") => cmd_index(&args[1..]),
        Some("backup") => cmd_backup(&args[1..]),
        Some("restore") => cmd_restore(&args[1..]),
        Some("verify-vault") => cmd_verify_vault(&args[1..]),
        Some("export-node-key") => cmd_export_node_key(&args[1..]),
        Some("export-all") => cmd_export_all(&args[1..]),
        _ => {
            eprintln!(
                "usage:\n  \
                 hone wallet new    --account NAME --vault DIR\n  \
                 hone wallet import --account NAME --vault DIR   (HONE_WALLET_MNEMONIC=...)\n  \
                 hone wallet unlock --keystore FILE\n  \
                 hone wallet pubkeys --keystore FILE\n  \
                 hone wallet index  --vault DIR                 (writes INDEX.md, public only)\n  \
                 hone wallet backup  --keystore FILE --node URL (Layer 3: upload ciphertext only)\n  \
                 hone wallet restore --account NAME --node URL --vault DIR (fetch ciphertext)\n  \
                 hone wallet verify-vault --vault DIR --genesis genesis.json [--require-accounts FILE]\n  \
                 hone wallet export-node-key --keystore FILE [--role posting] [--out node.env | --print]\n\n\
                 Password comes from HONE_WALLET_PASSWORD (never a CLI arg)."
            );
            Ok(1)
        }
    }
}

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(String::as_str)
}

fn require_password() -> Result<String> {
    let pw = std::env::var("HONE_WALLET_PASSWORD")
        .map_err(|_| anyhow!("set HONE_WALLET_PASSWORD (the keystore password). It is never printed or stored in plaintext."))?;
    if pw.is_empty() {
        bail!("HONE_WALLET_PASSWORD is empty");
    }
    Ok(pw)
}

fn keystore_path(vault: &str, account: &str) -> PathBuf {
    Path::new(vault).join(format!("{account}.keystore.json"))
}

/// Print the wallet's public keys (never private) and the derived genesis entry.
fn print_public(wallet: &Wallet) -> Result<()> {
    let roles = wallet.hone_role_public_keys()?;
    println!("account: {}", wallet.account);
    println!("public keys (roles):");
    // Stable order for readability.
    for role in ["owner", "active", "posting", "memo", "hide", "seek"] {
        if let Some(pk) = roles.get(role) {
            println!("  {role:8} {pk}");
        }
    }
    // Genesis entry snippet — posting key is what genesis.json uses.
    if let Some(posting) = roles.get("posting") {
        println!("\ngenesis.json entry:");
        println!(
            "  \"{}\": {{ \"keys\": {{ \"posting\": \"{}\" }} }}",
            wallet.account, posting
        );
    }
    Ok(())
}

/// Create a new wallet: generate mnemonic, show it once, write encrypted keystore
/// + a public identity file, print public keys.
fn cmd_new(args: &[String]) -> Result<i32> {
    let account = flag(args, "--account").ok_or_else(|| anyhow!("--account NAME required"))?;
    let vault = flag(args, "--vault").ok_or_else(|| anyhow!("--vault DIR required"))?;
    let password = require_password()?;

    let ks_path = keystore_path(vault, account);
    if ks_path.exists() {
        bail!("keystore already exists at {} — refusing to overwrite. Delete it first if you really mean to.", ks_path.display());
    }

    let wallet = Wallet::generate(account).context("generating wallet")?;
    let phrase = wallet.mnemonic.to_string();

    // Encrypt the mnemonic into the recoverable keystore.
    let ks = Keystore::seal(account, &phrase, &password)?;
    ks.save(&ks_path)?;

    // Also write the public identity file (no secrets) next to the keystore.
    let id_path = Path::new(vault).join(format!("{account}.wallet.json"));
    wallet.save_to_file(&id_path)?;

    // Show the recovery phrase ONCE. This is the layer-2 backup the old flow
    // never actually delivered.
    println!("╔══════════════════════════════════════════════════════════════════╗");
    println!("║  RECOVERY PHRASE for '{account}' — WRITE THIS DOWN NOW.            ");
    println!("║  It is shown once. It is the ultimate backup if the keystore file  ║");
    println!("║  and its password are ever lost. Keep it offline and private.      ║");
    println!("╚══════════════════════════════════════════════════════════════════╝");
    println!("\n    {phrase}\n");
    println!("Keystore (encrypted, recoverable): {}", ks_path.display());
    println!("Identity file (public only):        {}", id_path.display());
    println!();
    print_public(&wallet)?;
    println!("\nStore the keystore file safely — with your password it fully recovers this account.");
    Ok(0)
}

/// Import an existing mnemonic (via HONE_WALLET_MNEMONIC) into a keystore.
fn cmd_import(args: &[String]) -> Result<i32> {
    let account = flag(args, "--account").ok_or_else(|| anyhow!("--account NAME required"))?;
    let vault = flag(args, "--vault").ok_or_else(|| anyhow!("--vault DIR required"))?;
    let password = require_password()?;
    let phrase = std::env::var("HONE_WALLET_MNEMONIC")
        .map_err(|_| anyhow!("set HONE_WALLET_MNEMONIC (the phrase to import). It is never printed."))?;
    let phrase = phrase.trim();

    let wallet = Wallet::from_phrase(phrase, account).context("invalid mnemonic")?;
    let ks_path = keystore_path(vault, account);
    if ks_path.exists() {
        bail!("keystore already exists at {} — refusing to overwrite.", ks_path.display());
    }
    let ks = Keystore::seal(account, phrase, &password)?;
    ks.save(&ks_path)?;
    let id_path = Path::new(vault).join(format!("{account}.wallet.json"));
    wallet.save_to_file(&id_path)?;

    println!("Imported '{account}' into recoverable keystore: {}", ks_path.display());
    print_public(&wallet)?;
    Ok(0)
}

/// Unlock a keystore (verify password) and print PUBLIC keys. Never prints the
/// mnemonic or any private key.
fn cmd_unlock(args: &[String]) -> Result<i32> {
    let ks_file = flag(args, "--keystore").ok_or_else(|| anyhow!("--keystore FILE required"))?;
    let password = require_password()?;
    let ks = Keystore::load(Path::new(ks_file))?;
    let phrase = ks.open(&password)?; // errors on wrong password / tamper
    let wallet = Wallet::from_phrase(&phrase, &ks.account)?;
    println!("unlocked ✓ (password correct, file intact)");
    print_public(&wallet)?;
    // phrase drops here; never printed.
    Ok(0)
}

/// Print only the public keys for a keystore — same as unlock but framed for
/// building a genesis entry. Requires the password (must decrypt to derive).
fn cmd_pubkeys(args: &[String]) -> Result<i32> {
    cmd_unlock(args)
}

/// Build `<vault>/INDEX.md` — a public-only readable index of the vault: each
/// account, its role public keys, and whether it has a recoverable keystore.
/// Reads ONLY the public `*.wallet.json` identity files and lists which
/// `*.keystore.json` exist. Never opens a keystore, never needs a password,
/// never touches a secret.
fn cmd_index(args: &[String]) -> Result<i32> {
    let vault = flag(args, "--vault").ok_or_else(|| anyhow!("--vault DIR required"))?;
    let dir = Path::new(vault);
    if !dir.is_dir() {
        bail!("vault {} is not a directory", dir.display());
    }

    // Collect accounts from identity files; note which have a keystore.
    let mut rows: Vec<(String, Option<String>, bool)> = Vec::new(); // (account, posting_pk, has_keystore)
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if let Some(account) = name.strip_suffix(".wallet.json") {
            let raw = std::fs::read_to_string(&path).unwrap_or_default();
            let v: serde_json::Value = serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null);
            let posting = v["hone_role_public_keys"]["posting"]
                .as_str()
                .or_else(|| v["hone_public_key_hex"].as_str())
                .map(|s| s.to_string());
            let has_ks = dir.join(format!("{account}.keystore.json")).exists();
            rows.push((account.to_string(), posting, has_ks));
        }
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));

    let mut md = String::new();
    md.push_str("# HONE Wallet Vault — INDEX\n\n");
    md.push_str(
        "Public information only. This index lists the accounts in this local vault, their \
         posting public keys (for genesis), and whether each has a recoverable encrypted \
         keystore. **No private keys, mnemonics, or passwords are ever stored here.**\n\n",
    );
    md.push_str(&format!("Accounts: {}\n\n", rows.len()));
    md.push_str("| Account | Recoverable keystore | Posting public key |\n");
    md.push_str("|---|---|---|\n");
    for (account, posting, has_ks) in &rows {
        let ks = if *has_ks { "✓ yes" } else { "✗ MISSING" };
        let pk = posting.clone().unwrap_or_else(|| "(unknown)".into());
        md.push_str(&format!("| `{account}` | {ks} | `{pk}` |\n"));
    }
    md.push_str(
        "\n---\nRecovery: each account's mnemonic is inside its `*.keystore.json`, unlockable \
         with `hone wallet unlock --keystore <file>` and the account's password. Keep this \
         vault backed up; it is gitignored and must never be committed.\n",
    );

    let out = dir.join("INDEX.md");
    std::fs::write(&out, md).with_context(|| format!("writing {}", out.display()))?;
    println!("wrote {} ({} accounts)", out.display(), rows.len());
    // Flag any account missing its keystore — that's the exact failure we're preventing.
    let missing: Vec<&String> = rows.iter().filter(|(_, _, k)| !k).map(|(a, _, _)| a).collect();
    if !missing.is_empty() {
        eprintln!("WARNING: these accounts have an identity file but NO recoverable keystore: {missing:?}");
    }
    Ok(0)
}

// ── Layer 3: optional encrypted-relay backup ────────────────────────────────
//
// These upload/fetch ONLY the encrypted keystore blob (the ciphertext already
// produced by Argon2id + AES-256-GCM). The password never leaves the device;
// the relay stores opaque bytes it cannot decrypt. Losing the local vault no
// longer means losing the account.
//
// Node contract (to be served by POST/GET /api/keystore/backup — the client is
// authoritative on the shape so the node route can be wired to match):
//   PUT  {node}/api/keystore/backup/{account}   body = keystore JSON (ciphertext)
//   GET  {node}/api/keystore/backup/{account}   -> keystore JSON

fn backup_url(node: &str, account: &str) -> String {
    format!("{}/api/keystore/backup/{}", node.trim_end_matches('/'), account)
}

/// Upload the encrypted keystore blob to a relay/node. Refuses to send anything
/// but a valid, sealed keystore (never a decrypted secret).
fn cmd_backup(args: &[String]) -> Result<i32> {
    let ks_file = flag(args, "--keystore").ok_or_else(|| anyhow!("--keystore FILE required"))?;
    let node = flag(args, "--node").ok_or_else(|| anyhow!("--node URL required"))?;

    // Load + sanity-check it IS an encrypted keystore (has ciphertext, argon2id).
    let ks = Keystore::load(Path::new(ks_file))?;
    if ks.crypto.kdf != "argon2id" || ks.crypto.ciphertext.is_empty() {
        bail!("refusing to back up: file is not a sealed argon2id keystore");
    }
    let body = serde_json::to_string(&ks)?;
    let client = reqwest::blocking::Client::new();
    let resp = client
        .put(backup_url(node, &ks.account))
        .header("content-type", "application/json")
        .body(body)
        .send()
        .with_context(|| format!("PUT keystore backup for '{}'", ks.account))?;
    if !resp.status().is_success() {
        bail!("backup failed: HTTP {}", resp.status());
    }
    println!(
        "backed up '{}' (ciphertext only — the relay cannot decrypt it) to {}",
        ks.account, node
    );
    Ok(0)
}

/// Decrypt a keystore and export a role's PRIVATE key as a node env value.
/// This is the piece that turns a sealed keystore into the `HONE_POSTING_KEY`
/// (64-hex ed25519 seed) a founder/clock node needs to sign.
///
/// Requires the password (HONE_WALLET_PASSWORD). By default it writes the env
/// file `<account>.node.env` (gitignored) rather than printing the key, so the
/// secret doesn't land in shell history/terminal scrollback. Pass --print to
/// echo it to stdout instead (use only when piping into a launch).
///
///   HONE_WALLET_PASSWORD=... hone wallet export-node-key \
///     --keystore shindevlin.keystore.json --role posting --out node.env
fn cmd_export_node_key(args: &[String]) -> Result<i32> {
    let ks_file = flag(args, "--keystore").ok_or_else(|| anyhow!("--keystore FILE required"))?;
    let role = flag(args, "--role").unwrap_or("posting");
    let password = require_password()?;

    let ks = Keystore::load(Path::new(ks_file))?;
    let phrase = ks.open(&password)?; // Argon2id+AES-GCM decrypt; errors on wrong pw/tamper

    // --mnemonic: reveal the recovery phrase itself (the master backup). Used when
    // exporting a full readable key record. Prints only the phrase.
    if args.iter().any(|a| a == "--mnemonic") {
        println!("{phrase}");
        return Ok(0);
    }

    let wallet = Wallet::from_phrase(&phrase, &ks.account)?;
    let kp = wallet
        .hone_role_keypair(role)
        .with_context(|| format!("deriving {role} key"))?;
    let priv_hex = kp.private_key_hex(); // 64-hex ed25519 seed
    let pub_hex = kp.public_key_hex();

    if args.iter().any(|a| a == "--print") {
        // Print only the key (for `export HONE_POSTING_KEY=$(... --print)`).
        println!("{priv_hex}");
        eprintln!("(exported {} {role} private key; public = {pub_hex})", ks.account);
        return Ok(0);
    }

    // Default: write a node env file, gitignored, not echoed.
    let out = flag(args, "--out")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(format!("{}.node.env", ks.account)));
    let body = format!(
        "# HONE node env for account '{}' — contains the {role} PRIVATE key.\n\
         # NEVER commit or share this file. Delete after the node has started.\n\
         HONE_ACCOUNT={}\n\
         HONE_POSTING_KEY={}\n",
        ks.account, ks.account, priv_hex
    );
    std::fs::write(&out, body).with_context(|| format!("writing {}", out.display()))?;
    println!(
        "wrote {} (HONE_ACCOUNT={}, HONE_POSTING_KEY set; {role} public = {}).\n\
         Source it into the node env, then delete it: `set -a; . {}; set +a`",
        out.display(),
        ks.account,
        pub_hex,
        out.display()
    );
    Ok(0)
}

/// FULL BACKUP EXPORT. Decrypt a keystore and write a complete, human-readable
/// key record: the mnemonic + all 6 HONE role keys (public + private) + the
/// external-chain keys (EVM / Solana / Bitcoin, public + private). This is the
/// "never get locked out again" file. Writes to --out (default <account>.keys.txt).
///
///   HONE_WALLET_PASSWORD=... hone wallet export-all \
///     --keystore shindevlin.keystore.json --out shindevlin.keys.txt
fn cmd_export_all(args: &[String]) -> Result<i32> {
    let ks_file = flag(args, "--keystore").ok_or_else(|| anyhow!("--keystore FILE required"))?;
    let password = require_password()?;

    let ks = Keystore::load(Path::new(ks_file))?;
    let phrase = ks.open(&password)?;
    let w = Wallet::from_phrase(&phrase, &ks.account)?;

    let mut s = String::new();
    s.push_str(&format!("# HONE full key record — account: {}\n", ks.account));
    s.push_str("# Generated by `hone wallet export-all`. KEEP PRIVATE — encrypt this file.\n");
    s.push_str("# The mnemonic alone regenerates EVERY key below (and any future chain).\n\n");

    s.push_str("## Recovery phrase (master backup)\n");
    s.push_str(&format!("mnemonic: {phrase}\n\n"));

    s.push_str("## HONE role keys (ed25519) — public / private\n");
    for role in ["owner", "active", "posting", "memo", "hide", "seek"] {
        match w.hone_role_keypair(role) {
            Ok(kp) => s.push_str(&format!(
                "{role:8} pub: {}\n{:8} prv: {}\n",
                kp.public_key_hex(),
                "",
                kp.private_key_hex()
            )),
            Err(e) => s.push_str(&format!("{role:8} <derive error: {e}>\n")),
        }
    }
    s.push('\n');

    s.push_str("## External chains — public (address) / private\n");
    // EVM
    match (w.evm_address(), w.evm_private_key_hex()) {
        (Ok(a), Ok(p)) => s.push_str(&format!("evm      address: {a}\nevm      private: {p}\n")),
        _ => s.push_str("evm      <derive error>\n"),
    }
    // Solana
    s.push_str(&format!(
        "solana   address: {}\nsolana   private: {}\n",
        w.solana_address(),
        w.solana_private_key_base58()
    ));
    // Bitcoin
    match (w.bitcoin_pubkey_hex(), w.bitcoin_private_key()) {
        (Ok(pk), Ok((hexk, wif))) => s.push_str(&format!(
            "bitcoin  pubkey:  {pk}\nbitcoin  private (hex): {hexk}\nbitcoin  private (WIF): {wif}\n"
        )),
        _ => s.push_str("bitcoin  <derive error>\n"),
    }

    let out = flag(args, "--out")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(format!("{}.keys.txt", ks.account)));
    std::fs::write(&out, s).with_context(|| format!("writing {}", out.display()))?;
    println!("wrote full key record for '{}' -> {}", ks.account, out.display());
    Ok(0)
}

/// PRE-LAUNCH SAFETY CHECK. Cross-check the drafted genesis.json against the
/// local wallet vault: every non-system genesis account must have a recoverable
/// keystore AND its identity-file posting pubkey must match the genesis entry.
/// A mismatch here means a chain launched with a key nobody can sign for — the
/// exact failure Genesis v2 exists to prevent, caught while it's still fixable.
///
/// Reads only public files (genesis.json + *.wallet.json). No password, no
/// secret. Exits non-zero if anything is wrong, so it can gate a launch script.
fn cmd_verify_vault(args: &[String]) -> Result<i32> {
    let vault = flag(args, "--vault").ok_or_else(|| anyhow!("--vault DIR required"))?;
    let genesis = flag(args, "--genesis").ok_or_else(|| anyhow!("--genesis FILE required"))?;

    let raw = std::fs::read_to_string(genesis)
        .with_context(|| format!("reading {genesis}"))?;
    let g: serde_json::Value = serde_json::from_str(&raw).context("parsing genesis.json")?;
    let accounts = g["accounts"]
        .as_object()
        .ok_or_else(|| anyhow!("genesis.json has no \"accounts\" object"))?;

    let dir = Path::new(vault);
    let mut problems: Vec<String> = Vec::new();
    let mut checked = 0usize;
    let mut system = 0usize;
    let mut required_count = 0usize;

    // Optional: every account in the required-accounts manifest MUST be present
    // in genesis.json. This closes the gap where an operated account is simply
    // left off the genesis list entirely (caught previously only by human eye).
    if let Some(req_file) = flag(args, "--require-accounts") {
        let req_raw = std::fs::read_to_string(req_file)
            .with_context(|| format!("reading required-accounts file {req_file}"))?;
        for line in req_raw.lines() {
            let name = line.trim();
            if name.is_empty() || name.starts_with('#') {
                continue;
            }
            required_count += 1;
            if !accounts.contains_key(name) {
                problems.push(format!(
                    "'{name}': REQUIRED account is MISSING from genesis.json entirely"
                ));
            }
        }
    }

    for (account, entry) in accounts {
        // System funds have no keystore by design.
        if account.starts_with("__") && account.ends_with("__") {
            system += 1;
            continue;
        }
        checked += 1;

        // The genesis posting pubkey for this account.
        let genesis_pk = entry["keys"]["posting"]
            .as_str()
            .or_else(|| entry["public_key"].as_str());
        let genesis_pk = match genesis_pk {
            Some(pk) => pk,
            None => {
                problems.push(format!("'{account}': genesis entry has no posting/public_key"));
                continue;
            }
        };

        // Must have a recoverable keystore.
        let ks_path = dir.join(format!("{account}.keystore.json"));
        if !ks_path.exists() {
            problems.push(format!("'{account}': NO recoverable keystore in vault ({})", ks_path.display()));
            continue;
        }

        // Identity file's posting pubkey must match the genesis entry.
        let id_path = dir.join(format!("{account}.wallet.json"));
        if !id_path.exists() {
            problems.push(format!("'{account}': keystore present but no identity file to verify pubkey"));
            continue;
        }
        let id_raw = std::fs::read_to_string(&id_path).unwrap_or_default();
        let id: serde_json::Value = serde_json::from_str(&id_raw).unwrap_or(serde_json::Value::Null);
        let vault_pk = id["hone_role_public_keys"]["posting"]
            .as_str()
            .or_else(|| id["hone_public_key_hex"].as_str())
            .unwrap_or("");
        if vault_pk != genesis_pk {
            problems.push(format!(
                "'{account}': genesis posting key {genesis_pk} does NOT match vault key {vault_pk}"
            ));
        }
    }

    println!(
        "verify-vault: {checked} operated accounts checked, {system} system funds skipped{}.",
        if required_count > 0 { format!(", {required_count} required accounts enforced") } else { String::new() }
    );
    if problems.is_empty() {
        println!("OK ✓ — every operated genesis account has a recoverable keystore with a matching key.");
        if required_count > 0 {
            println!("OK ✓ — every required account is present in genesis.");
        }
        println!("Safe to launch.");
        Ok(0)
    } else {
        eprintln!("\nFAIL — {} problem(s) that would launch an unrecoverable/forked account:", problems.len());
        for p in &problems {
            eprintln!("  ✗ {p}");
        }
        eprintln!("\nFix these before launching. A wrong key at genesis is permanent.");
        Ok(2)
    }
}

/// Fetch an encrypted keystore blob back from a relay into the local vault.
/// Does NOT decrypt — you still need the password to unlock it afterward.
fn cmd_restore(args: &[String]) -> Result<i32> {
    let account = flag(args, "--account").ok_or_else(|| anyhow!("--account NAME required"))?;
    let node = flag(args, "--node").ok_or_else(|| anyhow!("--node URL required"))?;
    let vault = flag(args, "--vault").ok_or_else(|| anyhow!("--vault DIR required"))?;

    let client = reqwest::blocking::Client::new();
    let resp = client
        .get(backup_url(node, account))
        .send()
        .with_context(|| format!("GET keystore backup for '{account}'"))?;
    if !resp.status().is_success() {
        bail!("restore failed: HTTP {} (no backup for '{account}'?)", resp.status());
    }
    let text = resp.text()?;
    // Validate it parses as a keystore before writing.
    let ks: Keystore = serde_json::from_str(&text).context("relay returned a non-keystore body")?;
    if ks.account != account {
        bail!("relay returned keystore for '{}', expected '{}'", ks.account, account);
    }
    let out = keystore_path(vault, account);
    if out.exists() {
        bail!("keystore already exists at {} — refusing to overwrite", out.display());
    }
    ks.save(&out)?;
    println!(
        "restored '{}' keystore to {}. Unlock it with:\n  \
         HONE_WALLET_PASSWORD=... hone wallet unlock --keystore {}",
        account,
        out.display(),
        out.display()
    );
    Ok(0)
}
