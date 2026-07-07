//! Inference ENGINE — the model runner (distinct from `inference.rs`, which is
//! the inference-job *marketplace* state machine). This is the ONE place the
//! node executes a model to produce tokens.
//!
//! Before this, the node called an external Ollama daemon over HTTP
//! (`{OLLAMA_URL}/api/chat`) from ~6 scattered sites. That daemon being down is
//! exactly what 503'd bullship at launch. This module fixes both fragilities:
//!
//!   1. **Embedded backend (default):** a candle GGUF model runs IN-PROCESS
//!      (ported from the proven `hone-android/src/llm.rs`). If the node is
//!      alive, inference is alive — no external daemon, no port, no 503.
//!   2. **One module:** all callers go through `chat` / `available`; the backend
//!      choice lives here alone.
//!
//! Backend selection (once, at startup):
//!   - `INFERENCE_URL` (or legacy `OLLAMA_URL`) set → route to that external
//!     OpenAI/Ollama-compatible server (how a GPU node points at vLLM).
//!   - else, built with `inference-embedded` → embedded candle GGUF.
//!   - else → unavailable (relay-only node).
//!
//! See docs/INFERENCE_EMBED_SPEC.md and docs/INFERENCE_MIGRATION_NOTES.md.
#![allow(dead_code)]

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// A chat message (OpenAI/Ollama shape).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

/// A chat request. `model` is advisory for the embedded backend (it serves the
/// single loaded GGUF); it's forwarded verbatim to an external backend.
#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub max_tokens: usize,
}

/// A completed (non-streaming) chat response.
#[derive(Debug, Clone)]
pub struct ChatResponse {
    pub content: String,
}

/// How this node serves inference. Chosen once at startup.
enum Backend {
    /// External OpenAI/Ollama-compatible HTTP server (e.g. vLLM, or a legacy
    /// Ollama). `base` has no trailing slash.
    Http { base: String },
    /// In-process candle GGUF. Always available once the model is resident.
    #[cfg(feature = "inference-embedded")]
    Embedded,
    /// No inference on this node (relay-only).
    None,
}

static BACKEND: OnceLock<Backend> = OnceLock::new();

/// Resolve the backend once. Priority: explicit external URL → embedded → none.
fn backend() -> &'static Backend {
    BACKEND.get_or_init(|| {
        // Explicit external server wins (the vLLM / GPU-node path). Accept the
        // new INFERENCE_URL first, then the deprecated OLLAMA_URL alias.
        let ext = std::env::var("INFERENCE_URL")
            .ok()
            .or_else(|| std::env::var("OLLAMA_URL").ok())
            .map(|u| u.trim_end_matches('/').to_owned())
            .filter(|u| !u.is_empty());
        if let Some(base) = ext {
            tracing::info!("inference-engine: external backend at {base}");
            return Backend::Http { base };
        }
        #[cfg(feature = "inference-embedded")]
        {
            tracing::info!("inference-engine: embedded candle GGUF backend");
            return Backend::Embedded;
        }
        #[allow(unreachable_code)]
        {
            tracing::warn!("inference-engine: no backend (relay-only — no INFERENCE_URL, embedded feature off)");
            Backend::None
        }
    })
}

/// Is inference available on this node right now?
/// - Http: assumed available (the external server is the operator's concern).
/// - Embedded: true once the model is resident.
/// - None: false.
pub fn available() -> bool {
    match backend() {
        Backend::Http { .. } => true,
        #[cfg(feature = "inference-embedded")]
        Backend::Embedded => candle_backend::available(),
        Backend::None => false,
    }
}

/// Non-streaming chat completion. Returns the assistant content.
pub async fn chat(req: ChatRequest) -> Result<ChatResponse> {
    match backend() {
        Backend::Http { base } => http_chat(base, &req).await,
        #[cfg(feature = "inference-embedded")]
        Backend::Embedded => candle_backend::chat(&req).await,
        Backend::None => Err(anyhow!("inference not available on this node")),
    }
}

/// Startup hook — ensure the embedded model is present (downloads once, ~400 MB).
/// No-op for the HTTP/none backends. Background it at boot so it doesn't block.
pub async fn warm_up() {
    #[cfg(feature = "inference-embedded")]
    if matches!(backend(), Backend::Embedded) {
        let _ = candle_backend::ensure_ready().await;
    }
}

// ── External HTTP backend (vLLM / legacy Ollama) ─────────────────────────────
// Preserves the /api/chat request/response shape the old scattered call sites
// used against Ollama, so an Ollama-compatible server is a drop-in.

async fn http_chat(base: &str, req: &ChatRequest) -> Result<ChatResponse> {
    let body = serde_json::json!({
        "model": req.model,
        "messages": req.messages,
        "stream": false,
        "options": { "num_predict": req.max_tokens },
    });
    let http = reqwest::Client::new();
    let resp = http
        .post(format!("{base}/api/chat"))
        .json(&body)
        .send()
        .await?
        .json::<serde_json::Value>()
        .await?;
    // Ollama: { "message": { "content" } }. OpenAI: { "choices":[{"message":{"content"}}] }.
    let content = resp["message"]["content"]
        .as_str()
        .or_else(|| resp["choices"][0]["message"]["content"].as_str())
        .unwrap_or("")
        .to_owned();
    Ok(ChatResponse { content })
}

// ── Embedded candle GGUF backend ─────────────────────────────────────────────
// Ported from hone-android/src/llm.rs — the proven in-process candle path.

#[cfg(feature = "inference-embedded")]
mod candle_backend {
    use super::{ChatRequest, ChatResponse, Message};
    use anyhow::{anyhow, bail, Result};
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;

    // HONE is model-agnostic: there is NO hardcoded default model and the node
    // never auto-downloads one. The operator selects a GGUF that already exists
    // in the central model store, by filename, via HONE_MODEL. With nothing
    // selected, the embedded backend is simply unavailable. See
    // docs/MODEL_REGISTRY_PROTOCOL.md.

    static MODEL_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();

    /// The central model store directory (shared across engines).
    fn model_dir() -> PathBuf {
        std::env::var("HONE_MODEL_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                dirs_next::data_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join("hone")
                    .join("models")
            })
    }

    /// The operator-selected GGUF path, or None if no model has been enabled.
    /// HONE_MODEL is a filename within the store (e.g. "qwen2.5-0.5b-q4.gguf");
    /// an absolute path is also accepted.
    fn model_path() -> &'static Option<PathBuf> {
        MODEL_PATH.get_or_init(|| {
            let sel = std::env::var("HONE_MODEL").ok().filter(|s| !s.trim().is_empty())?;
            let p = PathBuf::from(&sel);
            Some(if p.is_absolute() { p } else { model_dir().join(&sel) })
        })
    }

    /// True once the operator-selected GGUF is present on disk.
    pub fn available() -> bool {
        model_path().as_ref().map(|p| p.exists()).unwrap_or(false)
    }

    /// No-op download: HONE never auto-fetches a model. Readiness means the
    /// operator selected one (HONE_MODEL) and its file is present in the store.
    pub async fn ensure_ready() -> bool {
        match model_path() {
            None => {
                tracing::info!(
                    "inference-engine: no model enabled (set HONE_MODEL to a GGUF in {}); \
                     embedded inference unavailable until the operator enables one",
                    model_dir().display()
                );
                false
            }
            Some(p) if p.exists() => {
                tracing::info!("inference-engine: embedded model ready at {}", p.display());
                true
            }
            Some(p) => {
                tracing::warn!(
                    "inference-engine: HONE_MODEL points at {} which is not present in the store — \
                     enable a model that exists (no auto-download)",
                    p.display()
                );
                false
            }
        }
    }

    /// Flatten chat messages into a prompt. Uses a plain, model-neutral format
    /// (role-labelled turns) rather than a family-specific chat template, so it
    /// works across llama/qwen/etc. GGUFs. A future improvement is to read the
    /// model's own chat_template from GGUF metadata and apply it.
    fn format_prompt(messages: &[Message]) -> String {
        let mut s = String::new();
        for m in messages {
            let label = match m.role.as_str() {
                "system" => "System",
                "assistant" => "Assistant",
                _ => "User",
            };
            s.push_str(&format!("{label}: {}\n", m.content));
        }
        s.push_str("Assistant:");
        s
    }

    pub async fn chat(req: &ChatRequest) -> Result<ChatResponse> {
        let path = match model_path() {
            Some(p) if p.exists() => p.clone(),
            Some(p) => bail!("enabled model {} is not present in the store", p.display()),
            None => bail!("no model enabled — set HONE_MODEL to a GGUF in the model store"),
        };
        let prompt = format_prompt(&req.messages);
        let max_tokens = req.max_tokens.max(1);
        let content =
            tokio::task::spawn_blocking(move || generate_sync(&path, &prompt, max_tokens)).await??;
        Ok(ChatResponse { content })
    }

    /// A loaded quantized model, dispatched by GGUF architecture. Each variant
    /// wraps the per-architecture `ModelWeights` from candle-transformers; the
    /// generation loop only ever touches `forward`, so adding an architecture is
    /// (a) a new variant, (b) a load arm, (c) a `forward` arm — nothing else.
    ///
    /// Why this exists: `quantized_llama::ModelWeights::from_gguf` reads `llama.*`
    /// metadata keys and hard-fails on a Qwen GGUF ("cannot find
    /// llama.attention.head_count in metadata"), because Qwen GGUFs carry
    /// `qwen2.*` / `qwen3.*` keys. candle ships a matching loader per family; we
    /// pick it from `general.architecture`.
    enum LoadedModel {
        Llama(candle_transformers::models::quantized_llama::ModelWeights),
        Qwen2(candle_transformers::models::quantized_qwen2::ModelWeights),
        Qwen3(candle_transformers::models::quantized_qwen3::ModelWeights),
    }

    impl LoadedModel {
        /// Shared forward signature across families: `(input, index_pos) ->
        /// logits`. Each candle loader exposes exactly this, so the generation
        /// loop stays architecture-agnostic.
        fn forward(
            &mut self,
            input: &candle_core::Tensor,
            index_pos: usize,
        ) -> candle_core::Result<candle_core::Tensor> {
            match self {
                LoadedModel::Llama(m) => m.forward(input, index_pos),
                LoadedModel::Qwen2(m) => m.forward(input, index_pos),
                LoadedModel::Qwen3(m) => m.forward(input, index_pos),
            }
        }
    }

    /// Resolve which candle `Device` to run inference on.
    ///
    /// Controlled by `HONE_INFER_DEVICE` (default `"cpu"` when unset):
    ///   - `"cpu"` → `Device::Cpu`.
    ///   - `"cuda"` or `"cuda:N"` → `Device::new_cuda(N)` (default ordinal 0).
    ///
    /// GPU is opt-in, not opt-out — this keeps CPU as the default for both the
    /// chat-serving path and the mining/verification path (`miner.rs`'s
    /// `run_inference_prompt`, which calls through `inference_engine::chat` and
    /// therefore through here too). CPU vs CUDA floating-point results are not
    /// guaranteed bit-identical (different reduction order / kernels), so this
    /// module deliberately does NOT default mining inference to GPU — that
    /// cross-node determinism question is a separate, still-open decision (see
    /// docs/CONTRACT_CONSENSUS_FIX.md, "Coupled Decisions"). An operator who
    /// wants GPU (for serving, or knowingly for mining on a homogeneous fleet)
    /// opts in explicitly via the env var.
    ///
    /// If CUDA is requested but this binary wasn't built with the `cuda`
    /// feature, or `Device::new_cuda` fails at runtime (no GPU / driver issue),
    /// this logs a warning and falls back to `Device::Cpu` rather than
    /// crashing the node.
    fn resolve_device() -> candle_core::Device {
        use candle_core::Device;

        let raw = std::env::var("HONE_INFER_DEVICE").unwrap_or_else(|_| "cpu".to_string());
        let raw = raw.trim();
        if raw.is_empty() || raw.eq_ignore_ascii_case("cpu") {
            return Device::Cpu;
        }

        let lower = raw.to_ascii_lowercase();
        let requested_cuda = if let Some(idx) = lower.strip_prefix("cuda:") {
            match idx.parse::<usize>() {
                Ok(n) => Some(n),
                Err(_) => {
                    tracing::warn!(
                        "inference-engine: HONE_INFER_DEVICE={raw:?} has an unparseable cuda \
                         ordinal — falling back to cpu"
                    );
                    None
                }
            }
        } else if lower == "cuda" {
            Some(0)
        } else {
            tracing::warn!(
                "inference-engine: HONE_INFER_DEVICE={raw:?} is not recognized (expected \
                 \"cpu\", \"cuda\", or \"cuda:N\") — falling back to cpu"
            );
            None
        };

        match requested_cuda {
            Some(ordinal) => match Device::new_cuda(ordinal) {
                Ok(dev) => {
                    tracing::info!("inference-engine: using CUDA device {ordinal} (HONE_INFER_DEVICE={raw:?})");
                    dev
                }
                Err(e) => {
                    tracing::warn!(
                        "inference-engine: HONE_INFER_DEVICE={raw:?} requested but CUDA device \
                         {ordinal} is unavailable ({e}) — falling back to cpu. Build with \
                         `--features cuda` and ensure a working NVIDIA driver/toolkit if GPU \
                         inference is desired."
                    );
                    Device::Cpu
                }
            },
            None => Device::Cpu,
        }
    }

    /// The candle generate loop — ported from hone-android/src/llm.rs. Greedy.
    fn generate_sync(model_path: &Path, prompt: &str, max_tokens: usize) -> Result<String> {
        use candle_core::quantized::gguf_file;
        use candle_core::Tensor;
        // Shared sequence-length cap. candle's Qwen loaders expose no MAX_SEQ_LEN
        // constant; llama's 4096 is a safe general bound across the families here.
        use candle_transformers::models::quantized_llama::MAX_SEQ_LEN;
        use tokenizers::Tokenizer;

        let device = resolve_device();

        let mut file = std::fs::File::open(model_path)?;
        let gguf = gguf_file::Content::read(&mut file)?;

        // Detect the architecture from GGUF metadata and dispatch to the matching
        // candle loader. `gguf` is moved into `from_gguf`, so read the arch first.
        let arch = gguf
            .metadata
            .get("general.architecture")
            .and_then(|v| v.to_string().ok())
            .map(|s| s.to_owned())
            .unwrap_or_default();
        let mut model = match arch.as_str() {
            "qwen2" => LoadedModel::Qwen2(
                candle_transformers::models::quantized_qwen2::ModelWeights::from_gguf(
                    gguf, &mut file, &device,
                )?,
            ),
            "qwen3" => LoadedModel::Qwen3(
                candle_transformers::models::quantized_qwen3::ModelWeights::from_gguf(
                    gguf, &mut file, &device,
                )?,
            ),
            // "llama" and, for back-compat, any GGUF that omits the arch key (the
            // pre-dispatch behaviour) go through the proven llama loader.
            "llama" | "" => LoadedModel::Llama(
                candle_transformers::models::quantized_llama::ModelWeights::from_gguf(
                    gguf, &mut file, &device,
                )?,
            ),
            other => bail!(
                "unsupported GGUF architecture {other:?}: embedded candle backend supports \
                 llama, qwen2, qwen3 (add a LoadedModel variant to extend)"
            ),
        };

        // Tokenizer must live in the store next to the model (no auto-download).
        // Accept either "<model-stem>.tokenizer.json" or a shared "tokenizer.json".
        let dir = model_path.parent().unwrap_or_else(|| Path::new("."));
        let stem = model_path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let per_model = dir.join(format!("{stem}.tokenizer.json"));
        let shared = dir.join("tokenizer.json");
        let tok_path = if per_model.exists() { per_model } else { shared };
        let tokenizer = Tokenizer::from_file(&tok_path)
            .map_err(|e| anyhow!("tokenizer not found beside model at {} ({e}) — \
                place its tokenizer.json in the store next to the GGUF", tok_path.display()))?;

        let encoding = tokenizer.encode(prompt, true).map_err(|e| anyhow!("tokenize: {e}"))?;
        let mut tokens: Vec<u32> = encoding.get_ids().to_vec();
        let vocab = tokenizer.get_vocab(true);
        // Resolve EOS generically across model families: try the common special
        // tokens each vocab might use; if none exist, fall back to token 2 (the
        // llama `</s>` convention). Prevents Qwen-specific hardcoding.
        let eos = ["<|im_end|>", "<|endoftext|>", "</s>", "<|eot_id|>", "<end_of_turn>"]
            .iter()
            .find_map(|t| vocab.get(*t).copied())
            .unwrap_or(2);

        // Standard candle incremental decode with the model's internal KV-cache:
        // prefill the whole prompt once at position 0, then feed ONE new token per
        // step at its actual position (index_pos). The previous code re-fed the
        // full sequence every step while passing index_pos = len-1, which desyncs
        // the KV-cache and RoPE and panics with a broadcast-shape mismatch.
        let mut generated: Vec<u32> = Vec::new();
        let mut next: u32;

        // Prefill.
        {
            let input = Tensor::new(tokens.as_slice(), &device)?.unsqueeze(0)?;
            let logits = model.forward(&input, 0)?.squeeze(0)?;
            // forward may return logits for the whole sequence or just the last
            // position; take the final row either way.
            let last = if logits.dims().len() == 2 {
                logits.get(logits.dim(0)? - 1)?
            } else {
                logits
            };
            next = last.argmax(candle_core::D::Minus1)?.to_scalar::<u32>()?;
        }

        for _ in 0..max_tokens {
            if next == eos {
                break;
            }
            generated.push(next);
            tokens.push(next);
            if tokens.len() >= MAX_SEQ_LEN {
                break;
            }
            // Feed only the new token at its position.
            let index_pos = tokens.len() - 1;
            let input = Tensor::new(&[next], &device)?.unsqueeze(0)?;
            let logits = model.forward(&input, index_pos)?.squeeze(0)?;
            let last = if logits.dims().len() == 2 {
                logits.get(logits.dim(0)? - 1)?
            } else {
                logits
            };
            next = last.argmax(candle_core::D::Minus1)?.to_scalar::<u32>()?;
        }
        // Decode the whole generated sequence at once so the tokenizer can
        // reconstruct spacing correctly (per-token decode drops metaspace).
        let out = tokenizer
            .decode(&generated, true)
            .map_err(|e| anyhow!("decode: {e}"))?;
        Ok(out)
    }
}

// ── Live inference smoke test ────────────────────────────────────────────────
//
// Verifies the real embedded candle path (GGUF load + tokenizer + generation)
// against an operator-selected model, without needing a running server. Gated
// behind env so it only runs when a model is actually available:
//
//   HONE_MODEL_DIR=/mnt/x/hone-models/gguf HONE_MODEL=qwen2.5-0.5b.gguf \
//     cargo +1.90.0 test -p hone-node --release embedded_candle_smoke -- --ignored --nocapture
//
#[cfg(all(test, feature = "inference-embedded"))]
mod smoke {
    use super::*;

    #[tokio::test]
    #[ignore] // requires an operator-selected model present on disk
    async fn embedded_candle_smoke() {
        assert!(
            candle_backend::available(),
            "no model available — set HONE_MODEL_DIR + HONE_MODEL to a GGUF in the store"
        );
        let req = ChatRequest {
            model: std::env::var("HONE_MODEL").unwrap_or_default(),
            messages: vec![Message {
                role: "user".to_string(),
                content: "Reply with exactly: HELLO FROM CANDLE".to_string(),
            }],
            max_tokens: 24,
        };
        let resp = candle_backend::chat(&req).await.expect("candle chat failed");
        eprintln!("=== candle output ===\n{}\n=====================", resp.content);
        assert!(!resp.content.trim().is_empty(), "candle returned empty content");
    }
}
