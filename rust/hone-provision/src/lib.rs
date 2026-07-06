//! HONE node-wide capability provisioning — the autopilot layer.
//!
//! A person says "I want this machine to do X" and the node makes it happen:
//! check the hardware, pull whatever that capability needs, self-test it, and
//! only then advertise/enable the role. This is one uniform flow across every
//! node role — inference, storage, sensor, and render — not a per-vertical
//! bespoke path.
//!
//! ```text
//! enable(capability) → hardware gate → acquire resources → self-test → Ready
//!                    │ (gate fails)                                   │
//!                    └────────────────────── Refused ────────────────┘
//! ```
//!
//! The flow is generic: a [`CapabilityProvider`] supplies "what does this
//! capability need" (a [`ResourceManifest`]) and "acquire it" / "self-test it".
//! Rendering (rust/wiiv) is one consumer; the node's own inference/storage/sensor
//! roles are others. Nothing heavy is acquired before the hardware gate passes —
//! no surprise downloads, ever.
//!
//! Pure: no I/O, no clock. Providers do the real work behind the trait.

use serde::{Deserialize, Serialize};

// ── Roles & capabilities ────────────────────────────────────────────────────

/// A node role, aligned with HONE's on-chain reward vocabulary
/// (Mine/Clock/Storage/Sensor/Verifier/Service/Mempool) plus Render (Wiiv).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeRole {
    /// Text/inference mining (the embedded candle model; MineReward).
    Inference,
    /// Content storage / HONE-FS serving (StorageReward).
    Storage,
    /// Sensor data submission (SensorReward).
    Sensor,
    /// Clock / epoch timing (ClockReward).
    Clock,
    /// Render worker — image/video/audio/3D (Wiiv).
    Render,
}

impl NodeRole {
    pub fn as_str(self) -> &'static str {
        match self {
            NodeRole::Inference => "inference",
            NodeRole::Storage => "storage",
            NodeRole::Sensor => "sensor",
            NodeRole::Clock => "clock",
            NodeRole::Render => "render",
        }
    }
}

/// A capability a node can opt into. Free-form string so verticals (e.g. Wiiv's
/// `video_generation`) can define their own without changing this crate; the role
/// says which subsystem owns it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capability {
    pub role: NodeRole,
    /// e.g. "text_inference", "blob_serving", "gps", "video_generation".
    pub name: String,
}

impl Capability {
    pub fn new(role: NodeRole, name: impl Into<String>) -> Self {
        Self { role, name: name.into() }
    }
}

// ── Hardware gate ───────────────────────────────────────────────────────────

/// Local machine capacity relevant to provisioning. Roles read only what they
/// care about (storage cares about disk, render about VRAM, etc.).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Hardware {
    pub vram_mb: u64,
    pub ram_mb: u64,
    pub free_disk_mb: u64,
    pub cpu_cores: u32,
    pub has_gpu: bool,
}

/// Minimum requirements a capability imposes. Checked BEFORE any acquisition.
/// A field of 0 / false means "don't care".
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Requirements {
    pub vram_mb: u64,
    pub ram_mb: u64,
    pub disk_mb: u64,
    pub cpu_cores: u32,
    pub requires_gpu: bool,
}

impl Requirements {
    /// Unmet requirements (empty = the gate passes).
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
            out.push(format!("needs >={} MB free disk, found {}", self.disk_mb, hw.free_disk_mb));
        }
        if (hw.cpu_cores as u64) < (self.cpu_cores as u64) {
            out.push(format!("needs >={} CPU cores, found {}", self.cpu_cores, hw.cpu_cores));
        }
        out
    }

    /// Does this machine meet the requirements? (suggestion/autopilot use)
    pub fn met_by(&self, hw: &Hardware) -> bool {
        self.shortfalls(hw).is_empty()
    }
}

// ── Resource manifest ───────────────────────────────────────────────────────

/// One resource a capability must acquire before it can serve — a model file, a
/// runtime dependency, a disk allocation, etc. Kinds are open; the provider
/// interprets them. Sized so the user sees the cost before confirming.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resource {
    /// "model" | "runtime" | "disk_alloc" | "index" | ...
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub cid: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub size_mb: u64,
}

/// Everything a capability needs to become serveable: its hardware gate, the
/// resources to acquire, and which role it belongs to.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceManifest {
    pub capability: Capability,
    pub provider_id: String,
    pub version: String,
    pub requirements: Requirements,
    pub resources: Vec<Resource>,
}

impl ResourceManifest {
    /// Total bytes (MB) to acquire — surfaced before the user confirms.
    pub fn total_acquire_mb(&self) -> u64 {
        self.resources.iter().map(|r| r.size_mb).sum()
    }

    pub fn validate(&self) -> Vec<String> {
        let mut p = Vec::new();
        if self.provider_id.trim().is_empty() {
            p.push("provider_id is required".to_string());
        }
        if self.capability.name.trim().is_empty() {
            p.push("capability.name is required".to_string());
        }
        for (i, r) in self.resources.iter().enumerate() {
            // A downloadable resource needs a source + hash; a local alloc (e.g.
            // disk) legitimately has neither, so only check sized network pulls.
            let is_download = r.cid.is_some() || r.url.is_some();
            if is_download && r.sha256.as_deref().unwrap_or("").is_empty() {
                p.push(format!("resource[{i}] '{}' is a download but has no sha256", r.name));
            }
        }
        p
    }
}

// ── Provider seam ───────────────────────────────────────────────────────────

/// A subsystem that can provision one capability. The provisioner drives
/// gate → acquire → self-test → ready; the provider supplies the how.
///
/// Object-safe and async-free so concrete providers (a candle model puller, a
/// storage allocator, a Wiiv video provider) choose their own internals.
pub trait CapabilityProvider {
    fn manifest(&self) -> &ResourceManifest;

    /// Acquire + verify all resources (download models, allocate disk, install
    /// runtime). Returns MB acquired, or an error string.
    fn acquire(&self) -> Result<u64, String>;

    /// Prove the capability works locally (a tiny inference, a test write, a
    /// self-test render). Must pass before the node advertises the capability.
    fn self_test(&self) -> Result<(), String>;
}

// ── Provisioning flow ───────────────────────────────────────────────────────

/// Where an opted-in capability sits in the flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvisionState {
    Requested,
    Gated,
    Acquired,
    Ready,
    Refused,
}

/// The node's local record for one opted-in capability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Enablement {
    pub capability: Capability,
    pub provider_id: String,
    pub state: ProvisionState,
    #[serde(default)]
    pub acquired_mb: u64,
    #[serde(default)]
    pub notes: Vec<String>,
}

/// Drive the opt-in flow for one capability. Pure orchestration — the gate is
/// checked FIRST and, on shortfall, returns `Refused` before `acquire()` is ever
/// called (the no-surprise-download guarantee).
pub fn provision<P: CapabilityProvider>(provider: &P, hw: &Hardware) -> Enablement {
    let m = provider.manifest();
    let capability = m.capability.clone();
    let provider_id = m.provider_id.clone();

    // 1. Structural sanity.
    let problems = m.validate();
    if !problems.is_empty() {
        return Enablement { capability, provider_id, state: ProvisionState::Refused, acquired_mb: 0, notes: problems };
    }

    // 2. Hardware gate — BEFORE any acquisition.
    let shortfalls = m.requirements.shortfalls(hw);
    if !shortfalls.is_empty() {
        return Enablement { capability, provider_id, state: ProvisionState::Refused, acquired_mb: 0, notes: shortfalls };
    }

    // 3. Acquire resources.
    let acquired_mb = match provider.acquire() {
        Ok(mb) => mb,
        Err(e) => {
            return Enablement { capability, provider_id, state: ProvisionState::Refused, acquired_mb: 0, notes: vec![format!("acquire failed: {e}")] };
        }
    };

    // 4. Self-test.
    if let Err(e) = provider.self_test() {
        return Enablement { capability, provider_id, state: ProvisionState::Refused, acquired_mb, notes: vec![format!("self-test failed: {e}")] };
    }

    // 5. Ready to advertise/enable.
    Enablement { capability, provider_id, state: ProvisionState::Ready, acquired_mb, notes: vec![] }
}

/// Autopilot: given a machine and a set of candidate providers, provision every
/// one whose hardware gate passes, and report the results. This is the
/// "detect what this PC can do and turn it all on" entry point.
pub fn autopilot<P: CapabilityProvider>(providers: &[P], hw: &Hardware) -> Vec<Enablement> {
    providers.iter().map(|p| provision(p, hw)).collect()
}

/// Of a candidate set, which capabilities COULD this machine serve (gate only,
/// no acquisition)? Used to suggest roles without committing to downloads.
pub fn suggest<'a, P: CapabilityProvider>(providers: &'a [P], hw: &Hardware) -> Vec<&'a Capability> {
    providers
        .iter()
        .map(|p| p.manifest())
        .filter(|m| m.requirements.met_by(hw))
        .map(|m| &m.capability)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(role: NodeRole, name: &str, req: Requirements, mb: u64) -> ResourceManifest {
        ResourceManifest {
            capability: Capability::new(role, name),
            provider_id: format!("{name}-stub"),
            version: "0.1.0".to_string(),
            requirements: req,
            resources: if mb > 0 {
                vec![Resource {
                    kind: "model".into(),
                    name: format!("{name}-model"),
                    cid: Some("bafyfake".into()),
                    url: None,
                    sha256: Some("deadbeef".into()),
                    size_mb: mb,
                }]
            } else {
                vec![]
            },
        }
    }

    struct Stub {
        m: ResourceManifest,
        acquire_ok: bool,
        test_ok: bool,
    }
    impl CapabilityProvider for Stub {
        fn manifest(&self) -> &ResourceManifest { &self.m }
        fn acquire(&self) -> Result<u64, String> {
            if self.acquire_ok { Ok(self.m.total_acquire_mb()) } else { Err("net".into()) }
        }
        fn self_test(&self) -> Result<(), String> {
            if self.test_ok { Ok(()) } else { Err("bad".into()) }
        }
    }

    fn beefy() -> Hardware {
        Hardware { vram_mb: 24000, ram_mb: 32000, free_disk_mb: 500_000, cpu_cores: 16, has_gpu: true }
    }
    fn potato() -> Hardware {
        Hardware { vram_mb: 0, ram_mb: 8000, free_disk_mb: 50_000, cpu_cores: 4, has_gpu: false }
    }

    fn video_req() -> Requirements {
        Requirements { vram_mb: 12000, ram_mb: 16000, disk_mb: 20000, cpu_cores: 4, requires_gpu: true }
    }
    fn storage_req() -> Requirements {
        Requirements { disk_mb: 10000, ..Default::default() }
    }
    fn inference_req() -> Requirements {
        Requirements { ram_mb: 6000, ..Default::default() }
    }

    #[test]
    fn render_gate_blocks_gpu_less_machine_before_download() {
        // Provider panics if acquire is called — proves the gate runs first.
        struct NoAcquire(ResourceManifest);
        impl CapabilityProvider for NoAcquire {
            fn manifest(&self) -> &ResourceManifest { &self.0 }
            fn acquire(&self) -> Result<u64, String> { panic!("must not download when gate fails") }
            fn self_test(&self) -> Result<(), String> { panic!("must not self-test when gate fails") }
        }
        let p = NoAcquire(manifest(NodeRole::Render, "video_generation", video_req(), 6000));
        let e = provision(&p, &potato());
        assert_eq!(e.state, ProvisionState::Refused);
        assert!(e.notes.iter().any(|n| n.contains("GPU") || n.contains("VRAM")));
    }

    #[test]
    fn storage_provisions_on_a_gpu_less_machine() {
        // Storage needs no GPU — a potato can serve it.
        let p = Stub { m: manifest(NodeRole::Storage, "blob_serving", storage_req(), 0), acquire_ok: true, test_ok: true };
        let e = provision(&p, &potato());
        assert_eq!(e.state, ProvisionState::Ready);
    }

    #[test]
    fn inference_ready_on_beefy() {
        let p = Stub { m: manifest(NodeRole::Inference, "text_inference", inference_req(), 400), acquire_ok: true, test_ok: true };
        let e = provision(&p, &beefy());
        assert_eq!(e.state, ProvisionState::Ready);
        assert_eq!(e.acquired_mb, 400);
    }

    #[test]
    fn failed_self_test_refuses_after_acquire() {
        let p = Stub { m: manifest(NodeRole::Render, "video_generation", video_req(), 6000), acquire_ok: true, test_ok: false };
        let e = provision(&p, &beefy());
        assert_eq!(e.state, ProvisionState::Refused);
        assert!(e.notes.iter().any(|n| n.contains("self-test")));
    }

    #[test]
    fn autopilot_enables_viable_refuses_rest() {
        let providers = vec![
            Stub { m: manifest(NodeRole::Inference, "text_inference", inference_req(), 400), acquire_ok: true, test_ok: true },
            Stub { m: manifest(NodeRole::Storage, "blob_serving", storage_req(), 0), acquire_ok: true, test_ok: true },
            Stub { m: manifest(NodeRole::Render, "video_generation", video_req(), 6000), acquire_ok: true, test_ok: true },
        ];
        // On a potato: inference + storage ready, render refused (no GPU).
        let results = autopilot(&providers, &potato());
        let ready: Vec<_> = results.iter().filter(|e| e.state == ProvisionState::Ready).map(|e| e.capability.name.as_str()).collect();
        assert!(ready.contains(&"text_inference"));
        assert!(ready.contains(&"blob_serving"));
        assert!(!ready.contains(&"video_generation"));
    }

    #[test]
    fn suggest_lists_only_viable_without_downloading() {
        let providers = vec![
            Stub { m: manifest(NodeRole::Storage, "blob_serving", storage_req(), 0), acquire_ok: true, test_ok: true },
            Stub { m: manifest(NodeRole::Render, "video_generation", video_req(), 6000), acquire_ok: true, test_ok: true },
        ];
        let names: Vec<_> = suggest(&providers, &potato()).iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"blob_serving"));
        assert!(!names.contains(&"video_generation"));
    }
}
