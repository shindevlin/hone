//! Process-level checks: the gate stops a real binary that has no terminal
//! and no explicit acceptance, and lets it run with the right variable.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "hone-license-cli-{tag}-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run(dir: &PathBuf, env: Option<&str>, args: &[&str]) -> std::process::Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_hone-license"));
    c.args(args)
        .env("HONE_DATA_DIR", dir)
        .env_remove("HONE_ACCEPT_LICENSE")
        .stdin(Stdio::null());
    if let Some(v) = env {
        c.env("HONE_ACCEPT_LICENSE", v);
    }
    c.output().unwrap()
}

#[test]
fn non_tty_without_env_exits_nonzero() {
    let d = tmpdir("none");
    let o = run(&d, None, &["accept"]);
    assert_eq!(o.status.code(), Some(hone_license::EXIT_NOT_ACCEPTED));
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("HONE_ACCEPT_LICENSE=UFL-3.4-D"), "{err}");
    assert!(!d.join(hone_license::RECORD_FILE).exists());
}

#[test]
fn env_accepts_then_record_alone_suffices() {
    let d = tmpdir("env");
    let o = run(&d, Some("UFL-3.4-D"), &["accept"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(d.join(hone_license::RECORD_FILE).exists());
    let o = run(&d, None, &["accept"]);
    assert!(o.status.success());
    assert!(run(&d, None, &["status"]).status.success());
}

#[test]
fn wrong_env_exits_nonzero() {
    let d = tmpdir("bad");
    let o = run(&d, Some("yes"), &["accept"]);
    assert_eq!(o.status.code(), Some(hone_license::EXIT_NOT_ACCEPTED));
    assert!(!d.join(hone_license::RECORD_FILE).exists());
}

#[test]
fn flag_accepts_and_is_stripped() {
    let d = tmpdir("flag");
    let o = run(&d, None, &["accept", "--accept-ufl", "3.4"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    // The flag never reaches the command parser.
    let o = run(&d, None, &["--accept-ufl=3.4", "statement"]);
    assert!(String::from_utf8_lossy(&o.stdout).contains("UFL 3.4, Operational Scope: Decentralized"));
}
