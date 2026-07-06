//! Render-worker provisioning for Wiiv.
//!
//! A miner is text-only by default. To render, a node opts in to a specific
//! capability, which — after a hardware gate passes — pulls that capability's
//! provider manifest (model weights + runtime), self-tests it, and only then
//! advertises the capability via `WiivWorkerRegister`.
//!
//! This module holds the *node-side* enable/gate/pull/register logic and the
//! `RenderProvider` seam. It does NO large downloads itself: a concrete provider
//! supplies "how to fetch this model" and "how to render one job". See
//! `docs/WIIV_PROTOCOL.md` → Render Worker Provisioning.

use crate::{Capability, DeliverableKind};
use serde::{Deserialize, Serialize};

// ── Hardware ────────────────────────────────────────────────────────────────

/// A snapshot of the local machine's render-relevant hardware.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Hardware {
    pub vram_mb: u64,
    pub ram_mb: u64,
    pub free_disk_mb: u64,
    pub has_gpu: bool,
}

/// Minimum hardware a provider needs. Checked BEFORE any large download.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct MinHardware {
    pub vram_mb: u64,
    pub ram_mb: u64,
    pub disk_mb: u64,
    pub requires_gpu: bool,
}

impl MinHardware {
    /// Returns the list of unmet requirements (empty = the gate passes).
    pub fn shortfalls(&self, hw: &Hardware) -> Vec<String> {
        let mut out = Vec::new();
        if self.requires_gpu && !hw.has_gpu {
            out.push("requires a GPU, none detected".to_string());
        }
        if hw.vram_mb < self.vram_mb {
            out.push(format!("needs >={} MB VRAM, found {}", self.vram_mb, hw.vram_mb));
        }
        if hw.ram_mb < self.ram_mb {
            out.push(format!("needs >={} MB RAM, found {}", self.ram_mb, hw.ram_mb));
        }
        if hw.free_disk_mb < self.disk_mb {
            out.push(format!(
                "needs >={} MB free disk, found {}",
                self.disk_mb, hw.free_disk_mb
            ));
        }
        out
    }
}

// ── Provider manifest ───────────────────────────────────────────────────────

/// One model artifact to fetch and hash-verify.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelArtifact {
    /// HONE-FS CID (preferred) — content-addressed, mirror-served.
    #[serde(default)]
    pub cid: Option<String>,
    /// Origin URL fallback if no mirror holds the CID yet.
    #[serde(default)]
    pub url: Option<String>,
    pub sha256: String,
    pub size_mb: u64,
    /// Where it lands on disk, relative to the capability's model dir.
    pub path: String,
}

/// Runtime dependency (inference engine, codec, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeDep {
    pub name: String,
    pub version: String,
}

/// The full recipe for serving one capability: what to pull, what to run, the
/// hardware gate, a self-test, and which deliverables it can produce. Content-
/// addressed + versioned so every worker on a capability runs a known stack.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderManifest {
    pub capability: Capability,
    pub provider_id: String,
    pub version: String,
    pub models: Vec<ModelArtifact>,
    pub runtime: Vec<RuntimeDep>,
    pub min_hardware: MinHardware,
    pub output_kinds: Vec<DeliverableKind>,
}

impl ProviderManifest {
    /// Total bytes (MB) this manifest would pull — surfaced to the user before
    /// they confirm an enable.
    pub fn total_download_mb(&self) -> u64 {
        self.models.iter().map(|m| m.size_mb).sum()
    }

    /// Structural validation. Empty = valid.
    pub fn validate(&self) -> Vec<String> {
        let mut p = Vec::new();
        if self.provider_id.trim().is_empty() {
            p.push("provider_id is required".to_string());
        }
        if self.models.is_empty() {
            p.push("a provider must declare at least one model artifact".to_string());
        }
        for (i, m) in self.models.iter().enumerate() {
            if m.cid.is_none() && m.url.is_none() {
                p.push(format!("model[{i}] has neither cid nor url"));
            }
            if m.sha256.trim().is_empty() {
                p.push(format!("model[{i}] missing sha256"));
            }
        }
        if self.output_kinds.is_empty() {
            p.push("a provider must declare at least one output_kind".to_string());
        }
        p
    }
}

// ── Provider seam ───────────────────────────────────────────────────────────

/// A pluggable render backend for one capability. The node drives enable →
/// gate → pull → self-test → register; the provider supplies the how.
///
/// Kept object-safe and async-free at the trait level so concrete providers can
/// choose their own async runtime internals without forcing one here. Real
/// implementations wrap network/GPU work; this repo ships only a stub.
pub trait RenderProvider {
    fn manifest(&self) -> &ProviderManifest;

    /// Pull + verify all model artifacts. Returns bytes fetched, or an error
    /// string. A real provider fetches from HONE-FS/URL and checks sha256.
    fn ensure_models(&self) -> Result<u64, String>;

    /// Run the deterministic self-test render. Must pass before the node will
    /// advertise the capability.
    fn self_test(&self) -> Result<(), String>;
}

// ── Worker state ────────────────────────────────────────────────────────────

/// Where an enabled capability is in the provisioning flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvisionState {
    /// Enable requested; hardware gate not yet checked.
    Requested,
    /// Hardware gate passed; models not yet resident.
    Gated,
    /// Models pulled + verified; self-test not yet run.
    Pulled,
    /// Self-test passed; ready to advertise.
    Ready,
    /// Enable refused (hardware shortfall or provider failure).
    Refused,
}

/// A node's local record for one opted-in capability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityEnablement {
    pub capability: Capability,
    pub provider_id: String,
    pub state: ProvisionState,
    #[serde(default)]
    pub notes: Vec<String>,
}

/// The registration a node emits (as `WiivWorkerRegister`) once at least one
/// capability reached `Ready`. Bound to hardware attestation for anti-sybil.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerRegistration {
    pub account: String,
    pub machine_id: String,
    pub capabilities: Vec<Capability>,
    #[serde(default)]
    pub price_hint_hunits: Option<u64>,
}

/// Drive the opt-in flow for one capability against a provider and the local
/// hardware. Pure orchestration: it calls the provider's pull/self-test and
/// records the resulting state. Returns the enablement record.
///
/// Nothing large is pulled until the hardware gate passes — the gate is checked
/// first and, on shortfall, the flow ends in `Refused` before `ensure_models`
/// is ever called.
pub fn provision_capability<P: RenderProvider>(
    provider: &P,
    hw: &Hardware,
) -> CapabilityEnablement {
    let manifest = provider.manifest();
    let cap = manifest.capability;
    let provider_id = manifest.provider_id.clone();

    // 1. Hardware gate — BEFORE any download.
    let shortfalls = manifest.min_hardware.shortfalls(hw);
    if !shortfalls.is_empty() {
        return CapabilityEnablement {
            capability: cap,
            provider_id,
            state: ProvisionState::Refused,
            notes: shortfalls,
        };
    }

    // 2. Pull + verify models.
    if let Err(e) = provider.ensure_models() {
        return CapabilityEnablement {
            capability: cap,
            provider_id,
            state: ProvisionState::Refused,
            notes: vec![format!("model pull failed: {e}")],
        };
    }

    // 3. Self-test render.
    if let Err(e) = provider.self_test() {
        return CapabilityEnablement {
            capability: cap,
            provider_id,
            state: ProvisionState::Refused,
            notes: vec![format!("self-test failed: {e}")],
        };
    }

    // 4. Ready to advertise.
    CapabilityEnablement {
        capability: cap,
        provider_id,
        state: ProvisionState::Ready,
        notes: vec![],
    }
}

// ── Bridge to the node-wide provisioner ─────────────────────────────────────
//
// Wiiv render workers are one *consumer* of the general capability provisioner
// (`hone_provision`). This bridge maps a Wiiv `RenderProvider` into the general
// `CapabilityProvider` and routes through `hone_provision::provision`, so a
// render capability flows through the exact same gate→acquire→self-test→ready
// path the node uses for inference / storage / sensor roles. `provision_capability`
// above stays as the Wiiv-native convenience wrapper.

/// Map a Wiiv capability to the general capability name.
fn wiiv_capability_name(cap: Capability) -> &'static str {
    match cap {
        Capability::VideoGeneration => "video_generation",
        Capability::ImageGeneration => "image_generation",
        Capability::ThreedGeneration => "threed_generation",
        Capability::VoiceSynthesis => "voice_synthesis",
        Capability::MusicGeneration => "music_generation",
        Capability::EditingAssembly => "editing_assembly",
        Capability::Upscaling => "upscaling",
        Capability::Subtitles => "subtitles",
        Capability::TextInference => "text_inference",
        Capability::Storage => "blob_serving",
        Capability::Review => "review",
    }
}

/// Build a general `ResourceManifest` (Render role) from a Wiiv provider manifest.
pub fn to_general_manifest(m: &ProviderManifest) -> hone_provision::ResourceManifest {
    hone_provision::ResourceManifest {
        capability: hone_provision::Capability::new(
            hone_provision::NodeRole::Render,
            wiiv_capability_name(m.capability),
        ),
        provider_id: m.provider_id.clone(),
        version: m.version.clone(),
        requirements: hone_provision::Requirements {
            vram_mb: m.min_hardware.vram_mb,
            ram_mb: m.min_hardware.ram_mb,
            disk_mb: m.min_hardware.disk_mb,
            cpu_cores: 0,
            requires_gpu: m.min_hardware.requires_gpu,
        },
        resources: m
            .models
            .iter()
            .map(|a| hone_provision::Resource {
                kind: "model".to_string(),
                name: a.path.clone(),
                cid: a.cid.clone(),
                url: a.url.clone(),
                sha256: Some(a.sha256.clone()),
                size_mb: a.size_mb,
            })
            .collect(),
    }
}

/// Adapter so a Wiiv `RenderProvider` satisfies the general `CapabilityProvider`.
pub struct GeneralProviderAdapter<'a, P: RenderProvider> {
    inner: &'a P,
    manifest: hone_provision::ResourceManifest,
}

impl<'a, P: RenderProvider> GeneralProviderAdapter<'a, P> {
    pub fn new(inner: &'a P) -> Self {
        let manifest = to_general_manifest(inner.manifest());
        Self { inner, manifest }
    }
}

impl<'a, P: RenderProvider> hone_provision::CapabilityProvider for GeneralProviderAdapter<'a, P> {
    fn manifest(&self) -> &hone_provision::ResourceManifest {
        &self.manifest
    }
    fn acquire(&self) -> Result<u64, String> {
        self.inner.ensure_models()
    }
    fn self_test(&self) -> Result<(), String> {
        self.inner.self_test()
    }
}

/// Provision a Wiiv render capability THROUGH the node-wide provisioner. Returns
/// the general `Enablement`, proving render is one role among the node's roles.
pub fn provision_via_node<P: RenderProvider>(
    provider: &P,
    hw: &hone_provision::Hardware,
) -> hone_provision::Enablement {
    let adapter = GeneralProviderAdapter::new(provider);
    hone_provision::provision(&adapter, hw)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video_manifest() -> ProviderManifest {
        ProviderManifest {
            capability: Capability::VideoGeneration,
            provider_id: "stub-video-v0".to_string(),
            version: "0.1.0".to_string(),
            models: vec![ModelArtifact {
                cid: Some("bafyvideofake".to_string()),
                url: None,
                sha256: "deadbeef".to_string(),
                size_mb: 6000,
                path: "video/model.gguf".to_string(),
            }],
            runtime: vec![RuntimeDep { name: "ffmpeg".into(), version: "6".into() }],
            min_hardware: MinHardware {
                vram_mb: 12000,
                ram_mb: 16000,
                disk_mb: 20000,
                requires_gpu: true,
            },
            output_kinds: vec![DeliverableKind::GeneratedScene, DeliverableKind::FinalRender],
        }
    }

    struct StubProvider {
        m: ProviderManifest,
        pull_ok: bool,
        test_ok: bool,
    }
    impl RenderProvider for StubProvider {
        fn manifest(&self) -> &ProviderManifest {
            &self.m
        }
        fn ensure_models(&self) -> Result<u64, String> {
            if self.pull_ok {
                Ok(self.m.total_download_mb())
            } else {
                Err("network".into())
            }
        }
        fn self_test(&self) -> Result<(), String> {
            if self.test_ok {
                Ok(())
            } else {
                Err("bad render".into())
            }
        }
    }

    fn beefy() -> Hardware {
        Hardware { vram_mb: 24000, ram_mb: 32000, free_disk_mb: 100000, has_gpu: true }
    }
    fn weak() -> Hardware {
        Hardware { vram_mb: 8000, ram_mb: 16000, free_disk_mb: 100000, has_gpu: true }
    }

    #[test]
    fn manifest_validates() {
        assert!(video_manifest().validate().is_empty());
    }

    #[test]
    fn total_download_reported() {
        assert_eq!(video_manifest().total_download_mb(), 6000);
    }

    #[test]
    fn weak_hardware_refused_before_download() {
        // A provider whose pull would PANIC if called — proves the gate runs first.
        struct NoPull(ProviderManifest);
        impl RenderProvider for NoPull {
            fn manifest(&self) -> &ProviderManifest {
                &self.0
            }
            fn ensure_models(&self) -> Result<u64, String> {
                panic!("must not download when hardware gate fails");
            }
            fn self_test(&self) -> Result<(), String> {
                panic!("must not self-test when hardware gate fails");
            }
        }
        let p = NoPull(video_manifest());
        let e = provision_capability(&p, &weak());
        assert_eq!(e.state, ProvisionState::Refused);
        assert!(e.notes.iter().any(|n| n.contains("VRAM")));
    }

    #[test]
    fn capable_hardware_reaches_ready() {
        let p = StubProvider { m: video_manifest(), pull_ok: true, test_ok: true };
        let e = provision_capability(&p, &beefy());
        assert_eq!(e.state, ProvisionState::Ready);
        assert_eq!(e.capability, Capability::VideoGeneration);
    }

    #[test]
    fn failed_self_test_refuses() {
        let p = StubProvider { m: video_manifest(), pull_ok: true, test_ok: false };
        let e = provision_capability(&p, &beefy());
        assert_eq!(e.state, ProvisionState::Refused);
        assert!(e.notes.iter().any(|n| n.contains("self-test")));
    }

    #[test]
    fn render_provisions_through_the_node_wide_provisioner() {
        // A Wiiv render capability flows through hone_provision::provision, proving
        // render is one role among the node's roles (inference/storage/sensor/...).
        let p = StubProvider { m: video_manifest(), pull_ok: true, test_ok: true };
        let hw = hone_provision::Hardware {
            vram_mb: 24000,
            ram_mb: 32000,
            free_disk_mb: 100_000,
            cpu_cores: 16,
            has_gpu: true,
        };
        let e = provision_via_node(&p, &hw);
        assert_eq!(e.state, hone_provision::ProvisionState::Ready);
        assert_eq!(e.capability.role, hone_provision::NodeRole::Render);
        assert_eq!(e.capability.name, "video_generation");
    }
}
