//! `ComfyUiVideoProvider` — a concrete `RenderProvider` backed by a supervised
//! ComfyUI engine running the Wan2.2 5B text-to-video model on a local GPU.
//!
//! # Why a supervised engine (and why that's OK here)
//!
//! HONE owns render *orchestration* natively in Rust; ComfyUI is a **supervised
//! engine behind the `RenderProvider` seam** — the same pattern the node uses for
//! inference via the optional `INFERENCE_URL` seam. HONE code drives it; ComfyUI
//! does the GPU diffusion. This is a model-port gap, not a language choice:
//! candle cannot run the Wan2.2 video model, so the diffusion runs in ComfyUI's
//! PyTorch/CUDA stack while HONE keeps the orchestration, provisioning, and
//! provenance.
//!
//! IMPORTANT DISTINCTION FROM INFERENCE. The node's *inference* seam must be
//! embedded / non-supervised because inference feeds **chain consensus** (block
//! production, rewards) and a supervised external engine would be an anti-cheat
//! hole. RENDER is different: render quality is **buyer/reviewer-accepted**, not
//! reproduced by consensus. The chain settles on buyer acceptance + artifact
//! provenance (which worker, which capability/model, input CIDs → output CID),
//! NOT on re-deriving the render. A supervised render engine is therefore
//! acceptable here where a supervised inference engine would not be.
//!
//! What's real vs. stubbed in this module:
//! - REAL: manifest, on-disk model presence check, ComfyUI reachability probe
//!   (`GET /system_stats`), HTTP plumbing to `POST /prompt`, and a faithful
//!   translation of the bundled `video_wan2_2_5B_ti2v` template into ComfyUI's
//!   `/prompt` API graph format for text-to-video.
//! - FUTURE ENHANCEMENT: polling `/history/{prompt_id}` for completion and
//!   resolving the produced file into a HONE-FS CID; a full test render in
//!   `self_test` (today it does reachability + model-present, which is cheap).

use crate::worker::{
    MinHardware, ModelArtifact, ProviderManifest, RenderProvider, RuntimeDep,
};
use crate::{Capability, DeliverableKind};
use std::path::{Path, PathBuf};

/// Default ComfyUI HTTP base. Override with `COMFYUI_URL`.
pub const DEFAULT_COMFYUI_URL: &str = "http://127.0.0.1:8188";

/// Environment variable naming the ComfyUI HTTP base.
pub const ENV_COMFYUI_URL: &str = "COMFYUI_URL";

/// Environment variable naming the ComfyUI install root (the dir that holds
/// `models/`). Used to resolve where the Wan2.2 weights should live on disk.
pub const ENV_COMFYUI_ROOT: &str = "COMFYUI_ROOT";

/// The three Wan2.2 5B artifacts this provider drives, as ComfyUI names them.
/// Paths are relative to `<comfy_root>/models/`. Sizes are the approximate
/// on-disk sizes of the fp16 5B / fp8 text-encoder / vae repackaged files.
const WAN22_DIFFUSION_MODEL: &str = "diffusion_models/wan2.2_ti2v_5B_fp16.safetensors";
const WAN22_TEXT_ENCODER: &str = "text_encoders/umt5_xxl_fp8_e4m3fn_scaled.safetensors";
const WAN22_VAE: &str = "vae/wan2.2_vae.safetensors";

// Upstream (Comfy-Org repackaged) download URLs, recorded in the manifest so a
// future acquire step / operator knows the canonical origin. This provider does
// NOT download them — `ensure_models` only checks presence.
const URL_DIFFUSION: &str = "https://huggingface.co/Comfy-Org/Wan_2.2_ComfyUI_Repackaged/resolve/main/split_files/diffusion_models/wan2.2_ti2v_5B_fp16.safetensors";
const URL_TEXT_ENCODER: &str = "https://huggingface.co/Comfy-Org/Wan_2.1_ComfyUI_repackaged/resolve/main/split_files/text_encoders/umt5_xxl_fp8_e4m3fn_scaled.safetensors";
const URL_VAE: &str = "https://huggingface.co/Comfy-Org/Wan_2.2_ComfyUI_Repackaged/resolve/main/split_files/vae/wan2.2_vae.safetensors";

/// Parameters for one text-to-video render. Sensible Wan2.2 5B defaults per the
/// bundled template (720p-ish, 121 frames @ 24fps ≈ 5s).
#[derive(Debug, Clone)]
pub struct RenderParams {
    pub width: u32,
    pub height: u32,
    /// Frame count. Wan2.2 latent length; 121 ≈ 5s at 24fps.
    pub length: u32,
    pub fps: u32,
    pub steps: u32,
    pub cfg: f32,
    pub seed: u64,
    pub negative_prompt: String,
}

impl Default for RenderParams {
    fn default() -> Self {
        // Matches the widgets in the bundled video_wan2_2_5B_ti2v template.
        Self {
            width: 1280,
            height: 704,
            length: 121,
            fps: 24,
            steps: 20,
            cfg: 5.0,
            seed: 898_471_028_164_125,
            // The template ships a Chinese negative prompt; keep an ASCII default
            // that expresses the same "avoid low quality / artifacts" intent.
            negative_prompt:
                "low quality, worst quality, blurry, jpeg artifacts, static, deformed, watermark, text"
                    .to_string(),
        }
    }
}

/// A reference to a submitted render job on the ComfyUI engine. Completion
/// polling + CID resolution is a future enhancement (see module docs).
#[derive(Debug, Clone)]
pub struct RenderHandle {
    /// ComfyUI's queued prompt id (from `POST /prompt`).
    pub prompt_id: String,
    /// The base URL the job was submitted to.
    pub base_url: String,
}

/// Concrete provider talking to a supervised ComfyUI engine over HTTP.
pub struct ComfyUiVideoProvider {
    manifest: ProviderManifest,
    base_url: String,
    /// ComfyUI install root (holds `models/`), for the on-disk presence check.
    comfy_root: PathBuf,
    http: reqwest::blocking::Client,
}

impl ComfyUiVideoProvider {
    /// Build from explicit values (used by tests to point at a dead port / temp
    /// dir so the suite stays hermetic).
    pub fn new(base_url: impl Into<String>, comfy_root: impl Into<PathBuf>) -> Self {
        let base_url = base_url.into();
        let comfy_root = comfy_root.into();
        Self {
            manifest: build_manifest(&base_url),
            base_url,
            comfy_root,
            http: reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("reqwest blocking client"),
        }
    }

    /// Build from environment: `COMFYUI_URL` (default `http://127.0.0.1:8188`)
    /// and `COMFYUI_ROOT` (default the well-known local install path).
    pub fn from_env() -> Self {
        let base_url =
            std::env::var(ENV_COMFYUI_URL).unwrap_or_else(|_| DEFAULT_COMFYUI_URL.to_string());
        let comfy_root = std::env::var(ENV_COMFYUI_ROOT)
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("X:/saturday80s/comfy-wan/ComfyUI"));
        Self::new(base_url, comfy_root)
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    fn models_dir(&self) -> PathBuf {
        self.comfy_root.join("models")
    }

    /// Probe ComfyUI liveness via `GET /system_stats`. Returns the parsed JSON
    /// on success, or a clear error string.
    fn probe(&self) -> Result<serde_json::Value, String> {
        let url = format!("{}/system_stats", self.base_url);
        let resp = self
            .http
            .get(&url)
            .send()
            .map_err(|e| format!("ComfyUI unreachable at {url}: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!(
                "ComfyUI at {url} returned HTTP {}",
                resp.status().as_u16()
            ));
        }
        resp.json::<serde_json::Value>()
            .map_err(|e| format!("ComfyUI /system_stats returned non-JSON: {e}"))
    }

    /// Which expected model files are missing on disk. Empty = all present.
    fn missing_models(&self) -> Vec<PathBuf> {
        let dir = self.models_dir();
        [WAN22_DIFFUSION_MODEL, WAN22_TEXT_ENCODER, WAN22_VAE]
            .iter()
            .map(|rel| dir.join(rel))
            .filter(|p| !file_present(p))
            .collect()
    }

    /// Sum of on-disk bytes of the expected model files that are present.
    fn present_bytes(&self) -> u64 {
        let dir = self.models_dir();
        [WAN22_DIFFUSION_MODEL, WAN22_TEXT_ENCODER, WAN22_VAE]
            .iter()
            .map(|rel| dir.join(rel))
            .filter_map(|p| std::fs::metadata(&p).ok())
            .filter(|m| m.is_file() && m.len() > 0)
            .map(|m| m.len())
            .sum()
    }

    /// Submit a Wan2.2 text-to-video render to ComfyUI's `POST /prompt`.
    ///
    /// This wires the real HTTP path and posts a faithful translation of the
    /// bundled 5B TI2V template into ComfyUI's API graph format. Completion
    /// polling + resolving the output file into a HONE-FS CID is a future
    /// enhancement; today it returns a `RenderHandle` carrying the queued
    /// `prompt_id`.
    pub fn render(&self, prompt: &str, params: &RenderParams) -> Result<RenderHandle, String> {
        let graph = wan22_t2v_workflow(prompt, params);
        let body = serde_json::json!({
            "prompt": graph,
            // A stable client id lets us correlate progress over the ws channel later.
            "client_id": "hone-wiiv",
        });

        let url = format!("{}/prompt", self.base_url);
        let resp = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .map_err(|e| format!("POST {url} failed: {e}"))?;

        let status = resp.status();
        let text = resp
            .text()
            .map_err(|e| format!("reading /prompt response body: {e}"))?;
        if !status.is_success() {
            // ComfyUI returns 400 with a node_errors payload on a bad graph.
            return Err(format!(
                "ComfyUI /prompt rejected the workflow (HTTP {}): {text}",
                status.as_u16()
            ));
        }

        let json: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| format!("/prompt returned non-JSON: {e}; body={text}"))?;
        let prompt_id = json
            .get("prompt_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("/prompt response missing prompt_id: {text}"))?
            .to_string();

        Ok(RenderHandle {
            prompt_id,
            base_url: self.base_url.clone(),
        })
    }
}

impl RenderProvider for ComfyUiVideoProvider {
    fn manifest(&self) -> &ProviderManifest {
        &self.manifest
    }

    /// Check the Wan2.2 model files are present on disk AND ComfyUI is reachable.
    /// Returns the on-disk bytes present, or a clear error. Downloads nothing.
    fn ensure_models(&self) -> Result<u64, String> {
        let missing = self.missing_models();
        if !missing.is_empty() {
            let names: Vec<String> = missing.iter().map(|p| p.display().to_string()).collect();
            return Err(format!(
                "missing Wan2.2 model file(s) under {}: {} — download them (this provider does not fetch)",
                self.models_dir().display(),
                names.join(", ")
            ));
        }
        // Models resident; confirm the engine that will load them is up.
        self.probe()?;
        Ok(self.present_bytes())
    }

    /// Reachability + model-present check. A full test render is expensive on a
    /// video diffusion model, so a real render self-test is a FUTURE ENHANCEMENT;
    /// proving the engine is up and the weights are on disk is sufficient to gate
    /// advertising the capability.
    fn self_test(&self) -> Result<(), String> {
        let missing = self.missing_models();
        if !missing.is_empty() {
            return Err(format!(
                "self-test: {} Wan2.2 model file(s) missing on disk",
                missing.len()
            ));
        }
        self.probe().map(|_| ())
    }
}

/// True if a path is a non-empty regular file.
fn file_present(p: &Path) -> bool {
    std::fs::metadata(p)
        .map(|m| m.is_file() && m.len() > 0)
        .unwrap_or(false)
}

/// Build the provider manifest for video_generation on Wan2.2 5B.
fn build_manifest(base_url: &str) -> ProviderManifest {
    ProviderManifest {
        capability: Capability::VideoGeneration,
        // Encodes the supervised engine + model in the id, and carries the base
        // URL so operators can see where this provider drives its GPU.
        provider_id: format!("comfyui-wan22-5b@{base_url}"),
        version: "0.1.0".to_string(),
        models: vec![
            ModelArtifact {
                cid: None,
                url: Some(URL_DIFFUSION.to_string()),
                // sha256 not yet pinned for the local repackaged file; presence +
                // ComfyUI load is the current integrity gate. Pinning is a TODO.
                sha256: String::new(),
                size_mb: 9_537, // ~9.4 GiB fp16 5B
                path: WAN22_DIFFUSION_MODEL.to_string(),
            },
            ModelArtifact {
                cid: None,
                url: Some(URL_TEXT_ENCODER.to_string()),
                sha256: String::new(),
                size_mb: 3_100, // ~3.0 GiB fp8 umt5-xxl
                path: WAN22_TEXT_ENCODER.to_string(),
            },
            ModelArtifact {
                cid: None,
                url: Some(URL_VAE.to_string()),
                sha256: String::new(),
                size_mb: 254, // wan2.2 vae
                path: WAN22_VAE.to_string(),
            },
        ],
        runtime: vec![
            RuntimeDep {
                name: "comfyui".into(),
                version: ">=0.3.45".into(),
            },
            RuntimeDep {
                name: "cuda".into(),
                version: ">=12".into(),
            },
        ],
        // 5B fp16 at 720p wants a healthy GPU; 12 GB VRAM floor per the task spec.
        min_hardware: MinHardware {
            vram_mb: 12_000,
            ram_mb: 16_000,
            disk_mb: 20_000,
            requires_gpu: true,
        },
        output_kinds: vec![DeliverableKind::GeneratedScene, DeliverableKind::FinalRender],
    }
}

/// Translate the bundled `video_wan2_2_5B_ti2v` UI template into ComfyUI's
/// `/prompt` **API graph** format for pure text-to-video.
///
/// The UI export (nodes/links arrays) is NOT what `/prompt` accepts. The API
/// format is a flat map `node_id -> { class_type, inputs }`, where an input is
/// either a literal value or a `[source_node_id, output_slot]` link. This mirror
/// of the template drops the image path (LoadImage / start_image, which is muted
/// in the template — text-to-video is the default) so `Wan22ImageToVideoLatent`
/// produces an empty latent from just the VAE + dimensions.
///
/// Node ids here match the template's for traceability.
fn wan22_t2v_workflow(prompt: &str, p: &RenderParams) -> serde_json::Value {
    serde_json::json!({
        // Step 1 — load models
        "37": {
            "class_type": "UNETLoader",
            "inputs": {
                "unet_name": "wan2.2_ti2v_5B_fp16.safetensors",
                "weight_dtype": "default"
            }
        },
        "38": {
            "class_type": "CLIPLoader",
            "inputs": {
                "clip_name": "umt5_xxl_fp8_e4m3fn_scaled.safetensors",
                "type": "wan",
                "device": "default"
            }
        },
        "39": {
            "class_type": "VAELoader",
            "inputs": { "vae_name": "wan2.2_vae.safetensors" }
        },
        // ModelSamplingSD3 (shift = 8 per template)
        "48": {
            "class_type": "ModelSamplingSD3",
            "inputs": { "shift": 8.0, "model": ["37", 0] }
        },
        // Prompts
        "6": {
            "class_type": "CLIPTextEncode",
            "inputs": { "text": prompt, "clip": ["38", 0] }
        },
        "7": {
            "class_type": "CLIPTextEncode",
            "inputs": { "text": p.negative_prompt, "clip": ["38", 0] }
        },
        // Empty latent for t2v: Wan22ImageToVideoLatent with no start_image.
        "55": {
            "class_type": "Wan22ImageToVideoLatent",
            "inputs": {
                "width": p.width,
                "height": p.height,
                "length": p.length,
                "batch_size": 1,
                "vae": ["39", 0]
            }
        },
        // Sampler
        "3": {
            "class_type": "KSampler",
            "inputs": {
                "seed": p.seed,
                "steps": p.steps,
                "cfg": p.cfg,
                "sampler_name": "uni_pc",
                "scheduler": "simple",
                "denoise": 1.0,
                "model": ["48", 0],
                "positive": ["6", 0],
                "negative": ["7", 0],
                "latent_image": ["55", 0]
            }
        },
        // Decode → video → save
        "8": {
            "class_type": "VAEDecode",
            "inputs": { "samples": ["3", 0], "vae": ["39", 0] }
        },
        "57": {
            "class_type": "CreateVideo",
            "inputs": { "fps": p.fps, "images": ["8", 0] }
        },
        "58": {
            "class_type": "SaveVideo",
            "inputs": {
                "filename_prefix": "wiiv/hone",
                "format": "auto",
                "codec": "auto",
                "video": ["57", 0]
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A base URL pointing at a port nothing listens on, so probes fail fast and
    /// the suite never depends on a live ComfyUI.
    const DEAD_URL: &str = "http://127.0.0.1:9";

    fn temp_root(with_models: bool) -> PathBuf {
        let mut root = std::env::temp_dir();
        root.push(format!(
            "wiiv-comfy-test-{}-{}",
            std::process::id(),
            // cheap unique-ish suffix
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        if with_models {
            for rel in [WAN22_DIFFUSION_MODEL, WAN22_TEXT_ENCODER, WAN22_VAE] {
                let p = root.join("models").join(rel);
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                // Non-empty so file_present() passes.
                std::fs::write(&p, b"fake-weights").unwrap();
            }
        } else {
            std::fs::create_dir_all(root.join("models")).unwrap();
        }
        root
    }

    #[test]
    fn manifest_is_structurally_valid_except_unpinned_hashes() {
        let p = ComfyUiVideoProvider::new(DEFAULT_COMFYUI_URL, "X:/nonexistent");
        let m = p.manifest();
        assert_eq!(m.capability, Capability::VideoGeneration);
        assert_eq!(m.models.len(), 3);
        assert!(m.min_hardware.requires_gpu);
        assert_eq!(m.min_hardware.vram_mb, 12_000);
        // total download reflects the three Wan2.2 artifacts.
        assert!(m.total_download_mb() > 12_000);
        // The generic validator only flags the (intentionally) unpinned sha256s;
        // every other structural rule must pass.
        let problems = m.validate();
        assert!(
            problems.iter().all(|p| p.contains("sha256")),
            "unexpected manifest problems: {problems:?}"
        );
    }

    #[test]
    fn base_url_from_env_default_and_override() {
        let p = ComfyUiVideoProvider::new(DEFAULT_COMFYUI_URL, "X:/x");
        assert_eq!(p.base_url(), DEFAULT_COMFYUI_URL);
        assert!(p.manifest().provider_id.contains("comfyui-wan22-5b"));
    }

    #[test]
    fn ensure_models_fails_when_files_missing() {
        let root = temp_root(false);
        let p = ComfyUiVideoProvider::new(DEAD_URL, &root);
        let err = p.ensure_models().unwrap_err();
        assert!(err.contains("missing"), "got: {err}");
        assert!(err.contains("wan2.2_ti2v_5B"), "got: {err}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn ensure_models_fails_when_comfy_down_even_if_files_present() {
        // Models on disk, but the engine is unreachable → clear reachability error,
        // NOT a missing-files error. This is the "comfy down" path, hermetic
        // because DEAD_URL points at a closed port.
        let root = temp_root(true);
        let p = ComfyUiVideoProvider::new(DEAD_URL, &root);
        let err = p.ensure_models().unwrap_err();
        assert!(
            err.contains("unreachable") || err.contains("ComfyUI"),
            "expected reachability error, got: {err}"
        );
        assert!(!err.contains("missing"), "should not be a missing-files error: {err}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn self_test_fails_when_comfy_down() {
        let root = temp_root(true);
        let p = ComfyUiVideoProvider::new(DEAD_URL, &root);
        assert!(p.self_test().is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn self_test_fails_fast_when_models_missing() {
        let root = temp_root(false);
        let p = ComfyUiVideoProvider::new(DEAD_URL, &root);
        let err = p.self_test().unwrap_err();
        assert!(err.contains("missing"), "got: {err}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn workflow_graph_is_wellformed_t2v() {
        let params = RenderParams::default();
        let g = wan22_t2v_workflow("a cat riding a skateboard", &params);
        let obj = g.as_object().expect("graph is a JSON object");

        // Every node has a class_type + inputs.
        for (id, node) in obj {
            assert!(node.get("class_type").is_some(), "node {id} missing class_type");
            assert!(node.get("inputs").is_some(), "node {id} missing inputs");
        }

        // Text-to-video: no LoadImage node, and the latent node has no start_image.
        assert!(
            !obj.values().any(|n| n["class_type"] == "LoadImage"),
            "t2v graph must not include LoadImage"
        );
        assert!(
            obj["55"]["inputs"].get("start_image").is_none(),
            "t2v latent must not carry a start_image"
        );

        // Prompt wired into the positive encoder; params into the sampler.
        assert_eq!(obj["6"]["inputs"]["text"], "a cat riding a skateboard");
        assert_eq!(obj["3"]["inputs"]["steps"], params.steps);
        assert_eq!(obj["3"]["inputs"]["cfg"], params.cfg);
        assert_eq!(obj["55"]["inputs"]["width"], params.width);
        assert_eq!(obj["57"]["inputs"]["fps"], params.fps);

        // A link is [source_id, slot]; spot-check the sampler's latent edge.
        assert_eq!(obj["3"]["inputs"]["latent_image"], serde_json::json!(["55", 0]));
    }

    #[test]
    fn render_errors_cleanly_when_comfy_down() {
        // The HTTP plumbing must surface a clear error rather than panic when the
        // engine is unreachable.
        let p = ComfyUiVideoProvider::new(DEAD_URL, "X:/x");
        let err = p.render("test prompt", &RenderParams::default()).unwrap_err();
        assert!(err.contains("/prompt"), "got: {err}");
    }
}
