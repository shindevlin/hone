//! Acoustic ownership challenge (Option A) — requires `--features acoustic`
//! and a USB microphone attached to the Pi.
//!
//! AirTags emit a 2.5kHz tone burst when triggered via the Find My app.
//! This module:
//!   1. Waits for a challenge request (serial_commitment + challenge_nonce)
//!   2. Records audio for up to CHALLENGE_WINDOW_SECS
//!   3. Runs a sliding FFT window; detects energy spike in BEEP_HZ_LO..BEEP_HZ_HI
//!   4. If a BLE AirTag advertisement appears within CORRELATION_SECS of the spike,
//!      generates a proof_hash and submits TrackerAcousticProof to the node.

use anyhow::{bail, Context};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rustfft::{num_complex::Complex, FftPlanner};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::api::NodeClient;
use crate::scanner::Sighting;

const CHALLENGE_WINDOW_SECS: u64 = 300; // 5 minutes to trigger the beep
const CORRELATION_SECS: u64 = 3;        // BLE must appear within 3s of acoustic spike
const BEEP_HZ_LO: f32 = 2000.0;
const BEEP_HZ_HI: f32 = 3500.0;
const PEAK_DB_THRESHOLD: f32 = -40.0;   // dBFS — tune for ambient noise floor
const FFT_WINDOW: usize = 4096;

#[derive(Debug, Clone)]
pub struct ChallengeRequest {
    pub serial_commitment: String,
    pub claimer: String,
    pub challenge_nonce: String,
    pub tag_type: String,
}

pub struct AcousticChallenger {
    client: NodeClient,
    observer_id: String,
    account: String,
    epoch_secs: u64,
}

impl AcousticChallenger {
    pub fn new(client: NodeClient, observer_id: String, account: String, epoch_secs: u64) -> Self {
        Self { client, observer_id, account, epoch_secs }
    }

    /// Handle a single challenge. Returns true if the challenge was proven.
    pub async fn run_challenge(
        &self,
        req: ChallengeRequest,
        sighting_rx: &mut mpsc::Receiver<Sighting>,
    ) -> anyhow::Result<bool> {
        info!("acoustic challenge started for claim {}", &req.serial_commitment[..12]);

        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .context("No audio input device — connect a USB microphone")?;
        let stream_config = device.default_input_config().context("audio config")?;
        let sample_rate = stream_config.sample_rate().0 as f32;

        // Shared ring buffer for audio samples.
        let samples: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
        let samples_w = Arc::clone(&samples);

        let stream = device.build_input_stream(
            &stream_config.into(),
            move |data: &[f32], _| {
                if let Ok(mut buf) = samples_w.lock() {
                    buf.extend_from_slice(data);
                    // Keep last ~10 seconds of audio to avoid unbounded growth.
                    let keep = (sample_rate * 10.0) as usize;
                    if buf.len() > keep {
                        let drain = buf.len() - keep;
                        buf.drain(..drain);
                    }
                }
            },
            |e| warn!("audio stream error: {}", e),
            None,
        ).context("build input stream")?;

        stream.play().context("audio stream play")?;

        let deadline = tokio::time::Instant::now()
            + std::time::Duration::from_secs(CHALLENGE_WINDOW_SECS);

        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT_WINDOW);

        let mut last_spike_ts: Option<std::time::Instant> = None;
        let mut check_interval = tokio::time::interval(std::time::Duration::from_millis(100));

        loop {
            tokio::select! {
                // Check for BLE AirTag co-event.
                Some(sighting) = sighting_rx.recv() => {
                    if matches!(sighting.tag_type, crate::classify::TagType::AirTag
                                | crate::classify::TagType::AndroidFmd
                                | crate::classify::TagType::Tile
                                | crate::classify::TagType::SamsungSmartTag) {
                        if let Some(spike_ts) = last_spike_ts {
                            let elapsed = spike_ts.elapsed().as_secs();
                            if elapsed <= CORRELATION_SECS {
                                info!("BLE + acoustic co-event confirmed! elapsed={}s rssi={}", elapsed, sighting.rssi);
                                stream.pause().ok();
                                return self.submit_proof(&req, sighting.rssi, elapsed as u32).await.map(|_| true);
                            }
                        }
                    }
                }

                // FFT analysis tick.
                _ = check_interval.tick() => {
                    if let Some(peak_db) = self.analyze_audio(&samples, &fft, sample_rate) {
                        debug!("audio peak: {:.1} dBFS", peak_db);
                        if peak_db > PEAK_DB_THRESHOLD {
                            info!("acoustic spike detected: {:.1} dBFS", peak_db);
                            last_spike_ts = Some(std::time::Instant::now());
                        }
                    }
                }

                _ = tokio::time::sleep_until(deadline) => {
                    warn!("acoustic challenge timed out");
                    stream.pause().ok();
                    return Ok(false);
                }
            }
        }
    }

    fn analyze_audio(
        &self,
        samples: &Arc<Mutex<Vec<f32>>>,
        fft: &Arc<dyn rustfft::Fft<f32>>,
        sample_rate: f32,
    ) -> Option<f32> {
        let buf = samples.lock().ok()?;
        if buf.len() < FFT_WINDOW {
            return None;
        }
        let window_data = &buf[buf.len() - FFT_WINDOW..];

        let mut complex: Vec<Complex<f32>> = window_data
            .iter()
            .map(|&s| Complex::new(s, 0.0))
            .collect();
        fft.process(&mut complex);

        // Find peak energy in the beep frequency band.
        let bin_lo = (BEEP_HZ_LO / sample_rate * FFT_WINDOW as f32) as usize;
        let bin_hi = (BEEP_HZ_HI / sample_rate * FFT_WINDOW as f32) as usize;

        let peak_mag = complex[bin_lo..bin_hi.min(complex.len())]
            .iter()
            .map(|c| c.norm())
            .fold(0.0f32, f32::max);

        if peak_mag == 0.0 {
            return None;
        }

        // Convert to dBFS.
        let db = 20.0 * (peak_mag / FFT_WINDOW as f32).log10();
        Some(db)
    }

    async fn submit_proof(
        &self,
        req: &ChallengeRequest,
        rssi: i16,
        elapsed_secs: u32,
    ) -> anyhow::Result<()> {
        let epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            / self.epoch_secs;

        let mut h = Sha256::new();
        h.update(req.tag_type.as_bytes());
        h.update(rssi.to_le_bytes());
        h.update(elapsed_secs.to_le_bytes());
        h.update(req.challenge_nonce.as_bytes());
        h.update(epoch.to_le_bytes());
        let proof_hash = hex::encode(h.finalize());

        let body = serde_json::json!({
            "serial_commitment": req.serial_commitment,
            "witness_id": self.observer_id,
            "proof_hash": proof_hash,
            "claimer": req.claimer,
            "epoch": epoch,
            "signed_by": self.account,
        });

        self.client.post_acoustic_proof(&body).await?;
        info!("acoustic proof submitted: {}", &proof_hash[..12]);
        Ok(())
    }
}
