use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use serde::{Deserialize, Serialize};
use once_cell::sync::OnceCell;

use crate::MinerState;
use crate::proof;

// Cached tract model — loaded once per model_id, reused across work units.
// Avoids the 2-3s load time on every inference job.
struct CachedModel {
    plan: tract_onnx::prelude::SimplePlan<
        tract_onnx::prelude::TypedFact,
        Box<dyn tract_onnx::prelude::TypedOp>,
        tract_onnx::prelude::Graph<tract_onnx::prelude::TypedFact, Box<dyn tract_onnx::prelude::TypedOp>>,
    >,
    tokenizer: tokenizers::Tokenizer,
}

static MODEL_CACHE: OnceCell<std::sync::Mutex<Option<CachedModel>>> = OnceCell::new();

fn model_cache() -> &'static std::sync::Mutex<Option<CachedModel>> {
    MODEL_CACHE.get_or_init(|| std::sync::Mutex::new(None))
}

// ---- posting-key signing ----
// Signs {account, job_id, work_hash} (keys sorted alphabetically, canonical JSON)
// with SHA256 → secp256k1 ECDSA, matching keyManager.signTransaction on the server.
fn sign_work_proof(account: &str, job_id: &str, work_hash: &str, posting_key_hex: &str) -> Option<String> {
    use k256::ecdsa::{SigningKey, signature::hazmat::PrehashSigner};
    use sha2::{Sha256, Digest};

    let key_bytes = hex::decode(posting_key_hex).ok()?;
    let signing_key = SigningKey::from_bytes(key_bytes.as_slice().into()).ok()?;

    // Server canonical: JSON.stringify(data, Object.keys(data).sort())
    // Keys in alphabetical order: account < job_id < work_hash
    let canonical = format!(
        r#"{{"account":"{}","job_id":"{}","work_hash":"{}"}}"#,
        account, job_id, work_hash
    );
    let hash = Sha256::digest(canonical.as_bytes());

    let (sig, _recovery): (k256::ecdsa::Signature, _) =
        signing_key.sign_prehash_recoverable(&hash).ok()?;
    Some(hex::encode(sig.to_bytes()))
}

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
    signature: String,
}

// ---- model download ----

fn make_download_client() -> reqwest::Client {
    reqwest::Client::builder()
        .tcp_keepalive(std::time::Duration::from_secs(30))
        .build()
        .expect("download client")
}

// ---- manifest structs ----

#[derive(Debug, Deserialize)]
struct ChunkEntry {
    index: u32,
    cid: String,
    size: u64,
    url: String,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum FileEntry {
    Chunked {
        #[allow(dead_code)]
        chunked: bool,
        chunk_size: u64,
        total_size: u64,
        chunks: Vec<ChunkEntry>,
    },
    Single {
        #[allow(dead_code)]
        chunked: Option<bool>,
        url: Option<String>,
        #[allow(dead_code)]
        cid: Option<String>,
        #[allow(dead_code)]
        size: Option<u64>,
    },
}

#[derive(Debug, Deserialize)]
struct ModelManifest {
    #[allow(dead_code)]
    model_id: String,
    files: std::collections::HashMap<String, FileEntry>,
}

async fn fetch_manifest(
    client: &reqwest::Client,
    api_base: &str,
    model_id: &str,
) -> anyhow::Result<ModelManifest> {
    let url = format!("{api_base}/api/models/{model_id}/manifest");
    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        return Err(anyhow::anyhow!("manifest HTTP {}", resp.status()));
    }
    Ok(resp.json::<ModelManifest>().await?)
}

// Download a chunked file. Each chunk is saved as <dest>.chunk.<N>, then all
// are assembled into <dest>.tmp and renamed. Existing chunk files are skipped
// so any connection drop resumes from the last incomplete chunk.
async fn download_chunks(
    client: &reqwest::Client,
    chunks: &[ChunkEntry],
    total_size: u64,
    dest: &Path,
    label: &str,
    state: &Arc<MinerState>,
) -> anyhow::Result<()> {
    use sha2::{Sha256, Digest};
    use tokio::io::AsyncWriteExt;

    let tmp = dest.with_extension("tmp");

    for chunk in chunks {
        let chunk_file = dest.with_extension(format!("chunk.{}", chunk.index));

        // Skip if already downloaded and correct size
        if chunk_file.exists() {
            if let Ok(meta) = tokio::fs::metadata(&chunk_file).await {
                if meta.len() == chunk.size {
                    *state.status.lock() = format!(
                        "Chunk {}/{} already downloaded, skipping",
                        chunk.index + 1, chunks.len()
                    );
                    continue;
                }
            }
            let _ = tokio::fs::remove_file(&chunk_file).await;
        }

        // Download this chunk with retries
        let mut attempts = 0u32;
        loop {
            attempts += 1;
            *state.status.lock() = format!(
                "Downloading {} chunk {}/{} ({} MB total)",
                label,
                chunk.index + 1,
                chunks.len(),
                total_size / 1_000_000
            );

            let resp = match client.get(&chunk.url).send().await {
                Ok(r) => r,
                Err(e) => {
                    if attempts >= 10 { return Err(e.into()); }
                    tokio::time::sleep(std::time::Duration::from_secs((attempts * 10).min(60) as u64)).await;
                    continue;
                }
            };

            if !resp.status().is_success() {
                if attempts >= 5 { return Err(anyhow::anyhow!("chunk HTTP {}", resp.status())); }
                tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                continue;
            }

            let bytes = match resp.bytes().await {
                Ok(b) => b,
                Err(e) => {
                    if attempts >= 10 { return Err(e.into()); }
                    tokio::time::sleep(std::time::Duration::from_secs((attempts * 10).min(60) as u64)).await;
                    continue;
                }
            };

            // Verify SHA-256
            let got_cid = hex::encode(Sha256::digest(&bytes));
            if got_cid != chunk.cid {
                if attempts >= 5 {
                    return Err(anyhow::anyhow!("chunk {} hash mismatch", chunk.index));
                }
                continue;
            }

            let mut f = tokio::fs::File::create(&chunk_file).await?;
            f.write_all(&bytes).await?;
            f.flush().await?;
            break;
        }
    }

    // Assemble all chunks into .tmp
    *state.status.lock() = format!("Assembling {} ({} chunks)…", label, chunks.len());
    {
        let mut out = tokio::fs::File::create(&tmp).await?;
        for chunk in chunks {
            let chunk_file = dest.with_extension(format!("chunk.{}", chunk.index));
            let data = tokio::fs::read(&chunk_file).await?;
            out.write_all(&data).await?;
        }
        out.flush().await?;
    }

    tokio::fs::rename(&tmp, dest).await?;

    // Clean up chunk files
    for chunk in chunks {
        let chunk_file = dest.with_extension(format!("chunk.{}", chunk.index));
        let _ = tokio::fs::remove_file(&chunk_file).await;
    }

    Ok(())
}

async fn ensure_model_files(
    api_base: &str,
    model_id: &str,
    model_dir: &Path,
    state: &Arc<MinerState>,
) -> anyhow::Result<(PathBuf, PathBuf)> {
    let model_file     = model_dir.join(format!("{model_id}.onnx"));
    let tokenizer_file = model_dir.join(format!("{model_id}-tokenizer.json"));
    let dl = make_download_client();

    // Try manifest-based download first (chunk-aware, network-distributed)
    match fetch_manifest(&dl, api_base, model_id).await {
        Ok(manifest) => {
            if !model_file.exists() {
                // Prefer plain model.onnx (encoder-only, works with tract) then quantized variants
                match manifest.files.get("onnx/model.onnx")
                    .or_else(|| manifest.files.get("onnx/model_q4.onnx"))
                    .or_else(|| manifest.files.get("onnx/model_q4f16.onnx"))
                {
                    Some(FileEntry::Chunked { chunks, total_size, .. }) => {
                        *state.status.lock() = format!(
                            "Downloading {} in {} chunks from network…",
                            model_id, chunks.len()
                        );
                        download_chunks(&dl, chunks, *total_size, &model_file, model_id, state).await
                            .map_err(|e| anyhow::anyhow!("chunked model download failed: {e}"))?;
                    }
                    Some(FileEntry::Single { url: Some(url), .. }) => {
                        download_resumable(&dl, url, &model_file, model_id, state).await
                            .map_err(|e| anyhow::anyhow!("model download failed: {e}"))?;
                    }
                    _ => {
                        // Manifest exists but no matching file entry — try model.onnx then q4
                        let url = format!("{api_base}/api/models/{model_id}/onnx/model.onnx");
                        let r = download_resumable(&dl, &url, &model_file, model_id, state).await;
                        if r.is_err() {
                            let url2 = format!("{api_base}/api/models/{model_id}/onnx/model_q4.onnx");
                            download_resumable(&dl, &url2, &model_file, model_id, state).await
                                .map_err(|e| anyhow::anyhow!("model download failed: {e}"))?;
                        }
                    }
                }
            }

            if !tokenizer_file.exists() {
                match manifest.files.get("tokenizer.json") {
                    Some(FileEntry::Single { url: Some(url), .. }) => {
                        download_resumable(&dl, url, &tokenizer_file, "tokenizer", state).await
                            .map_err(|e| anyhow::anyhow!("tokenizer download failed: {e}"))?;
                    }
                    _ => {
                        let url = format!("{api_base}/api/models/{model_id}/tokenizer.json");
                        download_resumable(&dl, &url, &tokenizer_file, "tokenizer", state).await
                            .map_err(|e| anyhow::anyhow!("tokenizer download failed: {e}"))?;
                    }
                }
            }
        }
        Err(_) => {
            // No manifest endpoint — try plain model.onnx first, then quantized
            if !model_file.exists() {
                let url = format!("{api_base}/api/models/{model_id}/onnx/model.onnx");
                let r = download_resumable(&dl, &url, &model_file, model_id, state).await;
                if r.is_err() {
                    let url2 = format!("{api_base}/api/models/{model_id}/onnx/model_q4.onnx");
                    download_resumable(&dl, &url2, &model_file, model_id, state).await
                        .map_err(|e| anyhow::anyhow!("model download failed: {e}"))?;
                }
            }
            if !tokenizer_file.exists() {
                let url = format!("{api_base}/api/models/{model_id}/tokenizer.json");
                download_resumable(&dl, &url, &tokenizer_file, "tokenizer", state).await
                    .map_err(|e| anyhow::anyhow!("tokenizer download failed: {e}"))?;
            }
        }
    }

    Ok((model_file, tokenizer_file))
}

// Resumable download using HTTP Range requests. The .tmp file is preserved
// across connection drops so each retry picks up where it left off.
async fn download_resumable(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    label: &str,
    state: &Arc<MinerState>,
) -> anyhow::Result<()> {
    use futures::StreamExt;
    use tokio::io::{AsyncSeekExt, AsyncWriteExt};

    let tmp = dest.with_extension("tmp");

    // How many bytes do we already have from a previous attempt?
    let already = if tmp.exists() {
        tokio::fs::metadata(&tmp).await.map(|m| m.len()).unwrap_or(0)
    } else {
        0
    };

    // HEAD first to learn the total size (skip if already complete)
    let total = {
        let head = client.head(url).send().await;
        match head {
            Ok(r) if r.status().is_success() => r.content_length().unwrap_or(0),
            _ => 0,
        }
    };

    if total > 0 && already >= total {
        // Already fully downloaded — just rename
        tokio::fs::rename(&tmp, dest).await?;
        return Ok(());
    }

    let mut attempts = 0u32;
    loop {
        attempts += 1;
        let offset = if tmp.exists() {
            tokio::fs::metadata(&tmp).await.map(|m| m.len()).unwrap_or(0)
        } else {
            0
        };

        if total > 0 {
            *state.status.lock() = format!(
                "Downloading {} — {} / {} MB{}",
                label,
                offset / 1_000_000,
                total / 1_000_000,
                if attempts > 1 { format!(" (attempt {})", attempts) } else { String::new() }
            );
        } else {
            *state.status.lock() = format!("Downloading {} — {} MB", label, offset / 1_000_000);
        }

        let mut req = client.get(url);
        if offset > 0 {
            req = req.header("Range", format!("bytes={}-", offset));
        }

        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                if attempts >= 20 { return Err(e.into()); }
                let delay = (attempts * 15).min(120);
                *state.status.lock() = format!("Connect failed — retrying in {delay}s ({label})");
                tokio::time::sleep(std::time::Duration::from_secs(delay as u64)).await;
                continue;
            }
        };

        let status = resp.status();
        // 206 = partial content (range accepted), 200 = server ignored range
        if status == 416 {
            // Requested range not satisfiable — file on server may be smaller; start over
            let _ = tokio::fs::remove_file(&tmp).await;
            continue;
        }
        if !status.is_success() {
            return Err(anyhow::anyhow!("HTTP {}: {}", status, url));
        }

        // If server returned 200 (ignored our Range), restart the file
        let append = status.as_u16() == 206;
        let mut f = if append {
            let mut file = tokio::fs::OpenOptions::new()
                .create(true).append(true).open(&tmp).await?;
            file.seek(std::io::SeekFrom::End(0)).await?;
            file
        } else {
            tokio::fs::File::create(&tmp).await?
        };

        let mut written: u64 = if append { offset } else { 0 };
        let mut stream = resp.bytes_stream();
        let mut ok = true;

        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    if let Err(e) = f.write_all(&bytes).await {
                        *state.status.lock() = format!("Write error: {e} — retrying");
                        ok = false;
                        break;
                    }
                    written += bytes.len() as u64;
                    if total > 0 {
                        *state.status.lock() = format!(
                            "Downloading {} — {} / {} MB",
                            label,
                            written / 1_000_000,
                            total / 1_000_000
                        );
                    } else {
                        *state.status.lock() = format!(
                            "Downloading {} — {} MB",
                            label,
                            written / 1_000_000
                        );
                    }
                }
                Err(e) => {
                    *state.status.lock() = format!("Connection dropped — resuming ({label}): {e}");
                    ok = false;
                    break;
                }
            }
        }

        if !ok {
            // Flush what we have and retry — Range header will resume from here
            let _ = f.flush().await;
            drop(f);
            if attempts >= 20 { return Err(anyhow::anyhow!("too many retries")); }
            let delay = (attempts * 10).min(60) as u64;
            tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
            continue;
        }

        f.flush().await?;
        drop(f);
        tokio::fs::rename(&tmp, dest).await?;
        return Ok(());
    }
}

// ---- ONNX inference via tract ----

fn load_model(model_path: &Path, tokenizer_path: &Path) -> anyhow::Result<CachedModel> {
    use tract_onnx::prelude::*;

    // Use concrete input shape so tract doesn't choke on symbolic KV-cache dims.
    // We run the model as a plain encoder (BERT/MiniLM style): two inputs,
    // both fixed at [1, MAX_SEQ] and padded at runtime. No past_key_values.
    const MAX_SEQ: usize = 128;

    let plan = tract_onnx::onnx()
        .model_for_path(model_path)?
        .with_input_fact(0, InferenceFact::dt_shape(i64::datum_type(), tvec![1usize, MAX_SEQ]))?
        .with_input_fact(1, InferenceFact::dt_shape(i64::datum_type(), tvec![1usize, MAX_SEQ]))?
        .into_optimized()?
        .into_runnable()?;

    let tokenizer = tokenizers::Tokenizer::from_file(tokenizer_path)
        .map_err(|e| anyhow::anyhow!("tokenizer: {e}"))?;

    Ok(CachedModel { plan, tokenizer })
}

fn run_inference_sync(
    model_path: &Path,
    tokenizer_path: &Path,
    prompt: &str,
    _max_tokens: u32,
) -> anyhow::Result<(String, u32)> {
    use tract_onnx::prelude::*;

    const MAX_SEQ: usize = 128;

    // Ensure model is loaded — reuse across work units
    {
        let mut guard = model_cache().lock().unwrap();
        if guard.is_none() {
            *guard = Some(load_model(model_path, tokenizer_path)?);
        }
    }

    let guard = model_cache().lock().unwrap();
    let cached = guard.as_ref().unwrap();

    // Tokenise and pad/truncate to MAX_SEQ
    let encoding = cached.tokenizer.encode(prompt, true)
        .map_err(|e| anyhow::anyhow!("encode: {e}"))?;
    let mut ids: Vec<i64> = encoding.get_ids().iter().map(|&x| x as i64).collect();
    ids.truncate(MAX_SEQ);
    let seq_actual = ids.len();
    ids.resize(MAX_SEQ, 0i64); // pad with [PAD] = 0

    // Attention mask: 1 for real tokens, 0 for padding
    let mut mask: Vec<i64> = vec![0i64; MAX_SEQ];
    for i in 0..seq_actual { mask[i] = 1; }

    let ids_t = tract_ndarray::Array2::from_shape_vec((1, MAX_SEQ), ids)?;
    let msk_t = tract_ndarray::Array2::from_shape_vec((1, MAX_SEQ), mask)?;

    let outputs = cached.plan.run(tvec![
        TValue::from(Tensor::from(ids_t)),
        TValue::from(Tensor::from(msk_t)),
    ])?;

    // Take the [CLS] token embedding (output[0], position 0) as the compute proof.
    // Shape: [1, seq, hidden] or [1, hidden] depending on model.
    let out = outputs[0].to_array_view::<f32>()?;
    let flat: Vec<f32> = out.iter().take(64).cloned().collect();
    let token_count = seq_actual as u32;

    // Encode embedding as hex string — becomes the "output_text" in the work hash
    let bytes: Vec<u8> = flat.iter().flat_map(|f| f.to_le_bytes()).collect();
    let output_text = hex::encode(&bytes);

    Ok((output_text, token_count))
}

// ---- main miner loop ----

pub async fn run_miner(
    account: String,
    jwt: String,
    api_base: String,
    model_id: String,
    model_dir: String,
    posting_key: String,
    state: &Arc<MinerState>,
) -> anyhow::Result<()> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()?;

    let dir = PathBuf::from(&model_dir);
    tokio::fs::create_dir_all(&dir).await?;

    // Download model files — retries internally; loop here handles permanent failures gracefully
    let (model_path, tokenizer_path) = loop {
        match ensure_model_files(&api_base, &model_id, &dir, state).await {
            Ok(paths) => break paths,
            Err(e) => {
                *state.status.lock() = format!("Model download failed: {e} — retrying in 60s");
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                if !state.running.load(Ordering::SeqCst) { return Ok(()); }
            }
        }
    };

    // Invalidate any cached model from a previous model_id — paths may have changed
    { *model_cache().lock().unwrap() = None; }

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

        let signature = sign_work_proof(&account, &work.job_id, &work_hash, &posting_key)
            .unwrap_or_default();
        if signature.is_empty() {
            *state.status.lock() = "Signing failed — check posting key in Settings".to_string();
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            continue;
        }

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
                signature,
            })
            .send()
            .await;

        *state.status.lock() = format!("Proof submitted: epoch {} ({model_id})", work.epoch);
    }

    Ok(())
}
