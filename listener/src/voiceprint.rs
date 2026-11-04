//! Fingerprint hook. The `hone-voiceprint` crate (branch `voiceprint`, crates/hone-voiceprint)
//! is being built separately; until it lands the listener passes Perch embeddings to a no-op
//! sink. Embeddings never leave this process.

pub trait VoiceprintSink: Send {
    /// One detection: species (Latin name), the frame's Perch embedding (1,536 floats),
    /// detection time (ms since the Unix epoch) and confidence (0..1).
    fn on_detection(&mut self, species: &str, embedding: &[f32], t: u64, conf: f32);
}

/// Default sink: does nothing. Replace with hone_voiceprint's sink once that crate lands.
#[derive(Default)]
pub struct NoopVoiceprint;

impl VoiceprintSink for NoopVoiceprint {
    fn on_detection(&mut self, _species: &str, _embedding: &[f32], _t: u64, _conf: f32) {}
}
