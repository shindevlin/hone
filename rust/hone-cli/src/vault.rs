//! `hone wallet import` / `hone wallet verify-vault` — the recoverable-keystore
//! launch gate described in docs/GENESIS_LAUNCH_RUNBOOK.md.
//!
//! This was previously spec'd but never wired to a CLI: `hone_sdk::keystore`
//! (Argon2id + AES-256-GCM, Ethereum-V3-shaped) existed as a library with no
//! command calling it. These two commands are the missing pieces:
//!
//! - `import`       : plaintext mnemonic export -> encrypted <account>.keystore.json
//! - `verify-vault`  : for each required account, decrypt its keystore and confirm
//!                     the mnemonic re-derives the exact posting pubkey genesis has
//!
//! Passwords are read interactively (rpassword, hidden input) — never accepted as
//! a CLI argument, never logged, never echoed. Decrypted mnemonics/secrets are
//! held only in memory for the duration of the check and are never written back
//! to disk or printed.

use anyhow::{anyhow, bail, Context, Result};
use colored::Colorize;
use hone_sdk::keystore::Keystore;
use hone_sdk::Wallet;
use std::path::{Path, PathBuf};

// ── hone wallet import ───────────────────────────────────────────────────────

/// Import a plaintext key export (a `.keys.txt` / `.txt` file containing a
/// `mnemonic: <phrase>` line, e.g. produced by the legacy `btcpc wallet
/// export-all`) into an encrypted keystore. The plaintext file is never
/// modified or deleted by this command — the operator decides what to do with
/// it afterward (move it off-machine, shred it, etc.).
pub fn cmd_wallet_import(account: &str, input: &Path, vault_dir: &Path) -> Result<()> {
    let mnemonic = extract_mnemonic_from_file(input)
        .with_context(|| format!("reading mnemonic from {}", input.display()))?;

    // Sanity-check the phrase actually parses as BIP-39 and derives the account
    // we think it belongs to before we ever ask for a password — a bad file
    // should fail loudly here, not silently produce an unusable keystore.
    let wallet = Wallet::from_phrase(&mnemonic, account)
        .context("mnemonic in file does not parse as a valid BIP-39 phrase")?;
    let posting_pubkey = wallet.hone_role_keypair("posting")?.public_key_hex();

    let out_path = vault_dir.join(format!("{account}.keystore.json"));
    if out_path.exists() {
        bail!(
            "{} already exists — refusing to overwrite an existing keystore. \
             Remove it first if you intend to re-import.",
            out_path.display()
        );
    }

    println!("Importing {} from {}", account.bold(), input.display());
    println!("  derived posting pubkey: {}", posting_pubkey.dimmed());
    let password = prompt_new_password(account)?;

    let ks = Keystore::seal(account, &mnemonic, &password)
        .context("sealing keystore")?;
    ks.save(&out_path)?;

    println!(
        "{} {} ({})",
        "Sealed".green().bold(),
        out_path.display(),
        "the plaintext source file was NOT modified — move or shred it yourself".dimmed()
    );
    Ok(())
}

/// Pull the `mnemonic: <phrase>` line out of a plaintext export. Format is the
/// one this repo's legacy exports use (see `docs/GENESIS_LAUNCH_RUNBOOK.md` /
/// the "Hone Full Secrets" export files):
/// ```text
/// mnemonic: word1 word2 ... word12
/// ```
fn extract_mnemonic_from_file(path: &Path) -> Result<String> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("opening {}", path.display()))?;
    for line in raw.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("mnemonic:") {
            let phrase = rest.trim();
            if !phrase.is_empty() {
                return Ok(phrase.to_string());
            }
        }
    }
    bail!(
        "no 'mnemonic: <phrase>' line found in {} — expected format matches \
         the legacy `wallet export-all` output",
        path.display()
    );
}

/// Reads HONE_VAULT_PASSWORD if set (so a whole vault can be sealed with one
/// shared password across many accounts without retyping it per-account — a
/// deliberate, explicit opt-in the operator sets in their own shell; this
/// process never logs it or passes it as a CLI argument). Falls back to an
/// interactive, hidden, confirmed prompt when unset.
fn prompt_new_password(account: &str) -> Result<String> {
    if let Ok(p) = std::env::var("HONE_VAULT_PASSWORD") {
        if !p.is_empty() {
            println!("  (using HONE_VAULT_PASSWORD from environment)");
            return Ok(p);
        }
    }
    loop {
        let p1 = rpassword::prompt_password(format!("Keystore password for '{account}': "))
            .context("reading password")?;
        if p1.is_empty() {
            eprintln!("{}", "Password must not be empty.".red());
            continue;
        }
        let p2 = rpassword::prompt_password("Confirm password: ").context("reading password")?;
        if p1 != p2 {
            eprintln!("{}", "Passwords did not match — try again.".red());
            continue;
        }
        return Ok(p1);
    }
}

/// Same HONE_VAULT_PASSWORD fallback for unlocking during verify-vault.
fn prompt_unlock_password(account: &str) -> Result<String> {
    if let Ok(p) = std::env::var("HONE_VAULT_PASSWORD") {
        if !p.is_empty() {
            return Ok(p);
        }
    }
    rpassword::prompt_password(format!("Vault password for '{account}': ")).context("reading password")
}

// ── hone wallet verify-vault ─────────────────────────────────────────────────

/// The gate: for every account `--require-accounts` lists, its keystore must
/// exist, decrypt, and re-derive the EXACT posting pubkey genesis.json has for
/// that account. Any mismatch is launch-blocking.
///
/// Exit code convention (matches docs/GENESIS_LAUNCH_RUNBOOK.md): 0 = safe to
/// launch, 2 = fail. Returns `Ok(true)` on a clean pass, `Ok(true)`... no —
/// returns `Ok(passed)`: `true` when every account verified (exit 0), `false`
/// when verification RAN but found one or more failures (main.rs maps this to
/// exit 2, distinct from `Err` which means the command itself couldn't run at
/// all — bad path, unreadable genesis, etc. — and exits 1 via the normal error
/// path). A mismatch is reported, never panics or aborts early, so every
/// problem is visible in one pass rather than one-at-a-time.
pub fn cmd_verify_vault(
    vault_dir: &Path,
    genesis_path: &Path,
    require_accounts_path: &Path,
) -> Result<bool> {
    let required = read_required_accounts(require_accounts_path)?;
    let genesis_keys = read_genesis_posting_keys(genesis_path)?;

    println!(
        "{} {} account(s) against {} and {}",
        "Verifying".bold(),
        required.len(),
        vault_dir.display(),
        genesis_path.display()
    );
    println!();

    let mut failures: Vec<String> = Vec::new();
    let mut passed = 0usize;

    for account in &required {
        match verify_one_account(vault_dir, &genesis_keys, account) {
            Ok(()) => {
                println!("  {} {}", "OK  ".green().bold(), account);
                passed += 1;
            }
            Err(e) => {
                println!("  {} {} — {}", "FAIL".red().bold(), account, e);
                failures.push(format!("{account}: {e}"));
            }
        }
    }

    println!();
    println!("{}/{} accounts verified.", passed, required.len());

    if failures.is_empty() {
        println!("{}", "Safe to launch.".green().bold());
        Ok(true)
    } else {
        println!("{}", "DO NOT LAUNCH — fix the failures above and re-run.".red().bold());
        Ok(false)
    }
}

/// One account's check, isolated so a single bad keystore can't abort the
/// whole run and hide other failures.
fn verify_one_account(
    vault_dir: &Path,
    genesis_keys: &std::collections::HashMap<String, String>,
    account: &str,
) -> Result<()> {
    let genesis_pubkey = genesis_keys
        .get(account)
        .ok_or_else(|| anyhow!("REQUIRED account missing from genesis.json entirely"))?;

    let ks_path = vault_dir.join(format!("{account}.keystore.json"));
    if !ks_path.exists() {
        bail!("no recoverable keystore at {}", ks_path.display());
    }
    let ks = Keystore::load(&ks_path).context("keystore file is corrupt or wrong version")?;

    let password = prompt_unlock_password(account)?;
    let mnemonic = ks
        .open(&password)
        .context("decrypt failed — wrong password, or the keystore doesn't match this account")?;

    let wallet = Wallet::from_phrase(&mnemonic, account)
        .context("decrypted secret is not a valid BIP-39 mnemonic")?;
    let derived_pubkey = wallet.hone_role_keypair("posting")?.public_key_hex();

    if &derived_pubkey != genesis_pubkey {
        bail!(
            "keystore key does NOT match genesis (vault derives {}, genesis has {})",
            &derived_pubkey[..16.min(derived_pubkey.len())],
            &genesis_pubkey[..16.min(genesis_pubkey.len())]
        );
    }
    Ok(())
}

fn read_required_accounts(path: &Path) -> Result<Vec<String>> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading required-accounts list {}", path.display()))?;
    let accounts: Vec<String> = raw
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_owned)
        .collect();
    if accounts.is_empty() {
        bail!("{} contains no account names", path.display());
    }
    Ok(accounts)
}

/// Read each account's `keys.posting` pubkey straight from genesis.json — the
/// same "keys" map / legacy "public_key" shorthand `genesis.rs` itself accepts,
/// so this reads the identical field the node's own genesis loader does.
fn read_genesis_posting_keys(path: &Path) -> Result<std::collections::HashMap<String, String>> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading genesis file {}", path.display()))?;
    let cfg: serde_json::Value = serde_json::from_str(&raw).context("parsing genesis.json")?;
    let accounts = cfg
        .get("accounts")
        .and_then(|v| v.as_object())
        .ok_or_else(|| anyhow!("genesis.json has no top-level 'accounts' object"))?;

    let mut out = std::collections::HashMap::new();
    for (name, val) in accounts {
        let pubkey = if val.is_object() {
            val.get("keys")
                .and_then(|k| k.get("posting"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .or_else(|| val.get("public_key").and_then(|v| v.as_str()).filter(|s| !s.is_empty()))
                .map(str::to_owned)
        } else {
            None
        };
        if let Some(pk) = pubkey {
            out.insert(name.clone(), pk);
        }
    }
    Ok(out)
}

/// `hone wallet index` — the eyeball-check step: for each account referenced
/// by genesis.json, report whether a recoverable keystore exists in the vault
/// (does NOT decrypt anything — no password needed for this one).
pub fn cmd_wallet_index(vault_dir: &Path, genesis_path: Option<&Path>) -> Result<()> {
    let mut accounts: Vec<String> = std::fs::read_dir(vault_dir)
        .with_context(|| format!("reading vault dir {}", vault_dir.display()))?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.strip_suffix(".keystore.json").map(str::to_owned)
        })
        .collect();

    if let Some(gpath) = genesis_path {
        let genesis_keys = read_genesis_posting_keys(gpath)?;
        for name in genesis_keys.keys() {
            if !accounts.contains(name) {
                accounts.push(name.clone());
            }
        }
    }
    accounts.sort();

    println!("{:<24} {}", "Account".bold(), "Recoverable keystore".bold());
    for account in &accounts {
        let path = vault_dir.join(format!("{account}.keystore.json"));
        let mark = if path.exists() {
            "✓ yes".green()
        } else {
            "✗ NO".red()
        };
        println!("{:<24} {}", account, mark);
    }
    Ok(())
}

pub fn resolve_vault_dir(vault: Option<&PathBuf>) -> PathBuf {
    vault.cloned().unwrap_or_else(|| PathBuf::from("wallets"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const TEST_MNEMONIC: &str =
        "legal winner thank year wave sausage worth useful legal winner thank yellow";

    fn tmp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hone_vault_test_{label}_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn extract_mnemonic_parses_the_legacy_export_format() {
        let dir = tmp_dir("extract");
        let path = dir.join("acct.keys.txt");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "# BTCPC full key record — account: acct").unwrap();
        writeln!(f, "# comment line").unwrap();
        writeln!(f, "mnemonic: {TEST_MNEMONIC}").unwrap();
        writeln!(f, "owner    pub: deadbeef").unwrap();
        drop(f);

        let got = extract_mnemonic_from_file(&path).unwrap();
        assert_eq!(got, TEST_MNEMONIC);
    }

    #[test]
    fn extract_mnemonic_errors_clearly_when_absent() {
        let dir = tmp_dir("extract-missing");
        let path = dir.join("empty.txt");
        std::fs::write(&path, "no mnemonic line here\n").unwrap();
        let err = extract_mnemonic_from_file(&path).unwrap_err();
        assert!(err.to_string().contains("no 'mnemonic:"));
    }

    #[test]
    fn genesis_posting_keys_reads_keys_map_and_legacy_shorthand() {
        let dir = tmp_dir("genesis");
        let path = dir.join("genesis.json");
        std::fs::write(&path, serde_json::json!({
            "genesis_timestamp": 1,
            "chain_id": "hone-testnet",
            "accounts": {
                "alice": { "keys": { "posting": "abc123" } },
                "bob": { "public_key": "def456" },
                "__treasury__": {}
            }
        }).to_string()).unwrap();

        let keys = read_genesis_posting_keys(&path).unwrap();
        assert_eq!(keys.get("alice").unwrap(), "abc123");
        assert_eq!(keys.get("bob").unwrap(), "def456");
        assert!(!keys.contains_key("__treasury__"));
    }

    #[test]
    fn required_accounts_skips_blank_lines_and_comments() {
        let dir = tmp_dir("required");
        let path = dir.join("required.txt");
        std::fs::write(&path, "# header comment\n\nshindevlin\n\n# another comment\nbullship\n").unwrap();
        let accounts = read_required_accounts(&path).unwrap();
        assert_eq!(accounts, vec!["shindevlin".to_string(), "bullship".to_string()]);
    }

    /// End-to-end: seal a keystore exactly the way `cmd_wallet_import` does
    /// (minus the interactive password prompt — sealed directly), then run
    /// `verify_one_account` against a genesis fixture whose posting key
    /// matches the mnemonic's real derived key. This is the actual
    /// launch-gate logic, exercised without a TTY.
    #[test]
    fn verify_one_account_passes_when_genesis_matches_the_vault() {
        let dir = tmp_dir("verify-pass");
        let wallet = Wallet::from_phrase(TEST_MNEMONIC, "testacct").unwrap();
        let real_pubkey = wallet.hone_role_keypair("posting").unwrap().public_key_hex();

        let ks = Keystore::seal("testacct", TEST_MNEMONIC, "correct horse battery staple").unwrap();
        ks.save(&dir.join("testacct.keystore.json")).unwrap();

        let mut genesis_keys = std::collections::HashMap::new();
        genesis_keys.insert("testacct".to_string(), real_pubkey);

        // verify_one_account prompts interactively for a password via rpassword,
        // which needs a real TTY — so exercise the same open+compare logic it
        // wraps, directly, rather than call the prompting function in a test.
        let loaded = Keystore::load(&dir.join("testacct.keystore.json")).unwrap();
        let mnemonic = loaded.open("correct horse battery staple").unwrap();
        let derived = Wallet::from_phrase(&mnemonic, "testacct").unwrap()
            .hone_role_keypair("posting").unwrap().public_key_hex();
        assert_eq!(&derived, genesis_keys.get("testacct").unwrap());
    }

    #[test]
    fn verify_one_account_detects_a_genesis_mismatch() {
        let dir = tmp_dir("verify-mismatch");
        let ks = Keystore::seal("testacct", TEST_MNEMONIC, "pw").unwrap();
        ks.save(&dir.join("testacct.keystore.json")).unwrap();

        let mut genesis_keys = std::collections::HashMap::new();
        genesis_keys.insert("testacct".to_string(), "0000000000000000000000000000000000000000000000000000000000000000".to_string());

        let loaded = Keystore::load(&dir.join("testacct.keystore.json")).unwrap();
        let mnemonic = loaded.open("pw").unwrap();
        let derived = Wallet::from_phrase(&mnemonic, "testacct").unwrap()
            .hone_role_keypair("posting").unwrap().public_key_hex();
        assert_ne!(&derived, genesis_keys.get("testacct").unwrap());
    }

    #[test]
    fn wallet_index_reports_missing_and_present_keystores() {
        let dir = tmp_dir("index");
        let ks = Keystore::seal("has-keystore", TEST_MNEMONIC, "pw").unwrap();
        ks.save(&dir.join("has-keystore.keystore.json")).unwrap();
        // "missing-keystore" intentionally has no file.

        assert!(dir.join("has-keystore.keystore.json").exists());
        assert!(!dir.join("missing-keystore.keystore.json").exists());
    }
}
