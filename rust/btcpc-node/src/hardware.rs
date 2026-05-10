//! Hardware fingerprinting — GPU serial detection and stable machine identity.
//! Used for anti-sybil: one physical machine = one BTCPC account.

use sha2::{Digest, Sha256};

#[allow(dead_code)]
pub struct HardwareInfo {
    /// e.g. "nvidia:ABC123" or "amd:XYZ456"
    pub gpu_serial: Option<String>,
    /// /etc/machine-id on Linux, registry GUID on Windows
    pub machine_id: Option<String>,
    /// SHA-256(gpu_serial | machine_id) — stable per physical machine
    pub fingerprint: String,
    /// Human-readable summary for display
    pub summary: String,
}

pub fn detect() -> HardwareInfo {
    let gpu_serial = detect_gpu_serial();
    let machine_id = detect_machine_id();

    let fingerprint = compute_fingerprint(gpu_serial.as_deref(), machine_id.as_deref());

    let summary = {
        let mut parts = Vec::new();
        if let Some(ref g) = gpu_serial  { parts.push(g.clone()); }
        if let Some(ref m) = machine_id  { parts.push(format!("machine:{}", &m[..8.min(m.len())])); }
        if parts.is_empty() { "unknown".to_owned() } else { parts.join(" ") }
    };

    HardwareInfo { gpu_serial, machine_id, fingerprint, summary }
}

fn compute_fingerprint(gpu: Option<&str>, machine: Option<&str>) -> String {
    let mut h = Sha256::new();
    if let Some(g) = gpu     { h.update(g.as_bytes()); }
    if let Some(m) = machine { h.update(m.as_bytes()); }
    let result = h.finalize();
    if result.iter().all(|b| *b == 0) {
        // nothing was hashed — no identifiers found
        return String::new();
    }
    hex::encode(result)
}

fn detect_gpu_serial() -> Option<String> {
    // ── NVIDIA ────────────────────────────────────────────────────────────────
    if let Ok(out) = std::process::Command::new("nvidia-smi")
        .args(["--query-gpu=serial", "--format=csv,noheader,nounits"])
        .output()
    {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout);
            // nvidia-smi may return multiple lines for multi-GPU; take first
            if let Some(line) = s.lines().next() {
                let line = line.trim();
                if !line.is_empty() && !line.eq_ignore_ascii_case("n/a") {
                    return Some(format!("nvidia:{}", line));
                }
            }
        }
    }

    // ── AMD via sysfs (Linux) ─────────────────────────────────────────────────
    #[cfg(target_os = "linux")]
    for entry in std::fs::read_dir("/sys/class/drm").into_iter().flatten().flatten() {
        let path = entry.path().join("device/serial_number");
        if let Ok(s) = std::fs::read_to_string(&path) {
            let s = s.trim().to_owned();
            if !s.is_empty() && s != "0" && s != "N/A" {
                return Some(format!("amd:{}", s));
            }
        }
    }

    // ── AMD/Intel via lspci (Linux fallback) ─────────────────────────────────
    #[cfg(target_os = "linux")]
    if let Ok(out) = std::process::Command::new("lspci")
        .args(["-v", "-mm"])
        .output()
    {
        let text = String::from_utf8_lossy(&out.stdout);
        for block in text.split("\n\n") {
            if block.contains("VGA") || block.contains("3D") || block.contains("Display") {
                for line in block.lines() {
                    if line.starts_with("Serial:") {
                        let s = line.trim_start_matches("Serial:").trim();
                        if !s.is_empty() {
                            return Some(format!("pci:{}", s));
                        }
                    }
                }
            }
        }
    }

    None
}

fn detect_machine_id() -> Option<String> {
    // Linux / WSL
    #[cfg(target_os = "linux")]
    {
        if let Ok(s) = std::fs::read_to_string("/etc/machine-id") {
            let s = s.trim().to_owned();
            if !s.is_empty() { return Some(s); }
        }
        // Some distros use /var/lib/dbus/machine-id
        if let Ok(s) = std::fs::read_to_string("/var/lib/dbus/machine-id") {
            let s = s.trim().to_owned();
            if !s.is_empty() { return Some(s); }
        }
    }

    // macOS
    #[cfg(target_os = "macos")]
    if let Ok(out) = std::process::Command::new("ioreg")
        .args(["-rd1", "-c", "IOPlatformExpertDevice"])
        .output()
    {
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            if line.contains("IOPlatformUUID") {
                if let Some(uuid) = line.split('"').nth(3) {
                    return Some(uuid.to_owned());
                }
            }
        }
    }

    // Windows
    #[cfg(target_os = "windows")]
    if let Ok(out) = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command",
            "(Get-ItemProperty 'HKLM:\\SOFTWARE\\Microsoft\\Cryptography').MachineGuid"])
        .output()
    {
        let s = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        if !s.is_empty() { return Some(s); }
    }

    None
}
