//! UFL 3.8 Decentralized acceptance gate (UFL Section 9).
//!
//! HONE is licensed under the Usufruct License (UFL) 3.8, Operational Scope:
//! Decentralized. That scope's dispute process, liability limit, and own-risk
//! terms take effect only through an affirmative acceptance step that the
//! Software presents before first use, and the Software must not run until it
//! is completed. This crate is that step. Every HONE binary calls
//! [`require_acceptance`] (or [`require_acceptance_or_exit`]) before doing any
//! work.
//!
//! Behavior:
//! - `{data_dir}/license-acceptance.json` naming this version and scope: run.
//! - `--accept-ufl 3.8` or `HONE_ACCEPT_LICENSE=UFL-3.8-D`: accept, record, run.
//!   Any other value of either is rejected.
//! - Interactive terminal: show the terms, require the typed phrase
//!   `I accept`. Anything else exits without starting.
//! - No terminal and no explicit acceptance: exit non-zero, naming the
//!   variable and the flag.
//!
//! The gate never accepts on the user's behalf. The record holds only the
//! license, version, scope, UTC time, and binary name, and stays in the local
//! data directory. Nothing is sent over the network or written on chain.
//!
//! This crate has no dependencies on purpose: it is linked into standalone
//! crates that keep their own lockfiles.

use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// UFL version HONE is released under.
pub const UFL_VERSION: &str = "3.8";
/// Operational Scope HONE adopts.
pub const SCOPE: &str = "Decentralized";
/// SPDX identifier (UFL is not SPDX-registered, hence `LicenseRef-`).
pub const SPDX: &str = "LicenseRef-UFL-3.8-D";
/// UFL Section 1C Ruling License statement for HONE releases.
pub const RULING_LICENSE: &str = "UFL 3.8, Operational Scope: Decentralized";
/// Environment variable for non-interactive acceptance.
pub const ENV_VAR: &str = "HONE_ACCEPT_LICENSE";
/// The only value of [`ENV_VAR`] that counts as acceptance.
pub const ENV_VALUE: &str = "UFL-3.8-D";
/// Command-line flag for non-interactive acceptance.
pub const FLAG: &str = "--accept-ufl";
/// The only value of [`FLAG`] that counts as acceptance.
pub const FLAG_VALUE: &str = "3.8";
/// Phrase the user must type at the interactive prompt.
pub const ACCEPT_PHRASE: &str = "I accept";
/// Record file name inside the data directory.
pub const RECORD_FILE: &str = "license-acceptance.json";
/// Where HONE's license text is published.
pub const LICENSE_URL: &str = "https://github.com/shindevlin/hone/blob/main/LICENSE";
/// Exit status when the license has not been accepted.
pub const EXIT_NOT_ACCEPTED: i32 = 3;
/// Full text of HONE's license, embedded so the gate can show it offline.
pub const LICENSE_TEXT: &str = include_str!("../../../LICENSE");

/// How the gate may ask for acceptance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Prompt if stdin and stderr are both terminals, otherwise behave as
    /// [`Mode::Headless`]. The right choice for almost every binary.
    Auto,
    /// Always prompt on stdin/stderr, terminal or not.
    Interactive,
    /// Never prompt. Accept only a matching record, the flag, or the variable.
    /// For processes whose stdin is a protocol pipe (e.g. a git remote helper).
    Headless,
}

/// How acceptance was established for this run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via {
    /// A matching record from an earlier acceptance.
    Record,
    /// `HONE_ACCEPT_LICENSE=UFL-3.8-D`.
    Env,
    /// `--accept-ufl 3.8`.
    Flag,
    /// The user typed the acceptance phrase.
    Prompt,
}

/// Successful gate result.
#[derive(Debug)]
pub struct Acceptance {
    pub via: Via,
    pub record_path: PathBuf,
    /// False if a new acceptance could not be saved (the next run will ask
    /// again). Always true for [`Via::Record`].
    pub recorded: bool,
}

/// What was found in the record file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecordState {
    Missing,
    Matching,
    /// A record for another license version or scope.
    Other { version: String, scope: String },
    Unreadable,
}

/// Why the gate refused to let the binary run.
#[derive(Debug)]
pub enum GateError {
    /// The user did not type the acceptance phrase.
    Declined,
    /// No terminal to ask on, and no explicit acceptance given.
    NotAccepted { record: RecordState },
    /// `HONE_ACCEPT_LICENSE` is set to something other than `UFL-3.8-D`.
    InvalidEnv(String),
    /// `--accept-ufl` was given without `3.8`.
    InvalidFlag(String),
    Io(io::Error),
}

impl fmt::Display for GateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GateError::Declined => write!(
                f,
                "license not accepted; HONE did not start. Run again and type \"{ACCEPT_PHRASE}\" to accept {RULING_LICENSE}."
            ),
            GateError::NotAccepted { record } => {
                let why = match record {
                    RecordState::Other { version, scope } => format!(
                        "HONE's license changed (you accepted UFL {version}, {scope}; it is now {RULING_LICENSE}), and no terminal is attached to ask again."
                    ),
                    _ => "HONE will not start until you accept its license, and no terminal is attached to ask.".to_string(),
                };
                write!(
                    f,
                    "{why}\n\n{}\nTo accept without a terminal, set {ENV_VAR}={ENV_VALUE} or pass {FLAG} {FLAG_VALUE}. Setting either is the act of acceptance.",
                    summary()
                )
            }
            GateError::InvalidEnv(v) => write!(
                f,
                "{ENV_VAR} is set to {v:?}. The only value that accepts HONE's license is {ENV_VALUE} ({RULING_LICENSE}). HONE did not start."
            ),
            GateError::InvalidFlag(v) => write!(
                f,
                "{FLAG} was given {v:?}. The only value that accepts HONE's license is {FLAG_VALUE} ({RULING_LICENSE}). HONE did not start."
            ),
            GateError::Io(e) => write!(f, "license acceptance step failed: {e}"),
        }
    }
}

impl std::error::Error for GateError {}

impl From<io::Error> for GateError {
    fn from(e: io::Error) -> Self {
        GateError::Io(e)
    }
}

/// Plain-language summary of the terms shown before acceptance. The license
/// text governs; this names the version and scope and shows the dispute
/// process and liability limit, as UFL Section 9 requires.
pub fn summary() -> String {
    format!(
        "HONE is licensed under the Usufruct License (UFL) {UFL_VERSION}, Operational Scope: {SCOPE} ({SPDX}).
Full text: {LICENSE_URL}

Summary (the license text governs):
  * Use is free at any scale and nothing is paid. Redistribution is reserved,
    except that forks of HONE as network software may be distributed if they
    credit HONE and keep the license's canonical-source notice (Section 2A).
  * Disputes: every dispute arising from your use of HONE is decided only by
    online arbitration under the ICC Rules, before a single arbitrator seated
    in California, on an individual basis and not as a class or collective
    action. The arbitrator applies the UNIDROIT Principles, then California
    law. Each side bears its own costs and fees unless a claim or defense is
    frivolous or in bad faith.
  * Liability cap: each party's total liability to the other for all claims
    is limited, to the extent the law allows, to one US dollar in value,
    paid in money or in HONE at the paying party's choice. HONE is valued at
    its 24-hour volume-weighted price on its busiest exchange, or at one US
    dollar per HONE while no exchange lists it.
  * No one consents to any court's jurisdiction by using HONE, and no use is
    Paid Use (Sections 8 and 10 to 13 do not apply).
  * Forks and bypassers are at their own risk: running HONE with this step
    bypassed, removed, or forged breaches the license and makes that copy a
    modified version, and the Licensor then owes you nothing. A modified
    version, fork, or reimplementation is used entirely at your own risk.
  * HONE is provided as is, without warranty.
  * Nothing here limits liability or waives rights that the law does not
    allow to be limited or waived, or stops either side seeking a court
    injunction against conduct Section 2 reserves or IP infringement.
"
    )
}

/// The record path for a data directory.
pub fn record_path(data_dir: &Path) -> PathBuf {
    data_dir.join(RECORD_FILE)
}

/// HONE's data directory: `HONE_DATA_DIR`, else `~/.hone` (same rule as
/// hone-node's `Config::from_env`).
pub fn default_data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("HONE_DATA_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(d);
    }
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".hone")
}

/// Run the gate for this process: read the record, the flag, and the
/// environment variable, and prompt when `mode` allows it.
pub fn require_acceptance(data_dir: &Path, mode: Mode) -> Result<Acceptance, GateError> {
    let inputs = Inputs {
        env: std::env::var(ENV_VAR).ok(),
        flag: flag_value(std::env::args_os()),
        tty: io::stdin().is_terminal() && io::stderr().is_terminal(),
        binary: current_binary(),
        now_utc: now_utc(),
    };
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut out = io::stderr();
    run_gate(data_dir, mode, &inputs, &mut input, &mut out)
}

/// [`require_acceptance`] with [`Mode::Auto`]; on refusal, print why and exit
/// with [`EXIT_NOT_ACCEPTED`]. Call it first thing in `main`.
pub fn require_acceptance_or_exit(data_dir: &Path) -> Acceptance {
    require_acceptance_mode_or_exit(data_dir, Mode::Auto)
}

/// [`require_acceptance`] with an explicit mode; exits on refusal.
pub fn require_acceptance_mode_or_exit(data_dir: &Path, mode: Mode) -> Acceptance {
    match require_acceptance(data_dir, mode) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{}: {e}", current_binary());
            std::process::exit(EXIT_NOT_ACCEPTED);
        }
    }
}

/// This process's arguments with `--accept-ufl <v>` / `--accept-ufl=<v>`
/// removed, for binaries whose own argument parser would reject the flag.
pub fn args_os() -> Vec<OsString> {
    strip_flag(std::env::args_os())
}

/// [`args_os`] as `String`s (lossy), for binaries that parse `env::args()`.
pub fn args() -> Vec<String> {
    args_os().into_iter().map(|a| a.to_string_lossy().into_owned()).collect()
}

/// Inputs to the gate, separated from the process so the logic is testable.
#[derive(Clone, Debug)]
pub struct Inputs {
    pub env: Option<String>,
    /// `None` if the flag is absent; `Some("")` if present with no value.
    pub flag: Option<String>,
    pub tty: bool,
    pub binary: String,
    pub now_utc: String,
}

/// Gate logic. `input`/`out` are the prompt's stdin and stderr.
pub fn run_gate(
    data_dir: &Path,
    mode: Mode,
    inputs: &Inputs,
    input: &mut dyn BufRead,
    out: &mut dyn Write,
) -> Result<Acceptance, GateError> {
    let path = record_path(data_dir);
    let record = read_record(&path);
    if record == RecordState::Matching {
        return Ok(Acceptance { via: Via::Record, record_path: path, recorded: true });
    }

    // Explicit, non-interactive acceptance. A wrong value is refused outright,
    // never treated as absent, so a typo cannot fall through to "run anyway".
    if let Some(v) = &inputs.flag {
        if v != FLAG_VALUE {
            return Err(GateError::InvalidFlag(v.clone()));
        }
        return Ok(accept(&path, Via::Flag, inputs, out));
    }
    if let Some(v) = inputs.env.as_deref().filter(|v| !v.is_empty()) {
        if v != ENV_VALUE {
            return Err(GateError::InvalidEnv(v.to_string()));
        }
        return Ok(accept(&path, Via::Env, inputs, out));
    }

    let prompt = match mode {
        Mode::Interactive => true,
        Mode::Headless => false,
        Mode::Auto => inputs.tty,
    };
    if !prompt {
        return Err(GateError::NotAccepted { record });
    }

    if let RecordState::Other { version, scope } = &record {
        writeln!(out, "\nHONE's license has changed since you accepted UFL {version}, {scope}. Please review and accept the new terms.")?;
    }
    writeln!(out, "\n{}", summary())?;
    loop {
        write!(
            out,
            "Type \"{ACCEPT_PHRASE}\" to accept and continue, \"show\" to print the full license, or anything else to exit: "
        )?;
        out.flush()?;
        let mut line = String::new();
        if input.read_line(&mut line)? == 0 {
            writeln!(out)?;
            return Err(GateError::Declined);
        }
        match line.trim() {
            ACCEPT_PHRASE => return Ok(accept(&path, Via::Prompt, inputs, out)),
            "show" => writeln!(out, "\n{LICENSE_TEXT}\n")?,
            _ => return Err(GateError::Declined),
        }
    }
}

fn accept(path: &Path, via: Via, inputs: &Inputs, out: &mut dyn Write) -> Acceptance {
    let how = match via {
        Via::Flag => format!("{FLAG} {FLAG_VALUE}"),
        Via::Env => format!("{ENV_VAR}={ENV_VALUE}"),
        _ => "prompt".to_string(),
    };
    let recorded = match write_record(path, &inputs.binary, &inputs.now_utc) {
        Ok(()) => {
            let _ = writeln!(
                out,
                "{}: accepted {RULING_LICENSE} via {how}; recorded locally at {}",
                inputs.binary,
                path.display()
            );
            true
        }
        Err(e) => {
            // Acceptance stands for this run; it just could not be saved.
            let _ = writeln!(
                out,
                "{}: accepted {RULING_LICENSE} via {how}, but could not save the record at {} ({e}); you will be asked again next run",
                inputs.binary,
                path.display()
            );
            false
        }
    };
    Acceptance { via, record_path: path.to_path_buf(), recorded }
}

/// Read and classify the record file.
pub fn read_record(path: &Path) -> RecordState {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return RecordState::Missing,
        Err(_) => return RecordState::Unreadable,
    };
    let (Some(lic), Some(version), Some(scope)) = (
        json_str(&text, "license"),
        json_str(&text, "version"),
        json_str(&text, "scope"),
    ) else {
        return RecordState::Unreadable;
    };
    if lic == "UFL" && version == UFL_VERSION && scope == SCOPE {
        RecordState::Matching
    } else {
        RecordState::Other { version, scope }
    }
}

/// Write the record: license, version, scope, UTC time, binary name. Nothing
/// else, and only to the local data directory.
pub fn write_record(path: &Path, binary: &str, now_utc: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let body = format!(
        "{{\n  \"license\": \"UFL\",\n  \"version\": \"{}\",\n  \"scope\": \"{}\",\n  \"accepted_at\": \"{}\",\n  \"binary\": \"{}\"\n}}\n",
        UFL_VERSION,
        SCOPE,
        json_escape(now_utc),
        json_escape(binary)
    );
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, body)?;
    fs::rename(&tmp, path)
}

/// Value of `--accept-ufl` in an argument list: `None` if absent.
pub fn flag_value<I: IntoIterator<Item = OsString>>(args: I) -> Option<String> {
    let mut it = args.into_iter().skip(1);
    while let Some(a) = it.next() {
        let a = a.to_string_lossy().into_owned();
        if a == FLAG {
            return Some(it.next().map(|v| v.to_string_lossy().into_owned()).unwrap_or_default());
        }
        if let Some(v) = a.strip_prefix(FLAG).and_then(|r| r.strip_prefix('=')) {
            return Some(v.to_string());
        }
    }
    None
}

/// Remove `--accept-ufl <v>` and `--accept-ufl=<v>` from an argument list.
pub fn strip_flag<I: IntoIterator<Item = OsString>>(args: I) -> Vec<OsString> {
    let mut out = Vec::new();
    let mut it = args.into_iter();
    if let Some(first) = it.next() {
        out.push(first);
    }
    while let Some(a) = it.next() {
        let s = a.to_string_lossy();
        if s == FLAG {
            it.next();
            continue;
        }
        if s.strip_prefix(FLAG).is_some_and(|r| r.starts_with('=')) {
            continue;
        }
        out.push(a);
    }
    out
}

/// File stem of this process's executable (e.g. `hone-node`).
pub fn current_binary() -> String {
    std::env::args_os()
        .next()
        .map(PathBuf::from)
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "hone".to_string())
}

/// Current time as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn now_utc() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    format_utc(secs)
}

/// Format Unix seconds as an RFC 3339 UTC timestamp.
pub fn format_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

fn json_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o
}

/// Minimal string-field lookup for the flat record this crate writes.
fn json_str(text: &str, key: &str) -> Option<String> {
    let pat = format!("\"{key}\"");
    let at = text.find(&pat)? + pat.len();
    let rest = text[at..].trim_start().strip_prefix(':')?.trim_start().strip_prefix('"')?;
    let mut val = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(val),
            '\\' => val.push(chars.next()?),
            c => val.push(c),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "hone-license-test-{tag}-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn inputs(env: Option<&str>, flag: Option<&str>, tty: bool) -> Inputs {
        Inputs {
            env: env.map(str::to_string),
            flag: flag.map(str::to_string),
            tty,
            binary: "hone-test".into(),
            now_utc: "2026-10-05T12:00:00Z".into(),
        }
    }

    fn gate(dir: &Path, mode: Mode, i: &Inputs, typed: &str) -> (Result<Acceptance, GateError>, String) {
        let mut input = Cursor::new(typed.as_bytes().to_vec());
        let mut out = Vec::new();
        let r = run_gate(dir, mode, i, &mut input, &mut out);
        (r, String::from_utf8(out).unwrap())
    }

    fn write_raw(dir: &Path, version: &str, scope: &str) {
        fs::write(
            record_path(dir),
            format!("{{\"license\": \"UFL\", \"version\": \"{version}\", \"scope\": \"{scope}\", \"accepted_at\": \"2026-01-01T00:00:00Z\", \"binary\": \"hone-node\"}}"),
        )
        .unwrap();
    }

    #[test]
    fn matching_record_runs_without_prompt() {
        let d = tmpdir("match");
        write_raw(&d, "3.8", "Decentralized");
        let (r, out) = gate(&d, Mode::Auto, &inputs(None, None, false), "");
        assert_eq!(r.unwrap().via, Via::Record);
        assert!(out.is_empty());
    }

    #[test]
    fn missing_record_headless_refuses() {
        let d = tmpdir("missing");
        let (r, _) = gate(&d, Mode::Auto, &inputs(None, None, false), "I accept\n");
        match r {
            Err(GateError::NotAccepted { record: RecordState::Missing }) => {}
            other => panic!("expected NotAccepted, got {other:?}"),
        }
        assert!(!record_path(&d).exists(), "must not auto-accept");
    }

    #[test]
    fn refusal_message_names_variable_flag_and_terms() {
        let msg = GateError::NotAccepted { record: RecordState::Missing }.to_string();
        assert!(msg.contains("HONE_ACCEPT_LICENSE=UFL-3.8-D"));
        assert!(msg.contains("--accept-ufl 3.8"));
        assert!(msg.contains("one US dollar"));
        assert!(msg.contains("ICC"));
    }

    #[test]
    fn wrong_version_record_does_not_run_headless() {
        let d = tmpdir("oldver");
        write_raw(&d, "3.3", "Decentralized");
        let (r, _) = gate(&d, Mode::Auto, &inputs(None, None, false), "");
        match r {
            Err(GateError::NotAccepted { record: RecordState::Other { version, .. } }) => assert_eq!(version, "3.3"),
            other => panic!("expected NotAccepted(Other), got {other:?}"),
        }
    }

    #[test]
    fn wrong_scope_record_does_not_match() {
        let d = tmpdir("scope");
        write_raw(&d, "3.8", "Unconditional");
        assert!(matches!(read_record(&record_path(&d)), RecordState::Other { .. }));
    }

    #[test]
    fn wrong_version_record_reprompts_and_updates() {
        let d = tmpdir("reprompt");
        write_raw(&d, "3.3", "Decentralized");
        let (r, out) = gate(&d, Mode::Auto, &inputs(None, None, true), "I accept\n");
        assert_eq!(r.unwrap().via, Via::Prompt);
        assert!(out.contains("license has changed"));
        assert_eq!(read_record(&record_path(&d)), RecordState::Matching);
    }

    #[test]
    fn env_accepts_and_records() {
        let d = tmpdir("env");
        let (r, _) = gate(&d, Mode::Auto, &inputs(Some("UFL-3.8-D"), None, false), "");
        let a = r.unwrap();
        assert_eq!(a.via, Via::Env);
        assert!(a.recorded);
        let text = fs::read_to_string(record_path(&d)).unwrap();
        assert_eq!(json_str(&text, "version").as_deref(), Some("3.8"));
        assert_eq!(json_str(&text, "scope").as_deref(), Some("Decentralized"));
        assert_eq!(json_str(&text, "accepted_at").as_deref(), Some("2026-10-05T12:00:00Z"));
        assert_eq!(json_str(&text, "binary").as_deref(), Some("hone-test"));
        // Only these fields: nothing that identifies the user or machine.
        assert_eq!(text.matches(':').count() - 2 /* time */, 5);
    }

    #[test]
    fn wrong_env_rejected_even_on_tty() {
        for bad in ["1", "yes", "true", "UFL-3.4-D", "ufl-3.8-d", "UFL-3.8"] {
            let d = tmpdir("badenv");
            let (r, _) = gate(&d, Mode::Auto, &inputs(Some(bad), None, true), "I accept\n");
            assert!(matches!(r, Err(GateError::InvalidEnv(_))), "{bad} must be rejected");
            assert!(!record_path(&d).exists());
        }
    }

    #[test]
    fn empty_env_is_treated_as_unset() {
        let d = tmpdir("emptyenv");
        let (r, _) = gate(&d, Mode::Auto, &inputs(Some(""), None, false), "");
        assert!(matches!(r, Err(GateError::NotAccepted { .. })));
    }

    #[test]
    fn flag_accepts_and_wrong_flag_rejected() {
        let d = tmpdir("flag");
        let (r, _) = gate(&d, Mode::Headless, &inputs(None, Some("3.8"), false), "");
        assert_eq!(r.unwrap().via, Via::Flag);
        let d = tmpdir("badflag");
        let (r, _) = gate(&d, Mode::Headless, &inputs(None, Some("3.3"), false), "");
        assert!(matches!(r, Err(GateError::InvalidFlag(_))));
        let (r, _) = gate(&d, Mode::Headless, &inputs(None, Some(""), false), "");
        assert!(matches!(r, Err(GateError::InvalidFlag(_))));
    }

    #[test]
    fn prompt_requires_exact_phrase() {
        for (typed, ok) in [
            ("I accept\n", true),
            ("  I accept  \r\n", true),
            ("i accept\n", false),
            ("yes\n", false),
            ("\n", false),
            ("", false),
        ] {
            let d = tmpdir("prompt");
            let (r, out) = gate(&d, Mode::Auto, &inputs(None, None, true), typed);
            assert_eq!(r.is_ok(), ok, "typed {typed:?}");
            assert_eq!(record_path(&d).exists(), ok);
            assert!(out.contains("one US dollar") && out.contains("ICC") && out.contains("own risk"));
            assert!(out.contains("UFL) 3.8, Operational Scope: Decentralized"));
        }
    }

    #[test]
    fn show_prints_full_text_then_asks_again() {
        let d = tmpdir("show");
        let (r, out) = gate(&d, Mode::Auto, &inputs(None, None, true), "show\nI accept\n");
        assert!(r.is_ok());
        assert!(out.contains("The Usufruct License (UFL) — Version 3.8"));
    }

    #[test]
    fn headless_mode_never_prompts() {
        let d = tmpdir("headless");
        let (r, out) = gate(&d, Mode::Headless, &inputs(None, None, true), "I accept\n");
        assert!(matches!(r, Err(GateError::NotAccepted { .. })));
        assert!(out.is_empty());
    }

    #[test]
    fn unreadable_record_is_not_acceptance() {
        let d = tmpdir("garbage");
        fs::write(record_path(&d), "not json").unwrap();
        let (r, _) = gate(&d, Mode::Auto, &inputs(None, None, false), "");
        assert!(matches!(r, Err(GateError::NotAccepted { record: RecordState::Unreadable })));
    }

    #[test]
    fn unwritable_record_still_accepts_this_run() {
        let d = tmpdir("ro");
        let blocker = d.join("file");
        fs::write(&blocker, "x").unwrap();
        // data_dir is a regular file, so the record cannot be created.
        let (r, out) = gate(&blocker, Mode::Auto, &inputs(Some("UFL-3.8-D"), None, false), "");
        let a = r.unwrap();
        assert!(!a.recorded);
        assert!(out.contains("asked again"));
    }

    #[test]
    fn flag_parsing_and_stripping() {
        let a = |v: &[&str]| v.iter().map(OsString::from).collect::<Vec<_>>();
        assert_eq!(flag_value(a(&["bin", "--accept-ufl", "3.8", "run"])).as_deref(), Some("3.8"));
        assert_eq!(flag_value(a(&["bin", "--accept-ufl=3.8"])).as_deref(), Some("3.8"));
        assert_eq!(flag_value(a(&["bin", "--accept-ufl"])).as_deref(), Some(""));
        assert_eq!(flag_value(a(&["bin", "status"])), None);
        // argv[0] is never read as the flag.
        assert_eq!(flag_value(a(&["--accept-ufl", "x"])), None);
        assert_eq!(strip_flag(a(&["bin", "--accept-ufl", "3.8", "run", "-v"])), a(&["bin", "run", "-v"]));
        assert_eq!(strip_flag(a(&["bin", "run", "--accept-ufl=3.8"])), a(&["bin", "run"]));
    }

    #[test]
    fn utc_formatting() {
        assert_eq!(format_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_utc(1_783_191_600), "2026-07-04T19:00:00Z");
        assert_eq!(format_utc(951_782_400), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn embedded_license_is_ufl_3_4_decentralized() {
        assert!(LICENSE_TEXT.starts_with("The Usufruct License (UFL) — Version 3.8"));
        assert!(LICENSE_TEXT.contains("Operational Scope: Decentralized"));
        assert!(LICENSE_TEXT.contains("Native\ntoken of the Software: HONE") || LICENSE_TEXT.contains("token of the Software: HONE"));
        assert!(LICENSE_TEXT.contains("LicenseRef-UFL-3.8"));
    }
}
