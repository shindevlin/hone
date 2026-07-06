//! On-device inference using candle GGUF engine.
//! Downloads model from HuggingFace Hub on first use, then runs locally.

use anyhow::{bail, Result};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use parking_lot::Mutex;

const HF_REPO:    &str = "Qwen/Qwen2.5-0.5B-Instruct-GGUF";
const GGUF_FILE:  &str = "qwen2.5-0.5b-instruct-q4_k_m.gguf";
const TOKENIZER_REPO: &str = "Qwen/Qwen2.5-0.5B-Instruct";

pub struct LlmEngine {
    model_path: PathBuf,
    status:     Arc<Mutex<String>>,
    ready:      bool,
}

impl LlmEngine {
    pub fn new(model_dir: &str, status: Arc<Mutex<String>>) -> Self {
        LlmEngine {
            model_path: PathBuf::from(model_dir).join(GGUF_FILE),
            status,
            ready: false,
        }
    }

    /// Ensure the model is present, downloading if needed.
    /// Returns true when the model is ready to use.
    pub async fn ensure_ready(&mut self) -> bool {
        if self.ready {
            return true;
        }
        if self.model_path.exists() {
            self.ready = true;
            return true;
        }
        // Download in background — caller can check status string.
        self.download().await
    }

    async fn download(&mut self) -> bool {
        *self.status.lock() = "Downloading model (qwen2.5-0.5b, ~400 MB)…".to_owned();

        let model_dir = self.model_path.parent().unwrap().to_path_buf();
        let model_path = self.model_path.clone();
        let status = self.status.clone();

        let result = tokio::task::spawn_blocking(move || {
            let api = hf_hub::api::sync::ApiBuilder::new()
                .with_cache_dir(model_dir)
                .build()?;
            let repo = api.model(HF_REPO.to_owned());
            repo.get(GGUF_FILE)?;
            anyhow::Ok(())
        }).await;

        match result {
            Ok(Ok(())) => {
                *self.status.lock() = "Model ready".to_owned();
                self.ready = true;
                true
            }
            Ok(Err(e)) => {
                *self.status.lock() = format!("Model download failed: {e}");
                false
            }
            Err(e) => {
                *self.status.lock() = format!("Model download task failed: {e}");
                false
            }
        }
    }

    /// Run inference on the given prompt. Returns the generated text.
    pub async fn generate(&self, prompt: &str, max_tokens: usize) -> Result<String> {
        if !self.ready {
            bail!("model not ready");
        }

        let model_path = self.model_path.clone();
        let prompt = prompt.to_owned();

        tokio::task::spawn_blocking(move || {
            Self::generate_sync(&model_path, &prompt, max_tokens)
        }).await?
    }

    fn generate_sync(model_path: &Path, prompt: &str, max_tokens: usize) -> Result<String> {
        use candle_core::{Device, Tensor};
        use candle_transformers::models::quantized_llama::{ModelWeights, MAX_SEQ_LEN};
        use candle_core::quantized::gguf_file;
        use tokenizers::Tokenizer;

        let device = Device::Cpu;

        // Load GGUF weights
        let mut file = std::fs::File::open(model_path)?;
        let gguf = gguf_file::Content::read(&mut file)?;
        let mut model = ModelWeights::from_gguf(gguf, &mut file, &device)?;

        // Load tokenizer from cached HF hub
        let tokenizer_path = model_path
            .parent()
            .unwrap()
            .join("tokenizer.json");

        // Try to load tokenizer from model dir; if not present, use a fallback
        let tokenizer = if tokenizer_path.exists() {
            Tokenizer::from_file(&tokenizer_path)
                .map_err(|e| anyhow::anyhow!("tokenizer load: {e}"))?
        } else {
            // Download tokenizer inline
            let api = hf_hub::api::sync::Api::new()?;
            let repo = api.model(TOKENIZER_REPO.to_owned());
            let tok_file = repo.get("tokenizer.json")?;
            // Cache it next to the model for next time
            let _ = std::fs::copy(&tok_file, &tokenizer_path);
            Tokenizer::from_file(&tok_file)
                .map_err(|e| anyhow::anyhow!("tokenizer load: {e}"))?
        };

        // Encode prompt
        let encoding = tokenizer.encode(prompt, true)
            .map_err(|e| anyhow::anyhow!("tokenize: {e}"))?;
        let mut tokens: Vec<u32> = encoding.get_ids().to_vec();

        let eos_token = tokenizer
            .get_vocab(true)
            .get("<|endoftext|>")
            .copied()
            .unwrap_or(151643);

        let mut generated = String::new();

        for i in 0..max_tokens {
            let input = Tensor::new(tokens.as_slice(), &device)?
                .unsqueeze(0)?;
            let logits = model.forward(&input, tokens.len().saturating_sub(1))?;
            let logits = logits.squeeze(0)?;

            // Greedy decode
            let next_token = logits
                .argmax(candle_core::D::Minus1)?
                .to_scalar::<u32>()?;

            if next_token == eos_token { break; }

            let text = tokenizer
                .decode(&[next_token], true)
                .map_err(|e| anyhow::anyhow!("decode: {e}"))?;
            generated.push_str(&text);
            tokens.push(next_token);

            if tokens.len() >= MAX_SEQ_LEN { break; }
        }

        Ok(generated)
    }
}
