use anyhow::{Context, Result};
use ed25519_dalek::{SigningKey, VerifyingKey, Signer};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tracing::{info, warn};

#[derive(Serialize, Deserialize)]
struct KeyFile {
    sensor_id: String,
    /// Hex-encoded 32-byte ed25519 seed (private key material)
    private_key_seed: String,
    /// Hex-encoded SPKI DER public key (44 bytes) — compatible with Node.js crypto
    public_key_spki: String,
}

pub struct SensorKeypair {
    pub sensor_id: String,
    signing_key: SigningKey,
    pub public_key_spki_hex: String,
}

/// Construct the 44-byte SPKI DER encoding of an ed25519 public key.
/// Compatible with Node.js `crypto.createPublicKey({ key: buf, format: 'der', type: 'spki' })`.
fn to_spki_der(verifying_key: &VerifyingKey) -> Vec<u8> {
    let pk_bytes = verifying_key.as_bytes();
    let mut der = vec![
        0x30, 0x2a,              // SEQUENCE, length 42
        0x30, 0x05,              // SEQUENCE, length 5
        0x06, 0x03, 0x2b, 0x65, 0x70,  // OID 1.3.101.112 (id-Ed25519)
        0x03, 0x21, 0x00,        // BIT STRING, length 33, 0 unused bits
    ];
    der.extend_from_slice(pk_bytes);
    der
}

fn key_path(sensor_id: &str) -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let data_dir = std::env::var("HONE_DATA_DIR")
        .unwrap_or_else(|_| format!("{}/.hone", home));
    // Sanitize sensor_id for use in filename (replace / with -)
    let safe_id = sensor_id.replace('/', "-");
    PathBuf::from(data_dir).join(format!("sensor-{}.key", safe_id))
}

impl SensorKeypair {
    /// Load existing keypair from disk, or generate a new one and save it.
    pub fn load_or_generate(sensor_id: &str) -> Result<Self> {
        let path = key_path(sensor_id);

        if path.exists() {
            let raw = fs::read_to_string(&path)
                .with_context(|| format!("reading sensor keypair at {:?}", path))?;
            let kf: KeyFile = serde_json::from_str(&raw)
                .with_context(|| "parsing sensor keypair JSON")?;

            let seed_bytes = hex::decode(&kf.private_key_seed)
                .with_context(|| "decoding private key seed hex")?;
            let seed: [u8; 32] = seed_bytes.try_into()
                .map_err(|_| anyhow::anyhow!("seed must be 32 bytes"))?;

            let signing_key = SigningKey::from_bytes(&seed);

            info!("Loaded sensor keypair for {} from {:?}", sensor_id, path);
            return Ok(SensorKeypair {
                sensor_id: sensor_id.to_string(),
                signing_key,
                public_key_spki_hex: kf.public_key_spki,
            });
        }

        // Generate new keypair
        let signing_key = SigningKey::generate(&mut OsRng);
        let verifying_key = signing_key.verifying_key();
        let seed_hex = hex::encode(signing_key.to_bytes());
        let spki_der = to_spki_der(&verifying_key);
        let spki_hex = hex::encode(&spki_der);

        // Create data dir if needed
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating dir {:?}", parent))?;
        }

        let kf = KeyFile {
            sensor_id: sensor_id.to_string(),
            private_key_seed: seed_hex,
            public_key_spki: spki_hex.clone(),
        };
        fs::write(&path, serde_json::to_string_pretty(&kf)?)
            .with_context(|| format!("writing sensor keypair to {:?}", path))?;

        info!("Generated new ed25519 keypair for sensor {} — stored at {:?}", sensor_id, path);
        info!("Public key (SPKI DER): {}", spki_hex);

        Ok(SensorKeypair {
            sensor_id: sensor_id.to_string(),
            signing_key,
            public_key_spki_hex: spki_hex,
        })
    }

    /// Sign a sensor reading. Returns hex-encoded signature.
    ///
    /// Payload format (must match sensorKeystore.js `readingPayload()`):
    ///   "{sensor_id}|{value}|{device_timestamp_ms}"
    pub fn sign_reading(&self, value: impl std::fmt::Display, device_timestamp_ms: u64) -> String {
        let payload = format!("{}|{}|{}", self.sensor_id, value, device_timestamp_ms);
        let sig = self.signing_key.sign(payload.as_bytes());
        hex::encode(sig.to_bytes())
    }

    /// Print registration instructions to stdout.
    pub fn print_registration_hint(&self, api_base: &str) {
        warn!("Sensor keypair not yet registered with chain API.");
        warn!("Register it with:");
        warn!("  curl -X POST {}/api/sensors/{}/register-key \\",
            api_base,
            urlencoding::encode(&self.sensor_id)
        );
        warn!("    -H 'Authorization: Bearer <your-jwt>' \\");
        warn!("    -H 'Content-Type: application/json' \\");
        warn!("    -d '{{\"public_key\": \"{}\"}}' ", self.public_key_spki_hex);
    }
}
