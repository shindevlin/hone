//! Auto-updater for the btcpc-node binary.
//!
//! Polls BTCPC_UPDATE_URL every 6h, verifies SHA-256 against X-Binary-Hash header,
//! writes to ~/.btcpc/btcpc-node.new, chmod +x, then renames to replace the running
//! binary on next restart. Only active when BTCPC_AUTO_UPDATE=1.

use std::path::PathBuf;
use std::time::Duration;
use sha2::{Digest, Sha256};
use tracing::{info, warn};

pub async fn run(data_dir: PathBuf) {
    if std::env::var("BTCPC_AUTO_UPDATE").map(|v| v == "1" || v == "true").unwrap_or(false) {
        info!("[updater] auto-update enabled");
    } else {
        return;
    }

    let update_url = match std::env::var("BTCPC_UPDATE_URL") {
        Ok(u) => u,
        Err(_) => {
            warn!("[updater] BTCPC_UPDATE_URL not set — auto-update disabled");
            return;
        }
    };

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .unwrap_or_default();

    let current_exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => { warn!("[updater] can't find current exe: {}", e); return; }
    };

    loop {
        tokio::time::sleep(Duration::from_secs(6 * 3600)).await;
        attempt_update(&http, &update_url, &current_exe, &data_dir).await;
    }
}

async fn attempt_update(
    http: &reqwest::Client,
    url: &str,
    current_exe: &std::path::Path,
    data_dir: &std::path::Path,
) {
    info!("[updater] checking for update at {}", url);

    let response = match http.get(url).send().await {
        Ok(r) => r,
        Err(e) => { warn!("[updater] fetch failed: {}", e); return; }
    };

    if !response.status().is_success() {
        warn!("[updater] server returned {}", response.status());
        return;
    }

    let expected_hash = response.headers()
        .get("x-binary-hash")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);

    let bytes = match response.bytes().await {
        Ok(b) => b,
        Err(e) => { warn!("[updater] read failed: {}", e); return; }
    };

    // Verify SHA-256 if header was present.
    if let Some(expected) = &expected_hash {
        let actual = hex::encode(Sha256::digest(&bytes));
        if actual != *expected {
            warn!("[updater] hash mismatch: got {} expected {}", actual, expected);
            return;
        }
    }

    // Check if this is actually different from the running binary.
    let current_hash = current_exe_hash(current_exe);
    let new_hash = hex::encode(Sha256::digest(&bytes));
    if current_hash.as_deref() == Some(new_hash.as_str()) {
        info!("[updater] already up to date ({})", &new_hash[..16]);
        return;
    }

    // Write to .new, make executable, rename.
    let new_path = data_dir.join("btcpc-node.new");
    if let Err(e) = std::fs::write(&new_path, &bytes) {
        warn!("[updater] write failed: {}", e);
        return;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&new_path, std::fs::Permissions::from_mode(0o755));
    }

    // Atomic rename over the running binary.
    if let Err(e) = std::fs::rename(&new_path, current_exe) {
        warn!("[updater] rename failed: {}", e);
        return;
    }

    info!("[updater] updated to {} — will take effect on next restart", &new_hash[..16]);
}

fn current_exe_hash(path: &std::path::Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    Some(hex::encode(Sha256::digest(&bytes)))
}
