//! Memory profile — how much RAM this node lets itself use.
//!
//! Measured on grouchly (2026-11-02, 20 cores): 690 MB RSS ten minutes after a restart,
//! of which glibc reported only 64 MB in use. The rest was 38 malloc arenas (glibc makes
//! up to 8 per core) each holding ~50 MB of freed-but-retained pages from transient
//! serde_json / RocksDB churn. One `malloc_trim(0)` dropped RSS 694 MB → 186 MB. On a
//! 4-core Pi with 906 MB that same pattern (32 arenas) is what thrashed nebra.
//!
//! So every node (deterministically, regardless of profile):
//!   * caps glibc arenas and pins the mmap threshold before any thread starts,
//!   * runs a periodic `malloc_trim(0)`,
//!   * shares ONE RocksDB block cache across column families (it was 9 × 32 MB) and caps
//!     memtables (it was 9 CFs × 2 × 64 MB).
//!
//! The LIGHT profile (auto when MemTotal < 2 GiB, or `HONE_PROFILE=light`) shrinks those
//! further, slows the catch-up vote sender, bounds the catch-up vote map, and defaults the
//! storage / worker / embedded-inference roles off. None of this touches consensus state:
//! it changes caches, buffer sizes and how fast votes are (re)sent — never what is applied.
//!
//! Env overrides (all optional): HONE_PROFILE=light|full, HONE_ROCKS_CACHE_MB,
//! HONE_ROCKS_WRITE_BUFFER_MB, HONE_CATCHUP_BATCH, HONE_CATCHUP_INTERVAL_SECS,
//! HONE_CATCHUP_VOTES_MAX, HONE_MALLOC_TRIM_SECS. MALLOC_ARENA_MAX in the environment
//! is respected (glibc reads it itself; we do not override it).

use std::sync::OnceLock;

/// Below this much physical RAM a node auto-selects the light profile.
pub const LIGHT_RAM_THRESHOLD_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemProfile {
    pub light: bool,
    /// Shared RocksDB LRU block cache for all column families.
    pub rocks_cache_bytes: usize,
    /// Per-CF memtable size.
    pub rocks_write_buffer_bytes: usize,
    /// Hard cap on all memtables together (RocksDB flushes when reached).
    pub rocks_db_write_buffer_bytes: usize,
    pub rocks_max_open_files: i32,
    /// Catch-up reward votes per batch, and seconds between batches.
    pub catchup_batch: usize,
    pub catchup_interval_secs: u64,
    /// Max past epochs whose catch-up votes are held in memory.
    pub catchup_votes_max: usize,
    /// Seconds between malloc_trim(0) calls (0 = off).
    pub malloc_trim_secs: u64,
}

const MB: usize = 1024 * 1024;

impl MemProfile {
    pub fn full() -> Self {
        Self {
            light: false,
            rocks_cache_bytes: 64 * MB,
            rocks_write_buffer_bytes: 16 * MB,
            rocks_db_write_buffer_bytes: 64 * MB,
            rocks_max_open_files: 512,
            catchup_batch: 512,
            catchup_interval_secs: 3,
            catchup_votes_max: 100_000,
            malloc_trim_secs: 30,
        }
    }

    pub fn light() -> Self {
        Self {
            light: true,
            rocks_cache_bytes: 16 * MB,
            rocks_write_buffer_bytes: 4 * MB,
            rocks_db_write_buffer_bytes: 16 * MB,
            rocks_max_open_files: 128,
            catchup_batch: 128,
            catchup_interval_secs: 6,
            catchup_votes_max: 20_000,
            malloc_trim_secs: 15,
        }
    }

    /// Resolve from an env lookup and the machine's RAM. Pure, for tests.
    pub fn resolve(env: impl Fn(&str) -> Option<String>, total_ram: Option<u64>) -> Self {
        let light = match env("HONE_PROFILE").as_deref().map(str::trim) {
            Some("light") => true,
            Some("full") => false,
            _ => total_ram.map(|r| r < LIGHT_RAM_THRESHOLD_BYTES).unwrap_or(false),
        };
        let mut p = if light { Self::light() } else { Self::full() };
        let num = |k: &str| env(k).and_then(|v| v.trim().parse::<u64>().ok());
        if let Some(v) = num("HONE_ROCKS_CACHE_MB") { p.rocks_cache_bytes = (v as usize).max(1) * MB; }
        if let Some(v) = num("HONE_ROCKS_WRITE_BUFFER_MB") {
            p.rocks_write_buffer_bytes = (v as usize).max(1) * MB;
            p.rocks_db_write_buffer_bytes = p.rocks_db_write_buffer_bytes.max(p.rocks_write_buffer_bytes * 2);
        }
        if let Some(v) = num("HONE_CATCHUP_BATCH") { p.catchup_batch = (v as usize).clamp(1, 4096); }
        if let Some(v) = num("HONE_CATCHUP_INTERVAL_SECS") { p.catchup_interval_secs = v.clamp(1, 600); }
        if let Some(v) = num("HONE_CATCHUP_VOTES_MAX") { p.catchup_votes_max = (v as usize).max(1_000); }
        if let Some(v) = num("HONE_MALLOC_TRIM_SECS") { p.malloc_trim_secs = v; }
        p
    }

    /// Role default for storage / worker / embedded inference: off on light nodes unless
    /// the operator set the env var explicitly.
    pub fn role_default(&self, detected: bool) -> bool {
        detected && !self.light
    }
}

static PROFILE: OnceLock<MemProfile> = OnceLock::new();

/// The process-wide profile, resolved once from the environment and /proc/meminfo.
pub fn get() -> &'static MemProfile {
    PROFILE.get_or_init(|| MemProfile::resolve(|k| std::env::var(k).ok(), total_ram_bytes()))
}

/// MemTotal from /proc/meminfo (Linux). None elsewhere.
pub fn total_ram_bytes() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/meminfo").ok()?;
    parse_memtotal(&s)
}

fn parse_memtotal(s: &str) -> Option<u64> {
    s.lines()
        .find(|l| l.starts_with("MemTotal:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|kb| kb.parse::<u64>().ok())
        .map(|kb| kb * 1024)
}

// ── glibc allocator tuning ───────────────────────────────────────────────────

#[cfg(all(target_os = "linux", target_env = "gnu"))]
mod glibc {
    extern "C" {
        pub fn mallopt(param: i32, value: i32) -> i32;
        pub fn malloc_trim(pad: usize) -> i32;
    }
    pub const M_TRIM_THRESHOLD: i32 = -1;
    pub const M_MMAP_THRESHOLD: i32 = -3;
    pub const M_ARENA_MAX: i32 = -8;
}

/// Call from `main` BEFORE the tokio runtime (or any other thread) starts, so the arena
/// cap is in force before glibc computes its arena limit.
pub fn tune_allocator() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    unsafe {
        if std::env::var_os("MALLOC_ARENA_MAX").is_none() {
            glibc::mallopt(glibc::M_ARENA_MAX, 2);
        }
        // A fixed threshold stops glibc raising it after a big free (dynamic threshold),
        // which is what pushes multi-MB JSON/RocksDB buffers into the arenas. Big buffers
        // stay mmap-backed and go back to the kernel the moment they are freed.
        if std::env::var_os("MALLOC_MMAP_THRESHOLD_").is_none() {
            glibc::mallopt(glibc::M_MMAP_THRESHOLD, 256 * 1024);
        }
        if std::env::var_os("MALLOC_TRIM_THRESHOLD_").is_none() {
            glibc::mallopt(glibc::M_TRIM_THRESHOLD, 4 * 1024 * 1024);
        }
    }
}

/// Give freed heap pages back to the kernel. Returns true if anything was released.
pub fn trim_now() -> bool {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    unsafe {
        return glibc::malloc_trim(0) == 1;
    }
    #[allow(unreachable_code)]
    false
}

/// Background OS thread running `trim_now()` every `malloc_trim_secs`.
pub fn spawn_trimmer() {
    let secs = get().malloc_trim_secs;
    if secs == 0 { return; }
    let _ = std::thread::Builder::new()
        .name("hone-malloc-trim".into())
        .spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(secs));
            trim_now();
        });
}

/// Current RSS in kB (Linux), for the startup/periodic log line.
pub fn rss_kb() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    s.lines()
        .find(|l| l.starts_with("VmRSS:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|v| v.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(m: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: HashMap<String, String> = m.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |k| m.get(k).cloned()
    }
    const GB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn small_ram_selects_light_large_selects_full() {
        assert!(MemProfile::resolve(env(&[]), Some(906 * 1024 * 1024)).light);
        assert!(!MemProfile::resolve(env(&[]), Some(64 * GB)).light);
        // Unknown RAM: never guess light.
        assert!(!MemProfile::resolve(env(&[]), None).light);
    }

    #[test]
    fn explicit_profile_wins_over_ram() {
        assert!(!MemProfile::resolve(env(&[("HONE_PROFILE", "full")]), Some(GB / 2)).light);
        assert!(MemProfile::resolve(env(&[("HONE_PROFILE", "light")]), Some(64 * GB)).light);
    }

    #[test]
    fn light_is_strictly_smaller_than_full() {
        let (l, f) = (MemProfile::light(), MemProfile::full());
        assert!(l.rocks_cache_bytes < f.rocks_cache_bytes);
        assert!(l.rocks_write_buffer_bytes < f.rocks_write_buffer_bytes);
        assert!(l.rocks_db_write_buffer_bytes < f.rocks_db_write_buffer_bytes);
        assert!(l.catchup_batch < f.catchup_batch);
        assert!(l.catchup_votes_max < f.catchup_votes_max);
        // Memtable total must hold at least two per-CF buffers or RocksDB flushes constantly.
        for p in [l, f] { assert!(p.rocks_db_write_buffer_bytes >= 2 * p.rocks_write_buffer_bytes); }
    }

    #[test]
    fn env_overrides_apply_and_clamp() {
        let p = MemProfile::resolve(env(&[
            ("HONE_PROFILE", "light"),
            ("HONE_ROCKS_CACHE_MB", "8"),
            ("HONE_CATCHUP_BATCH", "999999"),
            ("HONE_CATCHUP_INTERVAL_SECS", "0"),
            ("HONE_CATCHUP_VOTES_MAX", "5"),
        ]), None);
        assert_eq!(p.rocks_cache_bytes, 8 * MB);
        assert_eq!(p.catchup_batch, 4096);
        assert_eq!(p.catchup_interval_secs, 1);
        assert_eq!(p.catchup_votes_max, 1_000);
    }

    #[test]
    fn light_defaults_roles_off() {
        assert!(!MemProfile::light().role_default(true));
        assert!(MemProfile::full().role_default(true));
        assert!(!MemProfile::full().role_default(false));
    }

    #[test]
    fn parses_meminfo() {
        let s = "MemTotal:         927540 kB\nMemFree:  1 kB\n";
        assert_eq!(parse_memtotal(s), Some(927540 * 1024));
        assert_eq!(parse_memtotal("garbage"), None);
    }
}
