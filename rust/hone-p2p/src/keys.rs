/// Persistent keypair storage — stable PeerId across restarts.
/// Stored at $HONE_DATA_DIR/p2p-identity.json (ed25519 private key, hex-encoded).
/// Compatible with existing ~/.hone/noise-static-key.json convention.

use anyhow::{Context, Result};
use libp2p::identity::{self, Keypair};
use std::fs;
use std::path::Path;
use tracing::info;

pub fn load_or_create(data_dir: &str) -> Result<Keypair> {
    let path = Path::new(data_dir).join("p2p-identity.json");

    if path.exists() {
        let contents = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read keypair from {}", path.display()))?;
        let stored: StoredKey = serde_json::from_str(&contents)
            .with_context(|| "Failed to parse keypair JSON")?;
        let bytes = hex::decode(&stored.private_key_hex)
            .with_context(|| "Failed to decode keypair hex")?;
        let keypair = Keypair::ed25519_from_bytes(bytes)
            .with_context(|| "Failed to restore keypair")?;
        info!("Loaded existing P2P identity: {}", keypair.public().to_peer_id());
        return Ok(keypair);
    }

    // First start — generate and persist
    let keypair = identity::Keypair::generate_ed25519();
    let peer_id = keypair.public().to_peer_id();

    let private_bytes = keypair.try_into_ed25519()
        .map_err(|_| anyhow::anyhow!("Not an ed25519 keypair"))?
        .secret()
        .as_ref()
        .to_vec();

    let stored = StoredKey {
        private_key_hex: hex::encode(&private_bytes),
        peer_id: peer_id.to_string(),
    };

    fs::create_dir_all(data_dir)
        .with_context(|| format!("Failed to create data dir {}", data_dir))?;

    // Write with restricted permissions (0600)
    let json = serde_json::to_string_pretty(&stored)?;
    fs::write(&path, &json)
        .with_context(|| format!("Failed to write keypair to {}", path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }

    info!("Generated new P2P identity: {} (saved to {})", peer_id, path.display());

    // Restore keypair from saved bytes for consistent return
    let keypair = Keypair::ed25519_from_bytes(private_bytes)
        .with_context(|| "Failed to restore generated keypair")?;

    Ok(keypair)
}

#[derive(serde::Serialize, serde::Deserialize)]
struct StoredKey {
    private_key_hex: String,
    peer_id: String,
}
