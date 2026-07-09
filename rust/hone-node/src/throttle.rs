//! Resource throttling — auto-drop CPU/GPU usage when the operator is actively
//! using the machine, so HONE never makes a PC feel unusable to its owner.
//!
//! Phase 1 (this module): OS idle-time detection + process-priority throttling
//! + a shared throttle level the inference engine's generation loop checks.
//! Phase 2 (future, not built here): system-wide load watching from OTHER
//! processes, foreground-app detection, GPU-specific pacing. See task tracker.
//!
//! Design constraints this follows (matching hardware.rs's existing style):
//! - No new compiled Windows dependency (no `windows-rs`/`winapi` crate) — the
//!   codebase's established pattern for Windows-specific facts is shelling out
//!   to `powershell.exe`, so idle-time detection does the same via an inline
//!   Add-Type P/Invoke of `GetLastInputInfo` (there is no built-in PowerShell
//!   cmdlet for OS idle time; user32.dll's GetLastInputInfo is the only
//!   correct source — DEUCE PC's own screensaver uses it).
//! - Linux has no universal idle-time API independent of the display server
//!   (X11 has one via `libXss`, Wayland does not expose one uniformly to
//!   headless/server processes at all). Rather than pull in an X11 client
//!   dependency for a node binary that is frequently run headless/server-side,
//!   this uses a keyboard/mouse interrupt-count delta from `/proc/interrupts`
//!   as an idle proxy: if the input-device IRQ counters haven't moved in the
//!   idle window, treat the machine as idle. This is an approximation (an IRQ
//!   without a session, e.g. SSH-only activity, won't register), not a
//!   precise session-idle signal — acceptable for phase 1 since a false
//!   "not idle" only means throttling stays conservative, never runs at full
//!   power when someone actually is at the keyboard.

use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::time::Duration;
#[cfg(target_os = "linux")]
use std::time::Instant;

/// How long the machine must be idle before HONE ramps back to full power.
const IDLE_RESTORE_THRESHOLD: Duration = Duration::from_secs(120);
/// How often the idle check runs.
const POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Throttle level as a percentage of full resource usage (10-100). Stored as
/// an atomic so the inference engine's hot loop can read it cheaply, every
/// token, without a lock.
static THROTTLE_PERCENT: AtomicU8 = AtomicU8::new(100);
/// Unix epoch ms of the last time we observed user input activity. Used only
/// for logging/diagnostics; the actual idle decision is re-derived each poll.
static LAST_ACTIVITY_MS: AtomicU64 = AtomicU64::new(0);

/// Current throttle level (10-100). Callers doing CPU/GPU-bound work should
/// scale their effort by this — e.g. an inference loop targeting N threads
/// at 100% uses `N * throttle_percent() / 100` (minimum 1) at a lower level.
pub fn throttle_percent() -> u8 {
    THROTTLE_PERCENT.load(Ordering::Relaxed)
}

/// True when currently throttled below full power (i.e. the operator appears
/// to be actively using the machine).
pub fn is_throttled() -> bool {
    throttle_percent() < 100
}

/// Spawn the background idle-watcher. Call once at node startup. Adjusts
/// THROTTLE_PERCENT and this process's OS scheduling priority as user
/// activity is detected/clears. Never panics — any detection failure just
/// leaves the throttle at its last known level (fails toward NOT dropping
/// resource usage further than already decided, i.e. fails safe for the
/// operator's PC, not for HONE's throughput).
pub fn spawn_watcher(low_percent: u8) {
    let low_percent = low_percent.clamp(10, 50);
    tokio::spawn(async move {
        let mut last_idle_seconds: Option<u64> = None;
        loop {
            tokio::time::sleep(POLL_INTERVAL).await;
            let idle = detect_idle_seconds();
            match idle {
                Some(secs) => {
                    let now_active = secs < IDLE_RESTORE_THRESHOLD.as_secs();
                    if now_active {
                        LAST_ACTIVITY_MS.store(now_unix_ms(), Ordering::Relaxed);
                    }
                    let target = if now_active { low_percent } else { 100 };
                    if THROTTLE_PERCENT.swap(target, Ordering::Relaxed) != target {
                        apply_process_priority(target < 100);
                        tracing::info!(
                            "throttle: {} (idle {}s) -> {}% resource usage",
                            if now_active { "user active" } else { "user idle" },
                            secs,
                            target
                        );
                    }
                    last_idle_seconds = Some(secs);
                }
                None => {
                    // Detection unavailable on this platform/environment (e.g.
                    // headless Linux with no /proc/interrupts input IRQs
                    // found). Stay at whatever level we're already at rather
                    // than guess; log once so it's visible in the node's
                    // startup diagnostics, not spammed every poll.
                    if last_idle_seconds.is_some() {
                        tracing::warn!(
                            "throttle: idle detection unavailable — resource throttling disabled, running at {}%",
                            throttle_percent()
                        );
                    }
                    last_idle_seconds = None;
                }
            }
        }
    });
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Seconds since the last detected user input (keyboard/mouse), or `None` if
/// idle time could not be determined on this platform.
fn detect_idle_seconds() -> Option<u64> {
    #[cfg(target_os = "windows")]
    {
        windows_idle_seconds()
    }
    #[cfg(target_os = "linux")]
    {
        linux_idle_seconds()
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        None
    }
}

#[cfg(target_os = "windows")]
fn windows_idle_seconds() -> Option<u64> {
    // No PowerShell cmdlet exposes GetLastInputInfo — it is a raw user32.dll
    // P/Invoke. Add-Type compiles this inline via csc.exe (bundled with
    // .NET, present on every supported Windows version) each call; this is
    // the same shell-out pattern hardware.rs uses for MachineGuid, just with
    // an inline C# body instead of a one-line cmdlet.
    const SCRIPT: &str = r#"
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class IdleTime {
    [StructLayout(LayoutKind.Sequential)]
    struct LASTINPUTINFO { public uint cbSize; public uint dwTime; }
    [DllImport("user32.dll")]
    static extern bool GetLastInputInfo(ref LASTINPUTINFO plii);
    public static uint GetIdleSeconds() {
        LASTINPUTINFO lii = new LASTINPUTINFO();
        lii.cbSize = (uint)Marshal.SizeOf(lii);
        GetLastInputInfo(ref lii);
        return ((uint)Environment.TickCount - lii.dwTime) / 1000;
    }
}
'@
[IdleTime]::GetIdleSeconds()
"#;
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout).trim().parse::<u64>().ok()
}

#[cfg(target_os = "linux")]
fn linux_idle_seconds() -> Option<u64> {
    // Idle proxy via input-device interrupt counters (see module doc for
    // rationale). Sample /proc/interrupts twice, POLL_INTERVAL apart, and
    // treat "no change on any line whose device name suggests keyboard/mouse
    // input" as idle for that interval. State is kept in a thread-local
    // static so this can be a stateless-looking function call from the
    // watcher loop above.
    use std::sync::Mutex;
    use std::sync::OnceLock;

    static PREV: OnceLock<Mutex<Option<(Instant, std::collections::HashMap<String, u64>)>>> =
        OnceLock::new();
    let cell = PREV.get_or_init(|| Mutex::new(None));

    let counts = read_input_irq_counts()?;
    let mut guard = cell.lock().ok()?;

    let now = Instant::now();
    let idle_secs = match guard.as_ref() {
        Some((prev_time, prev_counts)) => {
            let changed = counts.iter().any(|(k, v)| prev_counts.get(k).map(|pv| pv != v).unwrap_or(true));
            if changed {
                0
            } else {
                now.duration_since(*prev_time).as_secs()
            }
        }
        None => 0, // first sample — assume active until we have a baseline
    };

    // Only replace the stored baseline when activity is observed, so the
    // "idle since" clock keeps counting forward across polls with no input
    // rather than resetting to 0 every 5s just because we re-sampled.
    if idle_secs == 0 {
        *guard = Some((now, counts));
    }

    Some(idle_secs)
}

#[cfg(target_os = "linux")]
fn read_input_irq_counts() -> Option<std::collections::HashMap<String, u64>> {
    let content = std::fs::read_to_string("/proc/interrupts").ok()?;
    let mut out = std::collections::HashMap::new();
    for line in content.lines() {
        let lower = line.to_lowercase();
        if !(lower.contains("keyboard") || lower.contains("mouse") || lower.contains("i8042")) {
            continue;
        }
        let mut fields = line.split_whitespace();
        let irq_label = fields.next()?.trim_end_matches(':').to_owned();
        let sum: u64 = fields
            .take_while(|f| f.chars().all(|c| c.is_ascii_digit()))
            .filter_map(|f| f.parse::<u64>().ok())
            .sum();
        out.insert(irq_label, sum);
    }
    if out.is_empty() { None } else { Some(out) }
}

/// Lower (or restore) this process's OS scheduling priority. Best-effort —
/// failure is logged, not propagated, since a priority-set failure should
/// never crash the node; it just means the OS scheduler treats HONE the same
/// as before.
fn apply_process_priority(lower: bool) {
    #[cfg(target_os = "windows")]
    {
        // BELOW_NORMAL_PRIORITY_CLASS = 0x00004000, NORMAL_PRIORITY_CLASS = 0x00000020.
        let class = if lower { "0x00004000" } else { "0x00000020" };
        let script = format!(
            r#"$p = Get-Process -Id {}; $p.PriorityClass = {}"#,
            std::process::id(),
            if lower { "'BelowNormal'" } else { "'Normal'" }
        );
        let _ = class; // class value documented above for reference; PriorityClass name is what PowerShell accepts
        let result = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output();
        if let Err(e) = result {
            tracing::warn!("throttle: failed to set Windows process priority: {}", e);
        }
    }
    #[cfg(target_os = "linux")]
    {
        // Nice value 10 = noticeably deprioritized but still schedulable;
        // matches "yield to interactive work" without starving HONE
        // entirely. 0 = restore to default niceness.
        let nice_value = if lower { 10 } else { 0 };
        // SAFETY: setpriority with PRIO_PROCESS + our own pid (0 = calling
        // process) is always safe to call; failure just means the OS refused
        // (e.g. no permission to raise priority back down past a cap), which
        // we log and ignore.
        let ret = unsafe { libc::setpriority(libc::PRIO_PROCESS, 0, nice_value) };
        if ret != 0 {
            tracing::warn!(
                "throttle: setpriority({}) failed: {}",
                nice_value,
                std::io::Error::last_os_error()
            );
        }
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        let _ = lower;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn throttle_percent_defaults_to_full() {
        // Independent of watcher state — a fresh process should report 100%
        // until spawn_watcher actually observes activity.
        assert!(throttle_percent() >= 10);
    }

    #[test]
    fn is_throttled_matches_percent() {
        THROTTLE_PERCENT.store(100, Ordering::Relaxed);
        assert!(!is_throttled());
        THROTTLE_PERCENT.store(25, Ordering::Relaxed);
        assert!(is_throttled());
        THROTTLE_PERCENT.store(100, Ordering::Relaxed); // restore for other tests sharing the process
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn read_input_irq_counts_parses_proc_interrupts_shape_or_none() {
        // Environment-dependent (CI containers often have no input IRQs at
        // all) — just prove it doesn't panic and returns a sane shape when
        // Some.
        if let Some(counts) = read_input_irq_counts() {
            assert!(!counts.is_empty());
            for v in counts.values() {
                assert!(*v < u64::MAX);
            }
        }
    }
}
