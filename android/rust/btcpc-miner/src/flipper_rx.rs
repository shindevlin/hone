//! flipper_rx.rs — receive signed sensor frames from a paired Flipper Zero.
//!
//! Ported from `rust/btcpc-android/src/flipper_rx.rs` (the in-process P2P
//! micronode's Flipper receiver). btcpc-miner has no `Chain`/`net` — it talks
//! to the chain over HTTP via reqwest — so the parse + verify + payload-mapping
//! logic below is a faithful copy of the reference, but the downstream is a
//! signed HTTP POST to `{api_base}/api/sensor/commit` (the node's canonical
//! sensor-submission endpoint) instead of a local `Chain::apply_entry` + gossip
//! broadcast.
//!
//! This module:
//!   1. parses the raw BLE bytes into a frame (magic + header + payload),
//!   2. verifies the ed25519 device signature against the Flipper's registered
//!      public key (the anti-spoof boundary — a third party cannot inject fake
//!      Flipper data into someone else's relay),
//!   3. maps the payload to a phone-owned sensor reading, signs it with the
//!      owner's posting key (matching the JSON body + canonical signing
//!      message the Java NativeSensorService already produces), and POSTs it.
//!
//! Wire format (must match clients/btcpc-flipper/protocol/btcpc_protocol.h,
//! all little-endian, header is __attribute__((packed))):
//!   [0..4]   magic "BTPC"
//!   [4]      msg_type (u8)
//!   [5..7]   payload_len (u16 LE)
//!   [7..71]  sig (64-byte ed25519 over payload bytes)
//!   [71..]   payload
//!
//! Shin Devlin — btcpc.network

use ed25519_dalek::{Signer, SigningKey, Signature, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};

const FRAME_MAGIC: [u8; 4] = *b"BTPC";
const SIG_LEN: usize = 64;
const HEADER_LEN: usize = 4 + 1 + 2 + SIG_LEN; // = 71

// Message types (from btcpc_protocol.h). Only Flipper->phone types handled.
const MSG_SUBGHZ_OBS: u8 = 0x01;
const MSG_RFID_SCAN: u8 = 0x02;
const MSG_NFC_SCAN: u8 = 0x03;
const MSG_IBUTTON: u8 = 0x04;
const MSG_HEARTBEAT: u8 = 0x05;
const MSG_IR_CAPTURE: u8 = 0x06;

/// A parsed, signature-verified frame ready to convert to a SensorReading.
struct Frame<'a> {
    msg_type: u8,
    payload: &'a [u8],
}

/// A reading derived from a verified Flipper frame, ready to POST to the chain.
pub struct SensorReading {
    /// "account/flipper-<short_pk>-<kind>"
    pub sensor_id: String,
    /// "continuous" | "event"  (a `SensorCommitBody.sensor_type` class)
    pub sensor_type: String,
    /// Representative numeric value for the node's cross-validation consensus.
    pub primary_value: f64,
    /// Raw per-sensor values as JSON, kept for the off-chain aggregation layer
    /// (VeraSens). Not part of the on-chain commit, which stores only the
    /// batch_hash + representative value.
    #[allow(dead_code)]
    pub values_json: String,
    /// Unit of `primary_value` (e.g. "dBm", "count"), for the aggregation layer.
    #[allow(dead_code)]
    pub unit: String,
    pub owner: String,
    /// SHA-256 over the raw signed frame payload (see `payload_data_hash`) —
    /// the phone commits to exactly what the Flipper signed. Sent as the
    /// commit's `batch_hash`.
    pub data_hash: String,
}

/// Parse and verify a raw BLE buffer against the Flipper's public key.
/// Returns None (and logs) on any malformed frame, bad magic, length mismatch,
/// or signature failure — the caller drops it silently, which is the anti-spoof
/// behaviour we want.
fn parse_and_verify<'a>(buf: &'a [u8], flipper_pubkey: &VerifyingKey) -> Option<Frame<'a>> {
    if buf.len() < HEADER_LEN {
        log::warn!("flipper_rx: frame too short ({} < {})", buf.len(), HEADER_LEN);
        return None;
    }
    if buf[0..4] != FRAME_MAGIC {
        log::warn!("flipper_rx: bad magic");
        return None;
    }
    let msg_type = buf[4];
    let payload_len = u16::from_le_bytes([buf[5], buf[6]]) as usize;
    let sig_bytes = &buf[7..7 + SIG_LEN];

    let payload_start = HEADER_LEN;
    let payload_end = payload_start.checked_add(payload_len)?;
    if buf.len() < payload_end {
        log::warn!(
            "flipper_rx: payload_len {} exceeds buffer ({} available)",
            payload_len,
            buf.len() - payload_start
        );
        return None;
    }
    let payload = &buf[payload_start..payload_end];

    // The Flipper signs the payload bytes only (see frame_sign in the C).
    let sig_arr: [u8; SIG_LEN] = sig_bytes.try_into().ok()?;
    let signature = Signature::from_bytes(&sig_arr);
    if flipper_pubkey.verify(payload, &signature).is_err() {
        log::warn!("flipper_rx: device signature verification FAILED — dropping");
        return None;
    }

    Some(Frame { msg_type, payload })
}

/// Deterministic data_hash over the raw payload bytes. The phone commits to
/// exactly what the Flipper sent (which is what the device signed), so the
/// hash is reproducible and tamper-evident.
fn payload_data_hash(msg_type: u8, payload: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update([msg_type]);
    h.update(payload);
    hex::encode(h.finalize())
}

/// Map a verified frame to a phone SensorReading owned by `owner`.
/// Returns None for message types that are not sensor readings (e.g. IR
/// capture is not yet a rewarded reading type; heartbeat is housekeeping).
fn frame_to_reading(frame: &Frame, owner: &str, flipper_pk_hex: &str) -> Option<SensorReading> {
    // sensor_id ties the reading to this specific Flipper device so the chain /
    // aggregation layer can group by physical device.
    let short_id = &flipper_pk_hex[..flipper_pk_hex.len().min(16)];

    let (sensor_type, primary_value, values_json, unit, kind_tag) = match frame.msg_type {
        MSG_SUBGHZ_OBS => {
            // BtcpcSubGhzObs: freq_hz(u32 LE), rssi_dbm(i8), modulation(u8), bandwidth(u8)
            if frame.payload.len() < 7 { return None; }
            let freq = u32::from_le_bytes(frame.payload[0..4].try_into().ok()?);
            let rssi = frame.payload[4] as i8;
            let modulation = frame.payload[5];
            let bandwidth = frame.payload[6];
            (
                "continuous",
                rssi as f64,
                format!(
                    "{{\"freq_hz\":{},\"rssi_dbm\":{},\"modulation\":{},\"bandwidth\":{}}}",
                    freq, rssi, modulation, bandwidth
                ),
                "dBm",
                "subghz",
            )
        }
        MSG_NFC_SCAN | MSG_RFID_SCAN | MSG_IBUTTON => {
            // Event-class: a distinct discovery. primary_value = 1 (a hit).
            // Keep the raw payload as hex in values_json for the aggregation layer.
            let tag = match frame.msg_type {
                MSG_NFC_SCAN => "nfc",
                MSG_RFID_SCAN => "rfid",
                _ => "ibutton",
            };
            (
                "event",
                1.0,
                format!("{{\"raw\":\"{}\"}}", hex::encode(frame.payload)),
                "count",
                tag,
            )
        }
        MSG_HEARTBEAT | MSG_IR_CAPTURE => {
            // Housekeeping / not-yet-rewarded — do not submit as a reading.
            return None;
        }
        other => {
            log::warn!("flipper_rx: unknown msg_type 0x{:02x}", other);
            return None;
        }
    };

    let data_hash = payload_data_hash(frame.msg_type, frame.payload);

    Some(SensorReading {
        sensor_id: format!("{}/flipper-{}-{}", owner, short_id, kind_tag),
        sensor_type: sensor_type.to_string(),
        primary_value,
        values_json,
        unit: unit.to_string(),
        owner: owner.to_string(),
        data_hash,
    })
}

/// Reproduces `canonical_signing_message`'s `SensorDataCommit` arm
/// (`rust/btcpc-node/src/tx.rs`, ~line 2496) exactly: same fields, same order,
/// same `serde_json::to_string` shape. Must stay byte-for-byte identical to
/// what the chain verifies against, or signatures produced here will be
/// rejected. The phone POSTs to `/api/sensor/commit`, which builds a
/// `SensorDataCommit` entry — so this is the message that entry signs.
fn build_canonical_signing_message(
    chain_id: &str,
    sensor_id: &str,
    owner: &str,
    batch_hash: &str,
    reading_count: u64,
    sensor_type: &str,
    signed_by: &str,
) -> String {
    let msg = serde_json::json!({
        "chain_id": chain_id,
        "type": "SENSOR_DATA_COMMIT",
        "sensor_id": sensor_id,
        "owner": owner,
        "batch_hash": batch_hash,
        "reading_count": reading_count,
        "sensor_type": sensor_type,
        "signed_by": signed_by,
    });
    serde_json::to_string(&msg).expect("canonical signing message is always valid JSON")
}

/// Sign `message` with a hex-encoded 32-byte ed25519 posting-key seed,
/// returning the hex-encoded 64-byte signature.
fn sign_with_posting_key(posting_key_hex: &str, message: &str) -> Option<String> {
    let bytes = hex::decode(posting_key_hex.trim()).ok()?;
    let seed: [u8; 32] = bytes.try_into().ok()?;
    let signing_key = SigningKey::from_bytes(&seed);
    let signature = signing_key.sign(message.as_bytes());
    Some(hex::encode(signature.to_bytes()))
}

/// POST a verified reading to `{api_base}/api/sensor/commit`.
///
/// This is the canonical sensor-submission endpoint on the node
/// (`post_sensor_commit` in `rust/btcpc-node/src/api.rs`). It builds a
/// `SensorDataCommit` ledger entry, runs ASN-diversity + 2-of-N consensus, and
/// requires a posting-key signature bound to `owner` (`signed_by == owner`).
///
/// The body is `SensorCommitBody`: `sensor_id`, `owner`, `batch_hash`,
/// `reading_count`, `sensor_type`, `value`, `signature`. We commit to exactly
/// what the Flipper signed by using the payload `data_hash` as `batch_hash`,
/// with `reading_count = 1` (one frame = one reading). The signature covers the
/// canonical `SENSOR_DATA_COMMIT` message the node verifies against.
///
/// `sensor_type` must be one of the node's accepted classes
/// ("continuous" | "event" | "sampled" | "pulse"); `frame_to_reading` only ever
/// yields "continuous" or "event".
async fn submit_reading(
    client: &reqwest::Client,
    api_base: &str,
    chain_id: &str,
    reading: SensorReading,
    posting_key: &str,
) -> anyhow::Result<()> {
    let batch_hash = reading.data_hash.clone();
    let reading_count: u64 = 1;
    let signed_by = reading.owner.clone();

    let mut body = serde_json::json!({
        "sensor_id": reading.sensor_id,
        "owner": reading.owner,
        "batch_hash": batch_hash,
        "reading_count": reading_count,
        "sensor_type": reading.sensor_type,
        "value": reading.primary_value,
    });

    if !posting_key.is_empty() {
        let message = build_canonical_signing_message(
            chain_id,
            &reading.sensor_id,
            &reading.owner,
            &batch_hash,
            reading_count,
            &reading.sensor_type,
            &signed_by,
        );
        if let Some(sig) = sign_with_posting_key(posting_key, &message) {
            body["signature"] = serde_json::Value::String(sig);
        }
    }

    let url = format!("{}/api/sensor/commit", api_base.trim_end_matches('/'));

    let resp = client.post(&url).json(&body).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("submit_reading: HTTP {}", resp.status());
    }
    Ok(())
}

/// Handle one raw BLE frame received from the paired Flipper.
///
/// `flipper_pk_hex` is the Flipper's registered device public key (64 hex
/// chars). `owner` is this phone's account; the resulting reading is
/// attributed to and signed by the owner's posting key. This is the async
/// worker the JNI `nativeIngestFrame` callback spawns onto the shared tokio
/// runtime.
pub async fn handle_ble_frame(
    buf: Vec<u8>,
    flipper_pk_hex: String,
    owner: String,
    posting_key: String,
    api_base: String,
    chain_id: String,
    client: reqwest::Client,
) {
    let pk_bytes = match hex::decode(flipper_pk_hex.trim()) {
        Ok(b) => b,
        Err(_) => { log::warn!("flipper_rx: flipper pubkey not valid hex"); return; }
    };
    let pk_arr: [u8; 32] = match pk_bytes.try_into() {
        Ok(a) => a,
        Err(_) => { log::warn!("flipper_rx: flipper pubkey must be 32 bytes"); return; }
    };
    let flipper_pubkey = match VerifyingKey::from_bytes(&pk_arr) {
        Ok(k) => k,
        Err(e) => { log::warn!("flipper_rx: bad flipper pubkey: {}", e); return; }
    };

    let frame = match parse_and_verify(&buf, &flipper_pubkey) {
        Some(f) => f,
        None => return, // already logged; drop silently (anti-spoof)
    };

    let reading = match frame_to_reading(&frame, &owner, &flipper_pk_hex) {
        Some(r) => r,
        None => return, // housekeeping / non-reading frame
    };

    log::info!(
        "flipper_rx: verified {} reading from device {} -> submitting",
        reading.sensor_type,
        flipper_pk_hex.get(..8).unwrap_or("")
    );

    if let Err(e) = submit_reading(&client, &api_base, &chain_id, reading, &posting_key).await {
        log::warn!("flipper_rx: submit failed: {}", e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::Signer;

    /// Build a valid signed frame the way the Flipper would.
    fn make_frame(sk: &ed25519_dalek::SigningKey, msg_type: u8, payload: &[u8]) -> Vec<u8> {
        let sig = sk.sign(payload);
        let mut buf = Vec::with_capacity(HEADER_LEN + payload.len());
        buf.extend_from_slice(&FRAME_MAGIC);
        buf.push(msg_type);
        buf.extend_from_slice(&(payload.len() as u16).to_le_bytes());
        buf.extend_from_slice(&sig.to_bytes());
        buf.extend_from_slice(payload);
        buf
    }

    fn subghz_payload(freq: u32, rssi: i8) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&freq.to_le_bytes());
        p.push(rssi as u8);
        p.push(2); // modulation OOK
        p.push(0); // bandwidth
        p
    }

    #[test]
    fn valid_frame_parses_and_verifies() {
        let sk = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
        let pk = sk.verifying_key();
        let payload = subghz_payload(433_920_000, -72);
        let frame = make_frame(&sk, MSG_SUBGHZ_OBS, &payload);

        let parsed = parse_and_verify(&frame, &pk).expect("valid frame must parse");
        assert_eq!(parsed.msg_type, MSG_SUBGHZ_OBS);
        assert_eq!(parsed.payload, &payload[..]);
    }

    #[test]
    fn wrong_key_signature_rejected() {
        let flipper_sk = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
        let attacker_pk = ed25519_dalek::SigningKey::from_bytes(&[9u8; 32]).verifying_key();
        let payload = subghz_payload(433_920_000, -72);
        let frame = make_frame(&flipper_sk, MSG_SUBGHZ_OBS, &payload);
        // Verify against the WRONG key — must fail (anti-spoof).
        assert!(parse_and_verify(&frame, &attacker_pk).is_none());
    }

    #[test]
    fn tampered_payload_rejected() {
        let sk = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
        let pk = sk.verifying_key();
        let payload = subghz_payload(433_920_000, -72);
        let mut frame = make_frame(&sk, MSG_SUBGHZ_OBS, &payload);
        // Flip a byte in the payload after signing.
        let last = frame.len() - 1;
        frame[last] ^= 0xFF;
        assert!(parse_and_verify(&frame, &pk).is_none());
    }

    #[test]
    fn bad_magic_rejected() {
        let sk = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
        let pk = sk.verifying_key();
        let payload = subghz_payload(433_920_000, -72);
        let mut frame = make_frame(&sk, MSG_SUBGHZ_OBS, &payload);
        frame[0] = b'X';
        assert!(parse_and_verify(&frame, &pk).is_none());
    }

    #[test]
    fn truncated_frame_rejected() {
        let sk = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
        let pk = sk.verifying_key();
        let payload = subghz_payload(433_920_000, -72);
        let frame = make_frame(&sk, MSG_SUBGHZ_OBS, &payload);
        assert!(parse_and_verify(&frame[..HEADER_LEN - 1], &pk).is_none());
    }

    #[test]
    fn subghz_frame_maps_to_continuous_reading() {
        let sk = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
        let pk_hex = hex::encode(sk.verifying_key().to_bytes());
        let payload = subghz_payload(433_920_000, -55);
        let frame = Frame { msg_type: MSG_SUBGHZ_OBS, payload: &payload };
        let r = frame_to_reading(&frame, "alice", &pk_hex).expect("subghz maps to a reading");
        assert_eq!(r.sensor_type, "continuous");
        assert_eq!(r.primary_value, -55.0);
        assert_eq!(r.owner, "alice");
        assert!(r.sensor_id.starts_with("alice/flipper-"));
        assert!(r.sensor_id.ends_with("-subghz"));
    }

    #[test]
    fn nfc_frame_maps_to_event_reading() {
        let uid = [0x04u8, 0x1a, 0x2b, 0x3c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        let mut payload = vec![0u8]; // tech = A
        payload.extend_from_slice(&uid);
        payload.push(4); // uid_len
        payload.extend_from_slice(&[0x00, 0x44]); // atqa
        payload.push(0x08); // sak
        let sk = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
        let pk_hex = hex::encode(sk.verifying_key().to_bytes());
        let frame = Frame { msg_type: MSG_NFC_SCAN, payload: &payload };
        let r = frame_to_reading(&frame, "bob", &pk_hex).expect("nfc maps to a reading");
        assert_eq!(r.sensor_type, "event");
        assert_eq!(r.primary_value, 1.0);
        assert!(r.sensor_id.ends_with("-nfc"));
    }

    #[test]
    fn heartbeat_frame_is_not_a_reading() {
        let sk = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
        let pk_hex = hex::encode(sk.verifying_key().to_bytes());
        let payload = vec![0u8; 13];
        let frame = Frame { msg_type: MSG_HEARTBEAT, payload: &payload };
        assert!(frame_to_reading(&frame, "carol", &pk_hex).is_none());
    }

    #[test]
    fn canonical_signing_message_matches_expected_shape() {
        // Must match the node's `SensorDataCommit` arm of
        // `canonical_signing_message` (rust/btcpc-node/src/tx.rs ~line 2496).
        let msg = build_canonical_signing_message(
            "btcpc-satoshi", "alice/flipper-abc-subghz", "alice",
            &"ab".repeat(32), 1, "continuous", "alice",
        );
        let parsed: serde_json::Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(parsed["chain_id"], "btcpc-satoshi");
        assert_eq!(parsed["type"], "SENSOR_DATA_COMMIT");
        assert_eq!(parsed["sensor_id"], "alice/flipper-abc-subghz");
        assert_eq!(parsed["owner"], "alice");
        assert_eq!(parsed["batch_hash"], "ab".repeat(32));
        assert_eq!(parsed["reading_count"], 1);
        assert_eq!(parsed["sensor_type"], "continuous");
        assert_eq!(parsed["signed_by"], "alice");
        // Server-set fields deliberately excluded.
        assert!(parsed.get("epoch").is_none());
        assert!(parsed.get("value").is_none());
        assert!(parsed.get("metadata").is_none());
    }

    #[test]
    fn signed_reading_verifies_against_canonical_message() {
        let posting_key_seed = [b'a'; 32];
        let signing_key = SigningKey::from_bytes(&posting_key_seed);
        let posting_key_hex = hex::encode(posting_key_seed);

        let message = build_canonical_signing_message(
            "btcpc-satoshi", "alice/flipper-abc-subghz", "alice",
            &"ab".repeat(32), 1, "continuous", "alice",
        );
        let sig_hex = sign_with_posting_key(&posting_key_hex, &message)
            .expect("posting key must be valid hex and produce a signature");

        let verifying_key = signing_key.verifying_key();
        let sig_bytes = hex::decode(&sig_hex).unwrap();
        let sig_array: [u8; 64] = sig_bytes.try_into().unwrap();
        let signature = Signature::from_bytes(&sig_array);
        assert!(verifying_key.verify(message.as_bytes(), &signature).is_ok());
    }
}
