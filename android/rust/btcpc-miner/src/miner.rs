use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use serde::{Deserialize, Serialize};

use crate::MinerState;
use crate::proof;

#[derive(Debug, Serialize)]
struct WorkClaim {
    account: String,
    device_type: String,
    model_hint: String,
}

#[derive(Debug, Deserialize)]
struct WorkUnit {
    job_id: String,
    prompt: String,
    max_tokens: u32,
    epoch: u64,
}

#[derive(Debug, Serialize)]
struct WorkResult {
    job_id: String,
    account: String,
    output: String,
    token_count: u32,
    work_hash: String,
    epoch: u64,
}

pub async fn run_miner(
    account: String,
    jwt: String,
    api_base: String,
    model_dir: String,
    state: &Arc<MinerState>,
) -> anyhow::Result<()> {
    use candle_core::{Device, Tensor};
    use candle_transformers::models::qwen2::{Config as QwenConfig, ModelForCausalLM};
    use candle_nn::VarBuilder;
    use hf_hub::{api::tokio::Api, Repo, RepoType};

    let device = Device::Cpu;

    // Download or load model from cache
    *state.status.lock() = "Downloading model…".to_string();
    let model_path = PathBuf::from(&model_dir);
    let model_file = model_path.join("model.safetensors");
    let config_file = model_path.join("config.json");
    let tokenizer_file = model_path.join("tokenizer.json");

    if !model_file.exists() {
        let api = Api::new()?;
        let repo = api.repo(Repo::new(
            "Qwen/Qwen2.5-0.5B-Instruct".to_string(),
            RepoType::Model,
        ));
        *state.status.lock() = "Downloading Qwen2.5-0.5B…".to_string();
        let _ = repo.get("model.safetensors").await?;
        let _ = repo.get("config.json").await?;
        let _ = repo.get("tokenizer.json").await?;
    }

    *state.status.lock() = "Loading model…".to_string();
    let config: QwenConfig = serde_json::from_reader(std::fs::File::open(&config_file)?)?;
    let vb = unsafe { VarBuilder::from_mmaped_safetensors(&[&model_file], candle_core::DType::F32, &device)? };
    let mut model = ModelForCausalLM::new(&config, vb)?;

    let tokenizer = tokenizers::Tokenizer::from_file(&tokenizer_file)
        .map_err(|e| anyhow::anyhow!("Tokenizer: {e}"))?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    *state.status.lock() = format!("Mining — account: {account}");

    while state.running.load(Ordering::SeqCst) {
        // Claim a work unit
        let claim_resp = client
            .post(format!("{api_base}/api/mining/phone/claim"))
            .bearer_auth(&jwt)
            .json(&WorkClaim {
                account: account.clone(),
                device_type: "android".to_string(),
                model_hint: "qwen2.5-0.5b".to_string(),
            })
            .send()
            .await;

        let work: WorkUnit = match claim_resp {
            Ok(r) if r.status().is_success() => {
                match r.json::<WorkUnit>().await {
                    Ok(w) => w,
                    Err(_) => {
                        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                        continue;
                    }
                }
            }
            _ => {
                *state.status.lock() = format!("Mining (waiting for work…)");
                tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                continue;
            }
        };

        *state.status.lock() = format!("Running inference (epoch {})", work.epoch);

        // Tokenize
        let encoding = tokenizer.encode(work.prompt.as_str(), true)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let tokens: Vec<u32> = encoding.get_ids().to_vec();
        let input = Tensor::new(tokens.as_slice(), &device)?.unsqueeze(0)?;

        // Run inference
        let logits = model.forward(&input, 0)?;
        let output_ids = sample_greedy(&logits, work.max_tokens, &tokenizer, &device, &mut model)?;
        let output_text = tokenizer.decode(&output_ids, true).unwrap_or_default();
        let token_count = output_ids.len() as u32;

        // Build proof
        let work_hash = proof::compute_work_hash(&work.job_id, &output_text, &account);

        // Submit result
        let result = WorkResult {
            job_id: work.job_id.clone(),
            account: account.clone(),
            output: output_text,
            token_count,
            work_hash,
            epoch: work.epoch,
        };

        let _ = client
            .post(format!("{api_base}/api/mining/phone/submit"))
            .bearer_auth(&jwt)
            .json(&result)
            .send()
            .await;

        *state.status.lock() = format!("Submitted proof (epoch {})", work.epoch);
    }

    Ok(())
}

fn sample_greedy(
    logits: &candle_core::Tensor,
    max_new: u32,
    tokenizer: &tokenizers::Tokenizer,
    device: &candle_core::Device,
    model: &mut candle_transformers::models::qwen2::ModelForCausalLM,
) -> anyhow::Result<Vec<u32>> {
    use candle_core::IndexOp;
    let mut output_ids = Vec::new();
    let mut last_logits = logits.clone();

    for _ in 0..max_new {
        let next_token = last_logits
            .i((.., last_logits.dim(1)? - 1, ..))?
            .argmax(candle_core::D::Minus1)?
            .to_scalar::<u32>()?;
        if next_token == 151643 { // EOS for Qwen
            break;
        }
        output_ids.push(next_token);
        let new_input = candle_core::Tensor::new(&[next_token], device)?.unsqueeze(0)?;
        last_logits = model.forward(&new_input, output_ids.len())?;
    }
    Ok(output_ids)
}
