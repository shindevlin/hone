use std::time::Duration;

pub struct NodeCapabilities {
    pub has_ollama: bool,
    pub has_gnss:   bool,
    pub has_gpu:    bool,
    pub disk_gb:    u64,
}

pub async fn probe(data_dir: &std::path::Path) -> NodeCapabilities {
    let (has_ollama, has_gnss, has_gpu, disk_gb) = tokio::join!(
        probe_ollama(),
        probe_gnss(),
        async { probe_gpu() },
        probe_disk(data_dir),
    );
    tracing::info!(
        "hardware probe — ollama={} gnss={} gpu={} disk={}GB",
        has_ollama, has_gnss, has_gpu, disk_gb
    );
    NodeCapabilities { has_ollama, has_gnss, has_gpu, disk_gb }
}

async fn probe_ollama() -> bool {
    let url = std::env::var("OLLAMA_URL")
        .unwrap_or_else(|_| "http://localhost:11434".to_owned());
    reqwest::Client::new()
        .get(format!("{}/api/tags", url))
        .timeout(Duration::from_secs(3))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

async fn probe_gnss() -> bool {
    let ip = match std::env::var("HONE_GNSS_DEVICE_IP") {
        Ok(ip) if !ip.is_empty() => ip,
        _ => return false,
    };
    tokio::process::Command::new("ping")
        .args(["-c", "1", "-W", "2", &ip])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

fn probe_gpu() -> bool {
    std::path::Path::new("/dev/nvidia0").exists()
        || std::process::Command::new("nvidia-smi")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
}

async fn probe_disk(data_dir: &std::path::Path) -> u64 {
    let path = data_dir.to_string_lossy().into_owned();
    match tokio::process::Command::new("df")
        .args(["-BG", "--output=avail", &path])
        .output()
        .await
    {
        Ok(out) => String::from_utf8_lossy(&out.stdout)
            .lines()
            .nth(1)
            .and_then(|l| l.trim().trim_end_matches('G').parse::<u64>().ok())
            .unwrap_or(0),
        Err(_) => 0,
    }
}
