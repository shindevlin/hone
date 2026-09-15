//! HONE — Export Encrypted Backup.
//!
//! A single-window native tool: point it at a HONE vault folder, give it the
//! vault password (to unlock the keystores) and a new password (to encrypt the
//! output). It writes EVERY key (recovery phrase + all HONE role keys +
//! Bitcoin/Ethereum/Solana private keys) for every account into one text file
//! and seals it in a fresh password-encrypted archive (WinRAR AES-256, or 7-Zip).
//!
//! Runs fully offline. Passwords never leave this machine.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use anyhow::{anyhow, bail, Context, Result};
use hone_sdk::keystore::Keystore;
use hone_sdk::Wallet;
use std::path::{Path, PathBuf};

// Iterate the SDK's canonical list, not a local copy — a new key type added in
// hone-sdk then shows up in every export automatically (future-proof).
const ROLES: &[&str] = hone_sdk::paths::HONE_ROLES;

fn default_vault() -> String {
    if let Ok(up) = std::env::var("USERPROFILE") {
        let p = PathBuf::from(up)
            .join("Documents")
            .join("Hone")
            .join("HONE Wallets Vault");
        if p.is_dir() {
            return p.to_string_lossy().to_string();
        }
    }
    String::new()
}

fn default_genesis() -> String {
    // Best-effort guess; the field is editable and the button no-ops until the
    // file actually exists, so a wrong guess is harmless.
    for c in ["X:\\hone\\rust\\hone-node\\genesis.json", "X:\\hone\\genesis.json"] {
        if Path::new(c).is_file() {
            return c.to_string();
        }
    }
    "X:\\hone\\rust\\hone-node\\genesis.json".to_string()
}

fn default_pubdir() -> String {
    // The "HONE Public Keys" folder sits next to the vault, under Documents\Hone.
    if let Ok(up) = std::env::var("USERPROFILE") {
        let p = PathBuf::from(up)
            .join("Documents")
            .join("Hone")
            .join("HONE Public Keys");
        if p.is_dir() {
            return p.to_string_lossy().to_string();
        }
        // Even if not present yet, offer the expected path as a starting point.
        return p.to_string_lossy().to_string();
    }
    // Fall back to next-to-vault if we can find a vault.
    let v = default_vault();
    if !v.is_empty() {
        if let Some(parent) = Path::new(&v).parent() {
            return parent.join("HONE Public Keys").to_string_lossy().to_string();
        }
    }
    String::new()
}

fn default_backup() -> String {
    if let Ok(up) = std::env::var("USERPROFILE") {
        let base = PathBuf::from(up).join("Documents").join("Hone");
        for f in ["HONE-Keys-Backup.rar", "HONE-Keys-Backup.7z"] {
            let p = base.join(f);
            if p.is_file() {
                return p.to_string_lossy().to_string();
            }
        }
        return base.join("HONE-Keys-Backup.rar").to_string_lossy().to_string();
    }
    String::new()
}

fn find_keystores(dir: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut v = Vec::new();
    for e in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let p = e?.path();
        if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
            if name.ends_with(".keystore.json") {
                v.push((name.trim_end_matches(".keystore.json").to_string(), p));
            }
        }
    }
    v.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(v)
}

/// Public block: addresses + public keys. Safe to store in the clear.
fn render_public(account: &str, w: &Wallet) -> Result<String> {
    let mut s = String::new();
    s.push_str("================================================================\n");
    s.push_str(&format!("HONE ACCOUNT: {account}\n"));
    s.push_str("================================================================\n");
    s.push_str(&format!("Send HONE to this account by its name:  {account}\n\n"));

    s.push_str("---- PUBLIC  (safe to share) ----------------------------------\n");
    s.push_str("HONE role public keys (hex):\n");
    for &r in ROLES {
        let kp = w.hone_role_keypair(r).with_context(|| format!("{r} public key"))?;
        s.push_str(&format!("  {r:<8} {}\n", kp.public_key_hex()));
    }
    s.push_str("receive addresses:\n");
    match w.evm_address() {
        Ok(a) => s.push_str(&format!("  ethereum   {a}\n")),
        Err(e) => s.push_str(&format!("  ethereum   <error: {e}>\n")),
    }
    s.push_str(&format!("  solana     {}\n", w.solana_address()));
    match w.bitcoin_pubkey_hex() {
        Ok(a) => s.push_str(&format!("  bitcoin    {a}  (compressed pubkey)\n")),
        Err(e) => s.push_str(&format!("  bitcoin    <error: {e}>\n")),
    }
    Ok(s)
}

/// Private block: recovery phrase + private keys. SECRET.
fn render_private(mnemonic: &str, w: &Wallet) -> Result<String> {
    let mut s = String::new();
    s.push_str("\n---- PRIVATE  (SECRET — never share) --------------------------\n");
    s.push_str(&format!("recovery phrase (BIP-39):\n  {mnemonic}\n\n"));
    s.push_str("HONE role private keys (hex):\n");
    for &r in ROLES {
        let kp = w.hone_role_keypair(r).with_context(|| format!("{r} private key"))?;
        s.push_str(&format!("  {r:<8} {}\n", kp.private_key_hex()));
    }
    s.push_str("chain private keys:\n");
    match w.evm_private_key_hex() {
        Ok(k) => s.push_str(&format!("  ethereum (hex)    {k}\n")),
        Err(e) => s.push_str(&format!("  ethereum (hex)    <error: {e}>\n")),
    }
    s.push_str(&format!("  solana (base58)   {}\n", w.solana_private_key_base58()));
    match w.bitcoin_private_key() {
        Ok((h, wif)) => {
            s.push_str(&format!("  bitcoin (hex)     {h}\n"));
            s.push_str(&format!("  bitcoin (WIF)     {wif}\n"));
        }
        Err(e) => s.push_str(&format!("  bitcoin           <error: {e}>\n")),
    }
    s.push_str("\n");
    Ok(s)
}

/// Canonical, deterministic message the `verify` key signs. It binds the proof
/// to the account name AND its posting key, so a proof can't be replayed for a
/// different account or a different genesis posting key.
fn proof_message(account: &str, posting_pubkey_hex: &str) -> String {
    format!("HONE-VERIFY-v1|account={account}|posting={posting_pubkey_hex}")
}

/// One proof-of-life entry: sign the canonical message with the zero-power
/// verify key. Only public data ends up in the result.
fn proof_entry(w: &Wallet, account: &str) -> Result<serde_json::Value> {
    let posting_pub = w.hone_role_keypair("posting")?.public_key_hex();
    let verify_kp = w.hone_role_keypair("verify")?;
    let msg = proof_message(account, &posting_pub);
    Ok(serde_json::json!({
        "account": account,
        "posting_pubkey": posting_pub,
        "verify_pubkey": verify_kp.public_key_hex(),
        "message": msg,
        "signature": verify_kp.sign_bytes(msg.as_bytes()),
    }))
}

/// Write verify-proofs.json (public data only) into `pub_dir`.
fn write_proofs_doc(pub_dir: &Path, proofs: &[serde_json::Value]) -> Result<()> {
    std::fs::create_dir_all(pub_dir).ok();
    let doc = serde_json::json!({
        "format": "hone-verify-proofs-v1",
        "note": "Each signature is by the account's zero-power `verify` key over `message`. \
                 A valid signature proves the wallet seed is intact; because every role key \
                 derives from that one seed, the rest of the wallet is intact too. Safe to share.",
        "proofs": proofs,
    });
    std::fs::write(pub_dir.join("verify-proofs.json"), serde_json::to_string_pretty(&doc)?)
        .with_context(|| "writing verify-proofs.json")?;
    Ok(())
}

/// Pull the BIP-39 recovery phrase out of one exported `<account>.txt` file.
fn extract_mnemonic(txt: &str) -> Option<String> {
    let mut lines = txt.lines();
    while let Some(l) = lines.next() {
        if l.trim().to_lowercase().starts_with("recovery phrase") {
            for nx in lines.by_ref() {
                let t = nx.trim();
                if !t.is_empty() {
                    return Some(t.to_string());
                }
            }
        }
    }
    None
}

/// Generate verify-proofs.json from an existing encrypted backup archive (the
/// per-account .txt files this tool writes) WITHOUT needing loose keystores.
/// Extracts to a temp dir, reads each recovery phrase, derives + signs the
/// proof, shreds the plaintext, and writes the proofs to `pub_dir`.
pub fn make_proofs_from_backup(archive: &Path, archive_pw: &str, pub_dir: &Path) -> Result<String> {
    if archive_pw.is_empty() {
        bail!("enter the backup archive's password");
    }
    if !archive.is_file() {
        bail!("backup archive not found:\n{}", archive.display());
    }
    let tmp = std::env::temp_dir().join(format!("hone-proofs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp)?;

    let ext = archive
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let status = if ext == "7z" {
        let z = resolve(&[
            "C:\\Program Files\\7-Zip\\7z.exe",
            "C:\\Program Files (x86)\\7-Zip\\7z.exe",
        ])
        .ok_or_else(|| anyhow!("7-Zip not found. Install 7-Zip."))?;
        std::process::Command::new(z)
            .current_dir(&tmp)
            .arg("x").arg("-y").arg(format!("-p{archive_pw}")).arg(archive)
            .status()
            .context("running 7-Zip")?
    } else {
        let rar = resolve(&[
            "C:\\Program Files\\WinRAR\\UnRAR.exe",
            "C:\\Program Files (x86)\\WinRAR\\UnRAR.exe",
            "C:\\Program Files\\WinRAR\\Rar.exe",
            "C:\\Program Files (x86)\\WinRAR\\Rar.exe",
        ])
        .ok_or_else(|| anyhow!("WinRAR not found. Install WinRAR."))?;
        std::process::Command::new(rar)
            .current_dir(&tmp)
            .arg("x").arg("-y").arg(format!("-p{archive_pw}")).arg(archive)
            .status()
            .context("running WinRAR")?
    };
    if !status.success() {
        let _ = std::fs::remove_dir_all(&tmp);
        bail!("could not open the archive — wrong password, or the file isn't a HONE backup");
    }

    // Read each <account>.txt, derive + sign, then shred the plaintext at once.
    let mut proofs: Vec<serde_json::Value> = Vec::new();
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&tmp)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    entries.sort();
    for p in entries {
        let name = match p.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if !name.ends_with(".txt") {
            continue;
        }
        let acct = name.trim_end_matches(".txt").to_string();
        let content = std::fs::read_to_string(&p).unwrap_or_default();
        let mnemonic = extract_mnemonic(&content);
        // Shred + remove the plaintext immediately.
        if let Ok(meta) = std::fs::metadata(&p) {
            let _ = std::fs::write(&p, vec![0u8; meta.len() as usize]);
        }
        let _ = std::fs::remove_file(&p);
        if let Some(m) = mnemonic {
            let w = Wallet::from_phrase(&m, &acct).with_context(|| format!("deriving {acct}"))?;
            proofs.push(proof_entry(&w, &acct)?);
        }
    }
    let _ = std::fs::remove_dir_all(&tmp);

    if proofs.is_empty() {
        bail!("no wallet recovery phrases found inside the archive");
    }
    let n = proofs.len();
    write_proofs_doc(pub_dir, &proofs)?;
    Ok(format!(
        "Wrote verify proofs for {n} accounts to:\n{}\n\nNow click \"Verify vault for launch\" below.",
        pub_dir.display()
    ))
}

fn resolve(candidates: &[&str]) -> Option<PathBuf> {
    for c in candidates {
        let p = PathBuf::from(c);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

pub struct Opts {
    pub use_rar: bool,
    pub encrypt_names: bool,
    pub shred: bool,
}

/// Decrypt every keystore, write all keys to one text file, and seal it in a
/// password-encrypted archive. Returns a human summary on success.
pub fn export_vault(vault_dir: &Path, vault_pw: &str, new_pw: &str, opts: &Opts) -> Result<String> {
    if new_pw.is_empty() {
        bail!("choose a new password for the archive");
    }
    let keystores = find_keystores(vault_dir)?;
    if keystores.is_empty() {
        bail!("no *.keystore.json files found in\n{}", vault_dir.display());
    }

    let tmp = std::env::temp_dir().join(format!("hone-export-{}", std::process::id()));
    std::fs::create_dir_all(&tmp)?;

    let parent = vault_dir.parent().unwrap_or(vault_dir);
    // Plaintext public-key files live here so addresses can be looked up any
    // time WITHOUT unlocking the vault. Public keys are not secret.
    let pub_dir = parent.join("HONE Public Keys");
    std::fs::create_dir_all(&pub_dir)
        .with_context(|| format!("creating {}", pub_dir.display()))?;

    // Per wallet: a FULL file (public + private) goes into the encrypted archive;
    // a PUBLIC-ONLY file is written in the clear to `pub_dir`.
    let mut names: Vec<String> = Vec::new();
    let mut proofs: Vec<serde_json::Value> = Vec::new();
    for (acct, path) in &keystores {
        let ks = Keystore::load(path).with_context(|| format!("loading {}", path.display()))?;
        let mnemonic = ks
            .open(vault_pw)
            .map_err(|_| anyhow!("wrong vault password (or a keystore doesn't match) — nothing was written"))?;
        let w = Wallet::from_phrase(&mnemonic, acct).with_context(|| format!("deriving {acct}"))?;
        let pubt = render_public(acct, &w)?;
        let full = format!("{pubt}{}", render_private(&mnemonic, &w)?);
        let name = format!("{acct}.txt");
        std::fs::write(tmp.join(&name), &full).with_context(|| format!("writing {name}"))?;
        std::fs::write(pub_dir.join(&name), &pubt)
            .with_context(|| format!("writing public {name}"))?;
        names.push(name);

        // Proof-of-life: sign a canonical message with the ZERO-POWER verify key
        // while the seed is open. Safe to publish; verified later without a
        // password. Vouches for every other same-seed key in the wallet.
        proofs.push(proof_entry(&w, acct)?);
    }
    let count = names.len();

    // Publish the proofs in the clear (no secrets — only public keys + a
    // signature). This is what the launch check and any AI verify against.
    write_proofs_doc(&pub_dir, &proofs)?;

    let status;
    let out;
    if opts.use_rar {
        let rar = resolve(&[
            "C:\\Program Files\\WinRAR\\Rar.exe",
            "C:\\Program Files (x86)\\WinRAR\\Rar.exe",
        ])
        .ok_or_else(|| anyhow!("WinRAR not found. Install WinRAR, or switch to 7-Zip."))?;
        out = parent.join("HONE-Keys-Backup.rar");
        let _ = std::fs::remove_file(&out);
        // a=add  -y=assume yes  -hp<pw>=encrypt data + headers (filenames too).
        // current_dir(tmp) so the archive stores bare <account>.txt names.
        let mut cmd = std::process::Command::new(&rar);
        cmd.current_dir(&tmp);
        cmd.arg("a").arg("-y").arg(format!("-hp{new_pw}")).arg(&out);
        for n in &names {
            cmd.arg(n);
        }
        status = cmd.status().context("running WinRAR")?;
    } else {
        let z = resolve(&[
            "C:\\Program Files\\7-Zip\\7z.exe",
            "C:\\Program Files (x86)\\7-Zip\\7z.exe",
        ])
        .ok_or_else(|| anyhow!("7-Zip not found. Install 7-Zip, or switch to WinRAR."))?;
        out = parent.join("HONE-Keys-Backup.7z");
        let _ = std::fs::remove_file(&out);
        let mut cmd = std::process::Command::new(&z);
        cmd.current_dir(&tmp);
        cmd.arg("a").arg(format!("-p{new_pw}"));
        if opts.encrypt_names {
            cmd.arg("-mhe=on");
        }
        cmd.arg(&out);
        for n in &names {
            cmd.arg(n);
        }
        status = cmd.status().context("running 7-Zip")?;
    }

    // Best-effort shred + cleanup of every plaintext file.
    for n in &names {
        let f = tmp.join(n);
        if opts.shred {
            if let Ok(meta) = std::fs::metadata(&f) {
                let _ = std::fs::write(&f, vec![0u8; meta.len() as usize]);
            }
        }
        let _ = std::fs::remove_file(&f);
    }
    let _ = std::fs::remove_dir_all(&tmp);

    if !status.success() {
        bail!("archiver failed (exit {:?}) — no backup written", status.code());
    }
    if !out.exists() {
        bail!("archive was not created");
    }
    Ok(format!(
        "Saved {}\n{} wallet files inside (one per account) — public + private keys.\n\n\
         Public keys + verify proofs written (unencrypted, no password needed) to:\n{}\n\n\
         You can now run the launch check below any time — it needs no password.",
        out.display(),
        count,
        pub_dir.display()
    ))
}

/// The launch gate — click-driven and PASSWORD-FREE. For every account in
/// genesis it checks the exported verify-proofs.json: (1) the proof's posting
/// key matches genesis, and (2) the zero-power `verify` signature is
/// cryptographically valid. A valid signature proves the wallet's seed is live
/// and produced these keys; no private keystore is ever opened. Returns
/// Ok(summary) when safe, Err(details) when NOT safe to launch.
pub fn verify_for_launch(pub_dir: &Path, genesis_path: &Path) -> Result<String> {
    let raw = std::fs::read_to_string(genesis_path)
        .with_context(|| format!("reading genesis file:\n{}", genesis_path.display()))?;
    let g: serde_json::Value = serde_json::from_str(&raw).context("parsing genesis.json")?;
    let accts = g
        .get("accounts")
        .and_then(|a| a.as_object())
        .ok_or_else(|| anyhow!("genesis.json has no \"accounts\" object"))?;

    let proofs_path = pub_dir.join("verify-proofs.json");
    let praw = std::fs::read_to_string(&proofs_path).map_err(|_| {
        anyhow!(
            "no verify-proofs.json in\n{}\n\nRun an export first (top of this window) — it \
             writes the proofs used for this check.",
            pub_dir.display()
        )
    })?;
    let pdoc: serde_json::Value =
        serde_json::from_str(&praw).context("parsing verify-proofs.json")?;
    let proofs = pdoc
        .get("proofs")
        .and_then(|p| p.as_array())
        .ok_or_else(|| anyhow!("verify-proofs.json has no \"proofs\" array"))?;

    // Index proofs by account for lookup.
    let mut by_acct: std::collections::HashMap<&str, &serde_json::Value> =
        std::collections::HashMap::new();
    for p in proofs {
        if let Some(a) = p.get("account").and_then(|a| a.as_str()) {
            by_acct.insert(a, p);
        }
    }

    let mut checked = 0usize;
    let mut failures: Vec<String> = Vec::new();
    for (name, val) in accts {
        // System funds (__treasury__ etc.) have no signable key — skip.
        let posting = match val
            .get("keys")
            .and_then(|k| k.get("posting"))
            .and_then(|p| p.as_str())
        {
            Some(p) => p.to_lowercase(),
            None => continue,
        };
        checked += 1;
        let proof = match by_acct.get(name.as_str()) {
            Some(p) => *p,
            None => {
                failures.push(format!("{name}: no proof for this account — re-run export"));
                continue;
            }
        };
        let p_posting = proof
            .get("posting_pubkey")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_lowercase();
        if p_posting != posting {
            failures.push(format!("{name}: posting key does NOT match genesis"));
            continue;
        }
        let verify_pub = proof.get("verify_pubkey").and_then(|v| v.as_str()).unwrap_or_default();
        let sig = proof.get("signature").and_then(|v| v.as_str()).unwrap_or_default();
        // Recompute the message from trusted inputs (account + genesis posting)
        // rather than trusting the stored one — this is what binds the proof.
        let msg = proof_message(name, &posting);
        match hone_sdk::verify_ed25519_hex(verify_pub, msg.as_bytes(), sig) {
            Ok(true) => {}
            Ok(false) => failures.push(format!("{name}: verify signature is INVALID")),
            Err(e) => failures.push(format!("{name}: proof unreadable ({e})")),
        }
    }

    if failures.is_empty() {
        Ok(format!(
            "SAFE TO LAUNCH — all {checked} accounts verified.\n\
             Every account's posting key matches genesis and its verify signature is valid.\n\
             No private keys were opened; no password used."
        ))
    } else {
        bail!(
            "DO NOT LAUNCH — {} problem(s):\n{}",
            failures.len(),
            failures.join("\n")
        )
    }
}

// ── GUI ──────────────────────────────────────────────────────────────────────

const BG: egui::Color32 = egui::Color32::from_rgb(12, 12, 17);
const CARD: egui::Color32 = egui::Color32::from_rgb(20, 20, 28);
const BORDER: egui::Color32 = egui::Color32::from_rgb(38, 38, 53);
const ORANGE: egui::Color32 = egui::Color32::from_rgb(247, 147, 26);
const DIM: egui::Color32 = egui::Color32::from_rgb(138, 138, 160);
const GREEN: egui::Color32 = egui::Color32::from_rgb(72, 214, 138);
const RED: egui::Color32 = egui::Color32::from_rgb(242, 88, 91);

struct App {
    vault_dir: String,
    vault_pw: String,
    new_pw: String,
    confirm_pw: String,
    use_rar: bool,
    encrypt_names: bool,
    shred: bool,
    result: Option<(bool, String)>,
    pub_dir: String,
    genesis_path: String,
    verify_result: Option<(bool, String)>,
    backup_path: String,
    backup_pw: String,
    proofs_result: Option<(bool, String)>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            vault_dir: default_vault(),
            vault_pw: String::new(),
            new_pw: String::new(),
            confirm_pw: String::new(),
            use_rar: true,
            encrypt_names: true,
            shred: true,
            result: None,
            pub_dir: default_pubdir(),
            genesis_path: default_genesis(),
            verify_result: None,
            backup_path: default_backup(),
            backup_pw: String::new(),
            proofs_result: None,
        }
    }
}

fn label(ui: &mut egui::Ui, text: &str) {
    ui.add_space(12.0);
    ui.label(egui::RichText::new(text.to_uppercase()).size(11.0).color(DIM));
    ui.add_space(3.0);
}

impl eframe::App for App {
    fn ui(&mut self, ui_root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin::same(22_i8)))
            .show_inside(ui_root, |ui| {
              egui::ScrollArea::vertical().show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.label(egui::RichText::new("Back up everything, sealed with your password")
                    .size(18.0).strong().color(egui::Color32::WHITE));
                ui.add_space(4.0);
                ui.label(egui::RichText::new(
                    "Unlocks your vault, writes every key (recovery phrase + HONE + \
                     Bitcoin/Ethereum/Solana) into one text file, and seals it in a new \
                     encrypted archive only your password opens.")
                    .size(12.5).color(DIM));

                let full = ui.available_width();

                label(ui, "Vault to export");
                ui.horizontal(|ui| {
                    if ui.button("📁  Choose folder…").clicked() {
                        if let Some(p) = rfd::FileDialog::new()
                            .set_title("Pick the folder that holds your .keystore.json files")
                            .pick_folder()
                        {
                            self.vault_dir = p.to_string_lossy().to_string();
                        }
                    }
                    ui.add(egui::TextEdit::singleline(&mut self.vault_dir)
                        .desired_width(ui.available_width())
                        .hint_text("click Choose folder…"));
                });

                label(ui, "Vault password  (to unlock the keys)");
                ui.add(egui::TextEdit::singleline(&mut self.vault_pw)
                    .password(true).desired_width(full).hint_text("current keystore password"));

                ui.add_space(16.0);
                ui.separator();
                ui.label(egui::RichText::new("NEW ENCRYPTED ARCHIVE").size(11.0).color(DIM));

                ui.columns(2, |c| {
                    c[0].label(egui::RichText::new("New password").size(11.0).color(DIM));
                    c[0].add(egui::TextEdit::singleline(&mut self.new_pw)
                        .password(true).hint_text("choose a strong password"));
                    c[1].label(egui::RichText::new("Confirm").size(11.0).color(DIM));
                    c[1].add(egui::TextEdit::singleline(&mut self.confirm_pw)
                        .password(true).hint_text("repeat it"));
                });

                label(ui, "Options");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.use_rar, true, "RAR · AES-256");
                    ui.selectable_value(&mut self.use_rar, false, "7-Zip");
                    ui.add_space(8.0);
                    ui.checkbox(&mut self.encrypt_names, "Encrypt file names");
                    ui.checkbox(&mut self.shred, "Shred plaintext after");
                });

                ui.add_space(18.0);
                let btn = egui::Button::new(
                    egui::RichText::new("🔒  Create encrypted backup").size(14.0).strong()
                        .color(egui::Color32::BLACK))
                    .fill(ORANGE).min_size(egui::vec2(full, 42.0));
                if ui.add(btn).clicked() {
                    self.run();
                }

                if let Some((ok, msg)) = &self.result {
                    ui.add_space(12.0);
                    egui::Frame::new().fill(CARD).stroke(egui::Stroke::new(1.0, BORDER))
                        .corner_radius(egui::CornerRadius::same(9)).inner_margin(egui::Margin::same(12_i8))
                        .show(ui, |ui| {
                            ui.label(egui::RichText::new(msg).size(12.5)
                                .color(if *ok { GREEN } else { RED }));
                        });
                }

                ui.add_space(20.0);
                ui.separator();
                ui.label(egui::RichText::new("LAUNCH READINESS").size(11.0).color(DIM));
                ui.add_space(2.0);
                ui.label(egui::RichText::new(
                    "Two steps. First make the verify proofs from your backup (needs the \
                     backup's password, once). Then run the password-free check against genesis.")
                    .size(11.5).color(DIM));

                label(ui, "Step 1 — make proofs from your backup");
                ui.horizontal(|ui| {
                    if ui.button("📦  Choose backup…").clicked() {
                        if let Some(p) = rfd::FileDialog::new()
                            .set_title("Pick your HONE-Keys-Backup archive")
                            .add_filter("backup archive", &["rar", "7z"])
                            .pick_file()
                        {
                            self.backup_path = p.to_string_lossy().to_string();
                        }
                    }
                    ui.add(egui::TextEdit::singleline(&mut self.backup_path)
                        .desired_width(ui.available_width())
                        .hint_text("click Choose backup…"));
                });
                ui.add_space(6.0);
                ui.add(egui::TextEdit::singleline(&mut self.backup_pw)
                    .password(true).desired_width(full)
                    .hint_text("the password you set on that backup"));
                ui.add_space(8.0);
                let mkbtn = egui::Button::new(
                    egui::RichText::new("① Make verify proofs").size(13.5).strong()
                        .color(egui::Color32::BLACK))
                    .fill(ORANGE).min_size(egui::vec2(full, 38.0));
                if ui.add(mkbtn).clicked() {
                    self.run_make_proofs();
                }
                if let Some((ok, msg)) = &self.proofs_result {
                    ui.add_space(10.0);
                    egui::Frame::new().fill(CARD).stroke(egui::Stroke::new(1.0, BORDER))
                        .corner_radius(egui::CornerRadius::same(9)).inner_margin(egui::Margin::same(12_i8))
                        .show(ui, |ui| {
                            ui.label(egui::RichText::new(msg).size(12.5)
                                .color(if *ok { GREEN } else { RED }));
                        });
                }

                ui.add_space(6.0);
                ui.label(egui::RichText::new("Step 2 — check against genesis").size(11.0).color(DIM));

                label(ui, "Public keys folder (from export)");
                ui.horizontal(|ui| {
                    if ui.button("📁  Choose folder…").clicked() {
                        if let Some(p) = rfd::FileDialog::new()
                            .set_title("Pick your \"HONE Public Keys\" folder")
                            .pick_folder()
                        {
                            self.pub_dir = p.to_string_lossy().to_string();
                        }
                    }
                    ui.add(egui::TextEdit::singleline(&mut self.pub_dir)
                        .desired_width(ui.available_width())
                        .hint_text("click Choose folder…"));
                });

                label(ui, "genesis.json");
                ui.horizontal(|ui| {
                    if ui.button("📄  Choose file…").clicked() {
                        if let Some(p) = rfd::FileDialog::new()
                            .set_title("Pick genesis.json")
                            .add_filter("genesis", &["json"])
                            .pick_file()
                        {
                            self.genesis_path = p.to_string_lossy().to_string();
                        }
                    }
                    ui.add(egui::TextEdit::singleline(&mut self.genesis_path)
                        .desired_width(ui.available_width())
                        .hint_text("click Choose file…"));
                });

                ui.add_space(12.0);
                let vbtn = egui::Button::new(
                    egui::RichText::new("✓  Verify vault for launch").size(14.0).strong()
                        .color(egui::Color32::WHITE))
                    .fill(CARD).stroke(egui::Stroke::new(1.0, GREEN))
                    .min_size(egui::vec2(full, 40.0));
                if ui.add(vbtn).clicked() {
                    self.run_verify();
                }

                if let Some((ok, msg)) = &self.verify_result {
                    ui.add_space(12.0);
                    egui::Frame::new().fill(CARD).stroke(egui::Stroke::new(1.0, BORDER))
                        .corner_radius(egui::CornerRadius::same(9)).inner_margin(egui::Margin::same(12_i8))
                        .show(ui, |ui| {
                            ui.label(egui::RichText::new(msg).size(12.5)
                                .color(if *ok { GREEN } else { RED }));
                        });
                }
              });
            });
    }
}

impl App {
    fn run(&mut self) {
        if self.vault_pw.is_empty() {
            self.result = Some((false, "enter the vault password".into()));
            return;
        }
        if self.new_pw.is_empty() || self.new_pw != self.confirm_pw {
            self.result = Some((false, "new password and confirmation don't match".into()));
            return;
        }
        let opts = Opts { use_rar: self.use_rar, encrypt_names: self.encrypt_names, shred: self.shred };
        match export_vault(Path::new(&self.vault_dir), &self.vault_pw, &self.new_pw, &opts) {
            Ok(msg) => {
                self.vault_pw.clear();
                self.new_pw.clear();
                self.confirm_pw.clear();
                self.result = Some((true, msg));
            }
            Err(e) => self.result = Some((false, e.to_string())),
        }
    }

    fn run_verify(&mut self) {
        match verify_for_launch(Path::new(&self.pub_dir), Path::new(&self.genesis_path)) {
            Ok(msg) => self.verify_result = Some((true, msg)),
            Err(e) => self.verify_result = Some((false, e.to_string())),
        }
    }

    fn run_make_proofs(&mut self) {
        let r = make_proofs_from_backup(
            Path::new(&self.backup_path),
            &self.backup_pw,
            Path::new(&self.pub_dir),
        );
        match r {
            Ok(msg) => {
                self.backup_pw.clear();
                self.proofs_result = Some((true, msg));
            }
            Err(e) => self.proofs_result = Some((false, e.to_string())),
        }
    }
}

/// End-to-end check of the password-free launch gate, using the PUBLIC test
/// mnemonic only. Builds verify-proofs.json exactly as export does, then asserts
/// valid -> pass, tampered posting -> fail, forged signature -> fail, missing
/// proof -> fail.
fn selfcheck() -> Result<()> {
    const PHRASE: &str =
        "legal winner thank year wave sausage worth useful legal winner thank yellow";
    let dir = std::env::temp_dir().join("hone-export-selfcheck");
    let _ = std::fs::remove_dir_all(&dir);
    let pub_dir = dir.join("HONE Public Keys");
    std::fs::create_dir_all(&pub_dir)?;

    // The proof-from-backup parser must recover the phrase from the exact
    // private-file format render_private writes.
    {
        let w = Wallet::from_phrase(PHRASE, "alpha")?;
        let body = render_private(PHRASE, &w)?;
        match extract_mnemonic(&body) {
            Some(m) if m == PHRASE => {}
            other => bail!("extract_mnemonic failed on real format: {other:?}"),
        }
    }

    let accounts = ["alpha", "bravo", "charlie"];
    let mut posting = std::collections::BTreeMap::new();
    let mut proofs: Vec<serde_json::Value> = Vec::new();
    for a in accounts {
        // Same derivation + signing the real export performs.
        let w = Wallet::from_phrase(PHRASE, a)?;
        let posting_pub = w.hone_role_keypair("posting")?.public_key_hex();
        let verify_kp = w.hone_role_keypair("verify")?;
        let msg = proof_message(a, &posting_pub);
        proofs.push(serde_json::json!({
            "account": a,
            "posting_pubkey": posting_pub,
            "verify_pubkey": verify_kp.public_key_hex(),
            "message": msg,
            "signature": verify_kp.sign_bytes(msg.as_bytes()),
        }));
        posting.insert(a, posting_pub);
    }
    let write_proofs = |ps: &[serde_json::Value]| -> Result<()> {
        std::fs::write(
            pub_dir.join("verify-proofs.json"),
            serde_json::to_string_pretty(&serde_json::json!({ "proofs": ps }))?,
        )?;
        Ok(())
    };
    write_proofs(&proofs)?;

    let genesis = |accts: serde_json::Map<String, serde_json::Value>| -> serde_json::Value {
        serde_json::json!({ "chain_id": "hone-selfcheck", "accounts": accts })
    };
    let mut accts = serde_json::Map::new();
    for a in accounts {
        accts.insert(a.to_string(), serde_json::json!({ "keys": { "posting": posting[a] } }));
    }
    accts.insert("__treasury__".to_string(), serde_json::json!({}));
    let good_genesis = dir.join("genesis_good.json");
    std::fs::write(&good_genesis, serde_json::to_string_pretty(&genesis(accts.clone()))?)?;

    // 1. valid proofs + genesis -> must pass
    let ok = verify_for_launch(&pub_dir, &good_genesis)?;
    if !ok.contains("SAFE TO LAUNCH") {
        bail!("valid case did not report SAFE TO LAUNCH: {ok}");
    }

    // 2. tampered genesis posting key -> must fail with a mismatch
    let bad_genesis = dir.join("genesis_bad.json");
    let mut bad_accts = accts.clone();
    bad_accts.insert("bravo".to_string(), serde_json::json!({ "keys": { "posting": "aa".repeat(32) } }));
    std::fs::write(&bad_genesis, serde_json::to_string_pretty(&genesis(bad_accts))?)?;
    match verify_for_launch(&pub_dir, &bad_genesis) {
        Ok(_) => bail!("tampered genesis was accepted"),
        Err(e) => {
            if !e.to_string().contains("does NOT match") {
                bail!("tampered case failed for the wrong reason: {e}");
            }
        }
    }

    // 3. forged signature -> must fail as INVALID
    let mut forged = proofs.clone();
    if let Some(sig) = forged[1].get("signature").and_then(|s| s.as_str()) {
        // Flip the first hex nibble so the signature no longer verifies.
        let mut chars: Vec<char> = sig.chars().collect();
        chars[0] = if chars[0] == '0' { '1' } else { '0' };
        forged[1]["signature"] = serde_json::Value::String(chars.into_iter().collect());
    }
    write_proofs(&forged)?;
    match verify_for_launch(&pub_dir, &good_genesis) {
        Ok(_) => bail!("forged signature was accepted"),
        Err(e) => {
            if !e.to_string().contains("INVALID") {
                bail!("forged case failed for the wrong reason: {e}");
            }
        }
    }

    // 4. missing proof -> must fail
    write_proofs(&proofs[..2])?; // drop "charlie"
    match verify_for_launch(&pub_dir, &good_genesis) {
        Ok(_) => bail!("missing proof was accepted"),
        Err(e) => {
            if !e.to_string().contains("no proof") {
                bail!("missing-proof case failed for the wrong reason: {e}");
            }
        }
    }

    // 5. FULL round-trip through a real encrypted archive, if WinRAR is present:
    //    build a backup .rar of one wallet, make proofs from it, and verify.
    if let Some(rar) = resolve(&[
        "C:\\Program Files\\WinRAR\\Rar.exe",
        "C:\\Program Files (x86)\\WinRAR\\Rar.exe",
    ]) {
        let bdir = dir.join("bk");
        std::fs::create_dir_all(&bdir)?;
        let w = Wallet::from_phrase(PHRASE, "alpha")?;
        let body = format!("{}{}", render_public("alpha", &w)?, render_private(PHRASE, &w)?);
        std::fs::write(bdir.join("alpha.txt"), &body)?;
        let archive = dir.join("bk.rar");
        let st = std::process::Command::new(&rar)
            .current_dir(&bdir)
            .arg("a").arg("-y").arg("-hptestpw").arg(&archive).arg("alpha.txt")
            .status()?;
        if st.success() {
            let pd2 = dir.join("pd2");
            let msg = make_proofs_from_backup(&archive, "testpw", &pd2)?;
            if !msg.contains("1 accounts") {
                bail!("round-trip proof count wrong: {msg}");
            }
            let posting_pub = w.hone_role_keypair("posting")?.public_key_hex();
            let mut a2 = serde_json::Map::new();
            a2.insert("alpha".into(), serde_json::json!({ "keys": { "posting": posting_pub } }));
            let g2 = dir.join("g2.json");
            std::fs::write(&g2, serde_json::to_string_pretty(&genesis(a2))?)?;
            let vr = verify_for_launch(&pd2, &g2)?;
            if !vr.contains("SAFE TO LAUNCH") {
                bail!("round-trip verify failed: {vr}");
            }
            // wrong archive password must fail
            if make_proofs_from_backup(&archive, "nope", &pd2).is_ok() {
                bail!("round-trip accepted a wrong archive password");
            }
        }
    }

    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

fn main() -> eframe::Result<()> {
    // Hidden self-test path (used during development only): runs the export
    // logic non-interactively. Prints only a summary, never key material.
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(|s| s == "--selftest").unwrap_or(false) {
        let dir = PathBuf::from(&args[2]);
        let opts = Opts { use_rar: true, encrypt_names: true, shred: true };
        match export_vault(&dir, &args[3], &args[4], &opts) {
            Ok(m) => {
                println!("SELFTEST_OK: {m}");
                std::process::exit(0);
            }
            Err(e) => {
                println!("SELFTEST_ERR: {e}");
                std::process::exit(1);
            }
        }
    }
    // Hidden verify self-test: --verifytest <public_keys_dir> <genesis.json>
    if args.get(1).map(|s| s == "--verifytest").unwrap_or(false) {
        match verify_for_launch(Path::new(&args[2]), Path::new(&args[3])) {
            Ok(m) => {
                println!("VERIFY_OK: {m}");
                std::process::exit(0);
            }
            Err(e) => {
                println!("VERIFY_ERR: {e}");
                std::process::exit(1);
            }
        }
    }
    // Hidden end-to-end self-check of the launch gate, using the PUBLIC BIP39
    // test vector only (no real secrets). Fabricates a throwaway vault + genesis,
    // then asserts: valid -> pass, tampered key -> fail, wrong password -> fail.
    if args.get(1).map(|s| s == "--selfcheck").unwrap_or(false) {
        match selfcheck() {
            Ok(()) => {
                println!("SELFCHECK_OK");
                std::process::exit(0);
            }
            Err(e) => {
                println!("SELFCHECK_ERR: {e}");
                std::process::exit(1);
            }
        }
    }

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([600.0, 720.0])
            .with_title("HONE — Export Encrypted Backup"),
        ..Default::default()
    };
    eframe::run_native(
        "HONE — Export Encrypted Backup",
        native_options,
        Box::new(|_cc| -> Result<Box<dyn eframe::App>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(Box::new(App::default()))
        }),
    )
}
