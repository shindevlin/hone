//! keycheck — SAFE key-location finder.
//!
//! Given a HONE mnemonic (via HONE_MNEMONIC env var), derive each role keypair
//! and print ONLY the PUBLIC keys, comparing them against an account's on-chain
//! keys. This answers "does this seed control account X" WITHOUT ever printing
//! a private key.
//!
//! It never writes anything and never emits secret material — only public keys
//! and match/no-match verdicts.
//!
//! Usage:
//!   HONE_MNEMONIC="word1 ... word12" \
//!     hone-keycheck --account bullship \
//!       --expect-active c9d08bea297d73e6df90868a5800c6342832d88a0db5986d01cf765ccba99a83
//!
//! You can pass any subset of --expect-<role> (owner/active/posting/memo/hide/seek);
//! each is compared to the derived public key for that role.

use anyhow::{anyhow, Result};
use hone_sdk::Wallet;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("keycheck: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let account = arg(&args, "--account").unwrap_or_else(|| "(unnamed)".into());

    let mnemonic = std::env::var("HONE_MNEMONIC")
        .map_err(|_| anyhow!("set HONE_MNEMONIC env var (the 12/24-word phrase). It is never printed."))?;
    let mnemonic = mnemonic.trim();

    // Restore the wallet from the phrase. account name is cosmetic here — the
    // keys derive from the seed regardless of name.
    let wallet = Wallet::from_phrase(mnemonic, &account)
        .map_err(|e| anyhow!("could not parse mnemonic: {e}"))?;

    println!("Derived PUBLIC keys for the given mnemonic (account label: {account}):\n");

    let roles = ["owner", "active", "posting", "memo", "hide", "seek"];
    let mut any_match = false;
    for role in roles {
        let kp = match wallet.hone_role_keypair(role) {
            Ok(kp) => kp,
            Err(e) => {
                println!("  {role:8} -> derive error: {e}");
                continue;
            }
        };
        let pub_hex = kp.public_key_hex();
        let expect = arg(&args, &format!("--expect-{role}"));
        match expect {
            Some(exp) if exp.trim() == pub_hex => {
                println!("  {role:8} -> {pub_hex}  ✓ MATCHES on-chain {role} key");
                any_match = true;
            }
            Some(exp) => {
                println!("  {role:8} -> {pub_hex}  ✗ (on-chain: {})", exp.trim());
            }
            None => {
                println!("  {role:8} -> {pub_hex}", );
            }
        }
    }

    println!();
    if any_match {
        println!("RESULT: this mnemonic controls at least one on-chain key for '{account}'.");
        println!("        The matching role's PRIVATE key can sign for that account.");
        println!("        (Private keys were NOT printed. Derive them yourself from this seed when needed.)");
    } else {
        println!("RESULT: no --expect-<role> matched. Either no expectations were passed,");
        println!("        or this seed does NOT control the given account's keys.");
    }
    Ok(())
}
