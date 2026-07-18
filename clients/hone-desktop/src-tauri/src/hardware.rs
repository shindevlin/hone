use serde_json::{json, Value};
use sysinfo::System;

pub fn detect() -> Value {
    let mut sys = System::new_all();
    sys.refresh_all();

    let cpu_count = sys.cpus().len();
    let cpu_brand = sys.cpus().first().map(|c| c.brand().to_string()).unwrap_or_default();
    let total_memory_gb = sys.total_memory() / (1024 * 1024 * 1024);
    let os = System::long_os_version().unwrap_or_else(|| "Unknown".to_string());
    let arch = std::env::consts::ARCH.to_string();

    // GPU detection — basic, just check if nvidia-smi or rocm-smi exists
    let has_nvidia = which::which("nvidia-smi").is_ok();
    let has_amd = which::which("rocm-smi").is_ok();

    // Recommend mode based on hardware
    let recommended = if has_nvidia || has_amd {
        "miner"
    } else if cpu_count >= 4 && total_memory_gb >= 8 {
        "miner"
    } else {
        "clock"
    };

    json!({
        "os": os,
        "arch": arch,
        "cpu_count": cpu_count,
        "cpu_brand": cpu_brand,
        "memory_gb": total_memory_gb,
        "has_nvidia": has_nvidia,
        "has_amd": has_amd,
        "recommended_mode": recommended,
    })
}
