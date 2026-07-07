//! `hone model` — local model discovery, hardware-fit, and selection.
//!
//! Pure local flow (docs/MODEL_REGISTRY_PROTOCOL.md "Local Flow"): scan the
//! central model store for GGUF files, read each one's own header metadata
//! (no chain call needed — no manifest has to exist yet), gate against this
//! machine's detected hardware, and let the operator enable one. HONE never
//! guesses or auto-downloads; `enable` only ever points HONE_MODEL at a file
//! that is already on disk.
//!
//! Config lives at `~/.hone/model.json` — the enabled model's filename, read by
//! `hone-node`'s own model resolution the same way `HONE_MODEL` would be (an
//! operator can still just set the env var directly; this file is a persisted
//! convenience the systemd unit / launcher can source).

use anyhow::{anyhow, bail, Context, Result};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

// ── Hardware detection ──────────────────────────────────────────────────────
//
// Reuses hone_provision::Hardware (the same type the node-wide capability
// provisioner and its Requirements::shortfalls()/met_by() gate already use),
// so `hone model`'s hardware-fit check is the identical logic hone-provision
// applies everywhere else — not a second, possibly-divergent implementation.

/// Detect this machine's capacity. Best-effort: any probe that fails just
/// leaves that field at 0 / false rather than erroring the whole command —
/// discovery should degrade gracefully on a machine without `nvidia-smi`, etc.
pub fn detect_hardware(store_dir: &Path) -> hone_provision::Hardware {
    let vram_mb = detect_vram_mb();
    hone_provision::Hardware {
        vram_mb,
        ram_mb: detect_ram_mb(),
        free_disk_mb: detect_free_disk_mb(store_dir),
        cpu_cores: num_cpus(),
        has_gpu: vram_mb > 0,
    }
}

fn detect_vram_mb() -> u64 {
    // nvidia-smi --query-gpu=memory.total --format=csv,noheader,nounits
    // Prints one line per GPU, e.g. "24576". Sum across GPUs, best-effort.
    let out = std::process::Command::new("nvidia-smi")
        .args(["--query-gpu=memory.total", "--format=csv,noheader,nounits"])
        .output();
    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter_map(|l| l.trim().parse::<u64>().ok())
            .sum(),
        _ => 0,
    }
}

#[cfg(target_os = "linux")]
fn detect_ram_mb() -> u64 {
    // /proc/meminfo's MemTotal is in kB.
    fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("MemTotal:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|kb| kb.parse::<u64>().ok())
        })
        .map(|kb| kb / 1024)
        .unwrap_or(0)
}

#[cfg(target_os = "windows")]
fn detect_ram_mb() -> u64 {
    // wmic is deprecated but still present on every Windows box we target;
    // avoids pulling in a heavyweight WinAPI binding for one number.
    let out = std::process::Command::new("wmic")
        .args(["ComputerSystem", "get", "TotalPhysicalMemory", "/value"])
        .output();
    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .lines()
            .find_map(|l| l.trim().strip_prefix("TotalPhysicalMemory="))
            .and_then(|b| b.trim().parse::<u64>().ok())
            .map(|bytes| bytes / (1024 * 1024))
            .unwrap_or(0),
        _ => 0,
    }
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn detect_ram_mb() -> u64 {
    0
}

fn detect_free_disk_mb(dir: &Path) -> u64 {
    #[cfg(target_os = "linux")]
    {
        let out = std::process::Command::new("df")
            .args(["-BM", "--output=avail", &dir.to_string_lossy()])
            .output();
        if let Ok(o) = out {
            if o.status.success() {
                return String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .nth(1)
                    .and_then(|l| l.trim().trim_end_matches('M').parse::<u64>().ok())
                    .unwrap_or(0);
            }
        }
        0
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = dir;
        0
    }
}

fn num_cpus() -> u32 {
    std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1)
}

// ── GGUF metadata (dependency-free reader) ──────────────────────────────────

/// The subset of a GGUF file's own header metadata we care about for
/// hardware-fit and display. Read directly from the file — GGUF carries this
/// in its own header, so no chain manifest is required to show it.
#[derive(Debug, Clone, Default)]
pub struct GgufInfo {
    pub architecture: Option<String>,
    pub quantization: Option<String>,
    pub context_length: Option<u64>,
    pub param_count_hint: Option<String>,
}

/// Minimal GGUF header reader (spec: https://github.com/ggml-org/ggml/blob/master/docs/gguf.md).
/// Reads only the metadata KV section — never the tensor data — so this stays
/// fast even on multi-GB files. Deliberately does not depend on candle: this
/// crate is a thin CLI and candle would be a heavy transitive pull for reading
/// a handful of header fields.
fn read_gguf_info(path: &Path) -> Result<GgufInfo> {
    use std::io::Read;
    let mut f = fs::File::open(path).with_context(|| format!("open {}", path.display()))?;

    let mut magic = [0u8; 4];
    f.read_exact(&mut magic).context("read GGUF magic")?;
    if &magic != b"GGUF" {
        bail!("not a GGUF file (bad magic)");
    }
    let version = read_u32(&mut f)?;
    if version < 2 {
        bail!("unsupported GGUF version {version}");
    }
    let tensor_count = read_u64(&mut f)?;
    let kv_count = read_u64(&mut f)?;
    let _ = tensor_count; // not needed — we stop before the tensor info section

    let mut info = GgufInfo::default();
    for _ in 0..kv_count {
        let key = read_gguf_string(&mut f)?;
        let value = read_gguf_value(&mut f)?;
        match key.as_str() {
            "general.architecture" => {
                if let GgufValue::Str(s) = &value {
                    info.architecture = Some(s.clone());
                }
            }
            k if k.ends_with(".context_length") => {
                if let GgufValue::UInt(n) = value {
                    info.context_length = Some(n);
                }
            }
            "general.file_type" => {
                if let GgufValue::UInt(n) = value {
                    info.quantization = Some(describe_file_type(n));
                }
            }
            "general.size_label" => {
                if let GgufValue::Str(s) = &value {
                    info.param_count_hint = Some(s.clone());
                }
            }
            _ => {}
        }
    }
    Ok(info)
}

enum GgufValue {
    Str(String),
    UInt(u64),
    Other,
}

fn read_gguf_value(f: &mut std::fs::File) -> Result<GgufValue> {
    let ty = read_u32(f)?;
    Ok(match ty {
        0 | 1 => { skip(f, 1)?; GgufValue::Other }              // UINT8 / INT8
        2 | 3 => { skip(f, 2)?; GgufValue::Other }              // UINT16 / INT16
        4 => GgufValue::UInt(read_u32(f)? as u64),              // UINT32
        5 => { skip(f, 4)?; GgufValue::Other }                  // INT32
        6 => { skip(f, 4)?; GgufValue::Other }                  // FLOAT32
        7 => { skip(f, 1)?; GgufValue::Other }                  // BOOL
        8 => GgufValue::Str(read_gguf_string(f)?),              // STRING
        9 => {                                                  // ARRAY — skip
            let elem_ty = read_u32(f)?;
            let len = read_u64(f)?;
            for _ in 0..len {
                skip_gguf_scalar(f, elem_ty)?;
            }
            GgufValue::Other
        }
        10 => GgufValue::UInt(read_u64(f)?),                    // UINT64
        11 => { skip(f, 8)?; GgufValue::Other }                 // INT64
        12 => { skip(f, 8)?; GgufValue::Other }                 // FLOAT64
        other => bail!("unknown GGUF value type {other}"),
    })
}

fn skip_gguf_scalar(f: &mut std::fs::File, ty: u32) -> Result<()> {
    match ty {
        0 | 1 | 7 => skip(f, 1),
        2 | 3 => skip(f, 2),
        4 | 5 | 6 => skip(f, 4),
        10 | 11 | 12 => skip(f, 8),
        8 => { read_gguf_string(f)?; Ok(()) }
        9 => bail!("nested arrays not expected in GGUF metadata"),
        other => bail!("unknown GGUF array element type {other}"),
    }
}

fn read_gguf_string(f: &mut std::fs::File) -> Result<String> {
    let len = read_u64(f)? as usize;
    // Guard against a corrupt/truncated file turning a bogus length into a
    // huge allocation.
    if len > 16 * 1024 * 1024 {
        bail!("GGUF string length implausibly large ({len} bytes) — file likely corrupt");
    }
    let mut buf = vec![0u8; len];
    std::io::Read::read_exact(f, &mut buf).context("read GGUF string bytes")?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

fn read_u32(f: &mut std::fs::File) -> Result<u32> {
    let mut b = [0u8; 4];
    std::io::Read::read_exact(f, &mut b).context("read u32")?;
    Ok(u32::from_le_bytes(b))
}

fn read_u64(f: &mut std::fs::File) -> Result<u64> {
    let mut b = [0u8; 8];
    std::io::Read::read_exact(f, &mut b).context("read u64")?;
    Ok(u64::from_le_bytes(b))
}

fn skip(f: &mut std::fs::File, n: u64) -> Result<()> {
    use std::io::{Seek, SeekFrom};
    f.seek(SeekFrom::Current(n as i64))?;
    Ok(())
}

/// GGUF `general.file_type` is a small enum of ggml quantization types.
/// Only the common ones are named; anything else shows as "type N".
fn describe_file_type(n: u64) -> String {
    match n {
        0 => "F32", 1 => "F16",
        2 => "Q4_0", 3 => "Q4_1",
        7 => "Q8_0", 8 => "Q5_0", 9 => "Q5_1",
        10 => "Q2_K", 11 => "Q3_K_S", 12 => "Q3_K_M", 13 => "Q3_K_L",
        14 => "Q4_K_S", 15 => "Q4_K_M", 16 => "Q5_K_S", 17 => "Q5_K_M",
        18 => "Q6_K", 24 => "IQ2_XXS", 31 => "BF16",
        other => return format!("type {other}"),
    }.to_string()
}

// ── Hardware requirements estimate (heuristic — no manifest exists yet) ────

/// Very rough VRAM/RAM estimate from on-disk size and quantization, used ONLY
/// when no on-chain manifest exists for this model (the common case for now,
/// since manifest publishing is a separate, not-yet-built chain feature — see
/// docs/MODEL_REGISTRY_PROTOCOL.md). A GGUF loaded via mmap needs roughly its
/// file size in RAM/VRAM plus a working-context overhead; this is intentionally
/// conservative (over- rather than under-estimates) since guessing wrong in the
/// "fits" direction is the worse failure mode.
fn estimate_requirements(size_mb: u64) -> hone_provision::Requirements {
    hone_provision::Requirements {
        vram_mb: 0,             // GPU is a speed-up, not a requirement, for candle CPU/GPU dual-path
        ram_mb: size_mb + size_mb / 4 + 512,
        disk_mb: 0,              // already on disk — discovery only lists what's present
        cpu_cores: 1,
        requires_gpu: false,
    }
}

// ── Discovery ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct DiscoveredModel {
    pub filename: String,
    pub path: PathBuf,
    pub size_mb: u64,
    pub info: GgufInfo,
    pub requirements: hone_provision::Requirements,
}

/// The central model store directory — same resolution `hone-node` uses
/// (`HONE_MODEL_DIR`, else `<data_dir>/hone/models`), so `hone model` and the
/// node always agree on where models live.
pub fn model_dir() -> PathBuf {
    std::env::var("HONE_MODEL_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            dirs_next::data_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("hone")
                .join("models")
        })
}

/// Scan the store for `.gguf` files and read each one's own header. Files that
/// fail to parse (corrupt/truncated/non-GGUF) are skipped with a warning
/// rather than aborting discovery for the whole store.
pub fn discover() -> Result<Vec<DiscoveredModel>> {
    let dir = model_dir();
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(&dir).with_context(|| format!("read model store {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("gguf") {
            continue;
        }
        let filename = path.file_name().unwrap().to_string_lossy().into_owned();
        let size_mb = entry.metadata().map(|m| m.len() / (1024 * 1024)).unwrap_or(0);
        match read_gguf_info(&path) {
            Ok(info) => {
                let requirements = estimate_requirements(size_mb);
                out.push(DiscoveredModel { filename, path, size_mb, info, requirements });
            }
            Err(e) => {
                eprintln!(
                    "{} {} — {}",
                    "skipping".yellow(),
                    filename,
                    e
                );
            }
        }
    }
    out.sort_by(|a, b| a.filename.cmp(&b.filename));
    Ok(out)
}

// ── Persisted selection ─────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Default)]
struct ModelConfig {
    enabled: Option<String>,
}

fn config_path() -> PathBuf {
    dirs_next::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".hone")
        .join("model.json")
}

fn load_config() -> ModelConfig {
    fs::read_to_string(config_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_config(cfg: &ModelConfig) -> Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, serde_json::to_string_pretty(cfg)?)?;
    Ok(())
}

// ── Commands ─────────────────────────────────────────────────────────────────

pub fn cmd_list() -> Result<()> {
    let hw = detect_hardware(&model_dir());
    let models = discover()?;
    let enabled = load_config().enabled;

    if models.is_empty() {
        println!(
            "No models in the store ({}). Copy a .gguf file there, or set HONE_MODEL_DIR.",
            model_dir().display()
        );
        return Ok(());
    }

    println!("{}", format!("Hardware: {} MB VRAM, {} MB RAM, {} cores{}",
        hw.vram_mb, hw.ram_mb, hw.cpu_cores,
        if hw.has_gpu { " (GPU detected)" } else { "" }
    ).dimmed());
    println!();

    for m in &models {
        let fits = m.requirements.met_by(&hw);
        let marker = if Some(&m.filename) == enabled.as_ref() {
            "*".green().bold()
        } else if fits {
            " ".normal()
        } else {
            "!".red()
        };
        let arch = m.info.architecture.as_deref().unwrap_or("unknown");
        let quant = m.info.quantization.as_deref().unwrap_or("?");
        let ctx = m.info.context_length.map(|c| c.to_string()).unwrap_or_else(|| "?".into());
        println!(
            "{} {}  {}  {} {}  ctx={}  {} MB",
            marker,
            m.filename.bold(),
            arch.cyan(),
            quant,
            if fits { "".into() } else { format!("({})", m.requirements.shortfalls(&hw).join("; ")).red() },
            ctx,
            m.size_mb,
        );
    }
    println!();
    println!(
        "{}",
        "* = enabled   ! = does not fit this machine's estimated requirements".dimmed()
    );
    Ok(())
}

pub fn cmd_show(name: &str) -> Result<()> {
    let models = discover()?;
    let m = models.iter().find(|m| m.filename == name)
        .ok_or_else(|| anyhow!("'{name}' not found in the model store ({})", model_dir().display()))?;
    let hw = detect_hardware(&model_dir());

    println!("{}: {}", "File".bold(), m.filename);
    println!("{}: {}", "Path".bold(), m.path.display());
    println!("{}: {} MB", "Size".bold(), m.size_mb);
    println!("{}: {}", "Architecture".bold(), m.info.architecture.as_deref().unwrap_or("unknown"));
    println!("{}: {}", "Quantization".bold(), m.info.quantization.as_deref().unwrap_or("unknown"));
    if let Some(ctx) = m.info.context_length {
        println!("{}: {}", "Context length".bold(), ctx);
    }
    if let Some(hint) = &m.info.param_count_hint {
        println!("{}: {}", "Size label".bold(), hint);
    }
    let shortfalls = m.requirements.shortfalls(&hw);
    if shortfalls.is_empty() {
        println!("{}", "Fits this machine's estimated requirements.".green());
    } else {
        println!("{}", "Does not fit:".red());
        for s in shortfalls {
            println!("  - {s}");
        }
    }
    println!();
    println!(
        "{}",
        "No on-chain manifest lookup yet (docs/MODEL_REGISTRY_PROTOCOL.md) — \
         the requirement estimate above is a local heuristic from file size, \
         not a published analyzer manifest.".dimmed()
    );
    Ok(())
}

pub fn cmd_enable(name: &str, force: bool) -> Result<()> {
    let models = discover()?;
    let m = models.iter().find(|m| m.filename == name)
        .ok_or_else(|| anyhow!("'{name}' not found in the model store ({})", model_dir().display()))?;

    let hw = detect_hardware(&model_dir());
    let shortfalls = m.requirements.shortfalls(&hw);
    if !shortfalls.is_empty() && !force {
        bail!(
            "'{name}' does not fit this machine's estimated requirements:\n  {}\n\
             Pass --force to enable anyway.",
            shortfalls.join("\n  ")
        );
    }

    save_config(&ModelConfig { enabled: Some(name.to_owned()) })?;
    println!("{} {}", "Enabled".green().bold(), name);
    println!(
        "Set {} for the node to pick this up, or point it at {}:",
        "HONE_MODEL".bold(), config_path().display()
    );
    println!("  export HONE_MODEL={}", name);
    Ok(())
}

pub fn cmd_disable() -> Result<()> {
    save_config(&ModelConfig { enabled: None })?;
    println!("{}", "Disabled — no model enabled.".yellow());
    Ok(())
}

pub fn cmd_current() -> Result<()> {
    match load_config().enabled {
        Some(name) => println!("{}", name),
        None => {
            println!("{}", "No model enabled.".yellow());
            println!("Run `hone model list` to see what's available, then `hone model enable <file>`.");
        }
    }
    Ok(())
}
