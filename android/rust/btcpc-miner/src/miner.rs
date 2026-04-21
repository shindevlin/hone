use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use serde::{Deserialize, Serialize};

use crate::MinerState;
use crate::proof;

// ---- chain API types ----

#[derive(Debug, Deserialize)]
struct PhoneModel {
    id: String,
    name: String,
    #[serde(default)]
    size_mb: u64,
    #[serde(default)]
    phone_suitable: bool,
}

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    models: Vec<PhoneModel>,
}

#[derive(Debug, Serialize)]
struct WorkClaim {
    account: String,
    device_type: String,
    model_id: String,
}

#[derive(Debug, Deserialize)]
struct WorkUnit {
    job_id: String,
    prompt: String,
    #[serde(default = "default_max_tokens")]
    max_tokens: u32,
    epoch: u64,
}
fn default_max_tokens() -> u32 { 32 }

#[derive(Debug, Serialize)]
struct WorkResult {
    job_id: String,
    account: String,
    output: String,
    token_count: u32,
    work_hash: String,
    epoch: u64,
    model_id: String,
}

// ---- model download ----

async fn ensure_model_files(
    api_base: &str,
    model_id: &str,
    model_dir: &Path,
    client: &reqwest::Client,
    state: &Arc<MinerState>,
) -> anyhow::Result<(PathBuf, PathBuf)> {
    let model_file    = model_dir.join(format!("{model_id}.onnx"));
    let tokenizer_file= model_dir.join(format!("{model_id}-tokenizer.json"));

    if !model_file.exists() {
        *state.status.lock() = format!("Downloading {model_id} from chain…");
        let url = format!("{api_base}/api/models/{model_id}/onnx/model_q4.onnx");
        download_file(client, &url, &model_file).await
            .map_err(|e| anyhow::anyhow!("model download: {e}"))?;
    }

    if !tokenizer_file.exists() {
        *state.status.lock() = format!("Downloading tokenizer for {model_id}…");
        let url = format!("{api_base}/api/models/{model_id}/tokenizer.json");
        download_file(client, &url, &tokenizer_file).await
            .map_err(|e| anyhow::anyhow!("tokenizer download: {e}"))?;
    }

    Ok((model_file, tokenizer_file))
}

async fn download_file(client: &reqwest::Client, url: &str, dest: &Path) -> anyhow::Result<()> {
    use tokio::io::AsyncWriteExt;
    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        return Err(anyhow::anyhow!("HTTP {}: {}", resp.status(), url));
    }
    let bytes = resp.bytes().await?;
    let mut f = tokio::fs::File::create(dest).await?;
    f.write_all(&bytes).await?;
    Ok(())
}

// ---- ONNX inference via tract ----

fn run_inference_sync(
    model_path: &Path,
    tokenizer_path: &Path,
    prompt: &str,
    max_tokens: u32,
) -> anyhow::Result<(String, u32)> {
    use tract_onnx::prelude::*;

    let tokenizer = tokenizers::Tokenizer::from_file(tokenizer_path)
        .map_err(|e| anyhow::anyhow!("tokenizer: {e}"))?;

    let encoding = tokenizer.encode(prompt, true)
        .map_err(|e| anyhow::anyhow!("encode: {e}"))?;
    let input_ids: Vec<i64> = encoding.get_ids().iter().map(|&x| x as i64).collect();
    let seq_len = input_ids.len();

    let model = tract_onnx::onnx()
        .model_for_path(model_path)?
        .into_optimized()?
        .into_runnable()?;

    // Build input tensor: shape [1, seq_len]
    let input = tract_ndarray::Array2::from_shape_vec(
        (1, seq_len),
        input_ids.clone(),
    )?;
    let input_tensor = TValue::from(tract_core::prelude::Tensor::from(input));

    // Attention mask: all ones
    let mask = tract_ndarray::Array2::<i64>::ones((1, seq_len));
    let mask_tensor = TValue::from(tract_core::prelude::Tensor::from(mask));

    let mut generated: Vec<i64> = input_ids;
    let eos: i64 = 151643; // Qwen EOS; generic models use 2 or 1

    for _ in 0..max_tokens {
        let cur_len = generated.len();
        let inp = tract_ndarray::Array2::from_shape_vec(
            (1, cur_len),
            generated.clone(),
        )?;
        let inp_t = TValue::from(tract_core::prelude::Tensor::from(inp));
        let msk = tract_ndarray::Array2::<i64>::ones((1, cur_len));
        let msk_t = TValue::from(tract_core::prelude::Tensor::from(msk));

        let outputs = model.run(tvec![inp_t, msk_t])?;
        let logits = outputs[0].to_array_view::<f32>()?;
        // logits shape: [1, seq, vocab] — take last token
        let vocab_size = logits.shape()[2];
        let last_row_start = (cur_len - 1) * vocab_size;
        let last_logits: &[f32] = &logits.as_slice().unwrap()
            [last_row_start..last_row_start + vocab_size];

        let next_token = last_logits
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(i, _)| i as i64)
            .unwrap_or(eos);

        if next_token == eos || next_token == 2 { break; }
        generated.push(next_token);
    }

    let new_tokens: Vec<u32> = generated[seq_len..].iter().map(|&x| x as u32).collect();
    let token_count = new_tokens.len() as u32;
    let output_text = tokenizer.decode(&new_tokens, true).unwrap_or_default();

    Ok((output_text, token_count))
}

// ---- main miner loop ----

pub async fn run_miner(
    account: String,
    jwt: String,
    api_base: String,
    model_id: String,
    model_dir: String,
    state: &Arc<MinerState>,
) -> anyhow::Result<()> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()?;

    let dir = PathBuf::from(&model_dir);
    tokio::fs::create_dir_all(&dir).await?;

    // Download model files from the chain
    let (model_path, tokenizer_path) = ensure_model_files(
        &api_base, &model_id, &dir, &client, state,
    ).await?;

    *state.status.lock() = format!("Mining with {model_id}");

    while state.running.load(Ordering::SeqCst) {
        // Claim work
        let claim = client
            .post(format!("{api_base}/api/mining/phone/claim"))
            .bearer_auth(&jwt)
            .json(&WorkClaim {
                account: account.clone(),
                device_type: "android".to_string(),
                model_id: model_id.clone(),
            })
            .send()
            .await;

        let work: WorkUnit = match claim {
            Ok(r) if r.status().is_success() => {
                match r.json().await {
                    Ok(w) => w,
                    Err(_) => {
                        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                        continue;
                    }
                }
            }
            _ => {
                *state.status.lock() = format!("Mining with {model_id} (waiting…)");
                tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                continue;
            }
        };

        *state.status.lock() = format!("Inference: epoch {} ({model_id})", work.epoch);

        // Run inference on a blocking thread (CPU-bound)
        let mp = model_path.clone();
        let tp = tokenizer_path.clone();
        let prompt = work.prompt.clone();
        let max_tok = work.max_tokens;
        let result = tokio::task::spawn_blocking(move || {
            run_inference_sync(&mp, &tp, &prompt, max_tok)
        }).await?;

        let (output_text, token_count) = match result {
            Ok(r) => r,
            Err(e) => {
                log::warn!("Inference failed: {e}");
                *state.status.lock() = format!("Inference error — retrying ({model_id})");
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                continue;
            }
        };

        let work_hash = proof::compute_work_hash(&work.job_id, &output_text, &account);

        let _ = client
            .post(format!("{api_base}/api/mining/phone/submit"))
            .bearer_auth(&jwt)
            .json(&WorkResult {
                job_id: work.job_id.clone(),
                account: account.clone(),
                output: output_text,
                token_count,
                work_hash,
                epoch: work.epoch,
                model_id: model_id.clone(),
            })
            .send()
            .await;

        *state.status.lock() = format!("Proof submitted: epoch {} ({model_id})", work.epoch);
    }

    Ok(())
}
