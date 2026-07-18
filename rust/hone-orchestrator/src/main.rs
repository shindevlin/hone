use hone_api_catalog::{CatalogQuery, CatalogSnapshot, LinkStatus, PublicApiRecord};
use hone_orchestrator::{
    InMemoryOrchestrator, JsonMap, Metadata, OrchestrationStore, ResourceKind, ResourceUsage,
    RuntimeJob, RuntimeJobConfig, RuntimeKind, RuntimeRequirements, RuntimeResource, RuntimeSpan,
    RuntimeWorker, SpanKind, WorkerCapabilities, WorkerStatus,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug)]
struct Config {
    catalog_path: PathBuf,
    category: String,
    query: String,
    worker_id: String,
    runtime_id: String,
    max_bytes: usize,
    timeout_secs: u64,
    allow_auth: bool,
    verified_only: bool,
    out_path: Option<PathBuf>,
}

impl Config {
    fn from_args() -> Result<Self, String> {
        let args: Vec<String> = std::env::args().collect();
        if args.len() == 1 || args.iter().any(|arg| arg == "--help" || arg == "-h") {
            return Err(usage(&args[0]));
        }
        if args.get(1).map(String::as_str) != Some("run-api-tool") {
            return Err(usage(&args[0]));
        }

        let mut config = Self {
            catalog_path: PathBuf::new(),
            category: "Weather".to_string(),
            query: "Open-Meteo".to_string(),
            worker_id: "local-api-worker".to_string(),
            runtime_id: "hone-api-tool-demo".to_string(),
            max_bytes: 16 * 1024,
            timeout_secs: 20,
            allow_auth: false,
            verified_only: false,
            out_path: None,
        };

        let mut index = 2;
        while index < args.len() {
            match args[index].as_str() {
                "--catalog" => {
                    index += 1;
                    config.catalog_path = PathBuf::from(required_value(&args, index, "--catalog")?);
                }
                "--category" => {
                    index += 1;
                    config.category = required_value(&args, index, "--category")?.to_string();
                }
                "--query" => {
                    index += 1;
                    config.query = required_value(&args, index, "--query")?.to_string();
                }
                "--worker" => {
                    index += 1;
                    config.worker_id = required_value(&args, index, "--worker")?.to_string();
                }
                "--runtime" => {
                    index += 1;
                    config.runtime_id = required_value(&args, index, "--runtime")?.to_string();
                }
                "--max-bytes" => {
                    index += 1;
                    config.max_bytes = required_value(&args, index, "--max-bytes")?
                        .parse()
                        .map_err(|_| "--max-bytes must be a positive integer".to_string())?;
                }
                "--timeout-secs" => {
                    index += 1;
                    config.timeout_secs =
                        required_value(&args, index, "--timeout-secs")?
                            .parse()
                            .map_err(|_| "--timeout-secs must be a positive integer".to_string())?;
                }
                "--allow-auth" => config.allow_auth = true,
                "--verified-only" => config.verified_only = true,
                "--out" => {
                    index += 1;
                    config.out_path = Some(PathBuf::from(required_value(&args, index, "--out")?));
                }
                unknown => {
                    return Err(format!(
                        "unknown argument: {unknown}\n\n{}",
                        usage(&args[0])
                    ))
                }
            }
            index += 1;
        }

        if config.catalog_path.as_os_str().is_empty() {
            return Err(format!("missing --catalog\n\n{}", usage(&args[0])));
        }
        if config.max_bytes == 0 {
            return Err("--max-bytes must be greater than zero".to_string());
        }
        Ok(config)
    }
}

#[derive(Serialize)]
struct ApiToolReport {
    mode: &'static str,
    catalog_source_commit: String,
    catalog_record_count: usize,
    catalog_content_hash: String,
    selected_api: SelectedApiReport,
    resource: RuntimeResource,
    job: RuntimeJob,
    worker: RuntimeWorker,
    attempt: hone_orchestrator::RuntimeAttempt,
    span: RuntimeSpan,
    attestation: Option<hone_orchestrator::RuntimeAttestation>,
    failed_attempt_status: Option<String>,
}

#[derive(Serialize)]
struct SelectedApiReport {
    name: String,
    category: String,
    url: String,
    auth: String,
    source_line: usize,
    risk_flags: Vec<String>,
    prior_verification_status: LinkStatus,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = match Config::from_args() {
        Ok(config) => config,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };

    let out_path = config.out_path.clone();
    let report = run_api_tool_job(config).await?;
    let json = serde_json::to_string_pretty(&report)?;
    if let Some(out_path) = out_path {
        std::fs::write(&out_path, json.as_bytes())?;
        eprintln!("wrote {}", out_path.display());
    } else {
        println!("{json}");
    }
    Ok(())
}

async fn run_api_tool_job(config: Config) -> Result<ApiToolReport, Box<dyn std::error::Error>> {
    let snapshot = CatalogSnapshot::load_json(&config.catalog_path)?;
    let catalog_hash = snapshot.content_hash()?;
    let record = select_api_record(&snapshot, &config).ok_or_else(|| {
        format!(
            "no catalog record matched category={} query={}",
            config.category, config.query
        )
    })?;

    let selected = SelectedApiReport {
        name: record.name.clone(),
        category: record.category.clone(),
        url: record.url.clone(),
        auth: format!("{:?}", record.auth),
        source_line: record.source_line,
        risk_flags: record.risk_flags.clone(),
        prior_verification_status: record.verification.status.clone(),
    };

    let now = now_unix_secs();
    let mut resource_metadata = Metadata::new();
    resource_metadata.insert("repo".to_string(), snapshot.source.repo_url.clone());
    resource_metadata.insert("source_path".to_string(), snapshot.source.path.clone());
    resource_metadata.insert(
        "record_count".to_string(),
        snapshot.records.len().to_string(),
    );
    let resource = RuntimeResource::new(
        ResourceKind::ApiCatalogSnapshot,
        format!("sha256:{catalog_hash}"),
        snapshot.source.commit.clone(),
        now,
        resource_metadata,
    );

    let mut requirements = RuntimeRequirements::for_kind(RuntimeKind::ApiTool);
    requirements.api_categories.insert(record.category.clone());

    let mut job_metadata = Metadata::new();
    job_metadata.insert("api_name".to_string(), record.name.clone());
    job_metadata.insert("api_url".to_string(), record.url.clone());
    job_metadata.insert("api_category".to_string(), record.category.clone());
    job_metadata.insert("source_line".to_string(), record.source_line.to_string());
    let input_commitment = sha256_hex(format!(
        "{}\n{}\n{}\n{}",
        record.name, record.category, record.url, catalog_hash
    ));
    let job = RuntimeJob::new(
        &config.runtime_id,
        RuntimeKind::ApiTool,
        input_commitment,
        vec![resource.resource_id.clone()],
        requirements,
        RuntimeJobConfig {
            timeout_secs: Some(config.timeout_secs),
            unresponsive_secs: Some(config.timeout_secs / 2),
            max_attempts: 1,
            challenge_window_secs: 120,
        },
        format!("{}:{}", snapshot.source.commit, record.source_line),
        now,
        job_metadata,
    );

    let worker = worker_for(&config.worker_id, &record.category);
    let mut store = InMemoryOrchestrator::default();
    store.publish_resource(resource.clone());
    store.register_worker(worker.clone());
    store.enqueue_job(job.clone());
    let attempt = store
        .claim_next(&worker.worker_id, now + 1)?
        .ok_or("worker did not claim the queued API tool job")?;

    let probe = probe_url(&record.url, config.max_bytes, config.timeout_secs).await;
    let mut attributes = JsonMap::new();
    attributes.insert("api.name".to_string(), record.name.clone().into());
    attributes.insert("api.category".to_string(), record.category.clone().into());
    attributes.insert("api.url".to_string(), record.url.clone().into());
    attributes.insert(
        "catalog.commit".to_string(),
        snapshot.source.commit.clone().into(),
    );
    attributes.insert("catalog.hash".to_string(), catalog_hash.clone().into());
    attributes.insert("catalog.source_line".to_string(), record.source_line.into());

    match probe {
        Ok(probe) => {
            attributes.insert("http.status".to_string(), probe.status_code.into());
            attributes.insert("http.final_url".to_string(), probe.final_url.clone().into());
            attributes.insert("http.elapsed_ms".to_string(), probe.elapsed_ms.into());
            attributes.insert("http.bytes_read".to_string(), probe.bytes_read.into());
            attributes.insert("http.truncated".to_string(), probe.truncated.into());
            let output_commitment = sha256_hex(format!(
                "{}\n{}\n{}\n{}\n{}",
                record.url, probe.final_url, probe.status_code, probe.body_hash, probe.bytes_read
            ));
            let mut span = RuntimeSpan::new(
                format!("trace:{}", attempt.attempt_id),
                None,
                &attempt.job_id,
                &attempt.attempt_id,
                SpanKind::ApiRequest,
                format!("probe {}", record.name),
                now + 2,
                attributes,
            );
            span.ended_at_unix_secs = Some(now + 3);
            span.output_commitment = Some(output_commitment.clone());
            store.record_span(span.clone())?;
            let attestation = if (200..400).contains(&probe.status_code) {
                Some(store.complete_attempt(
                    &attempt.attempt_id,
                    &output_commitment,
                    ResourceUsage {
                        cpu_ms: probe.elapsed_ms,
                        memory_peak_bytes: probe.bytes_read,
                        network_rx_bytes: probe.bytes_read,
                        network_tx_bytes: record.url.len() as u64,
                    },
                    now + 4,
                )?)
            } else {
                store.fail_attempt(
                    &attempt.attempt_id,
                    hone_orchestrator::AttemptStatus::Failed,
                    now + 4,
                )?;
                None
            };
            let failed_attempt_status = if attestation.is_some() {
                None
            } else {
                Some(format!("http_status_{}", probe.status_code))
            };
            let final_job = store.get_job(&attempt.job_id).cloned().unwrap_or(job);
            let final_attempt = store
                .get_attempt(&attempt.attempt_id)
                .cloned()
                .unwrap_or(attempt);
            return Ok(ApiToolReport {
                mode: "run-api-tool",
                catalog_source_commit: snapshot.source.commit,
                catalog_record_count: snapshot.records.len(),
                catalog_content_hash: catalog_hash,
                selected_api: selected,
                resource,
                job: final_job,
                worker,
                attempt: final_attempt,
                span,
                attestation,
                failed_attempt_status,
            });
        }
        Err(error) => {
            attributes.insert("error".to_string(), error.clone().into());
            let mut span = RuntimeSpan::new(
                format!("trace:{}", attempt.attempt_id),
                None,
                &attempt.job_id,
                &attempt.attempt_id,
                SpanKind::ApiRequest,
                format!("probe {}", record.name),
                now + 2,
                attributes,
            );
            span.ended_at_unix_secs = Some(now + 3);
            span.error = Some(error.clone());
            store.record_span(span.clone())?;
            store.fail_attempt(
                &attempt.attempt_id,
                hone_orchestrator::AttemptStatus::Failed,
                now + 4,
            )?;
            let final_job = store.get_job(&attempt.job_id).cloned().unwrap_or(job);
            let final_attempt = store
                .get_attempt(&attempt.attempt_id)
                .cloned()
                .unwrap_or(attempt);
            return Ok(ApiToolReport {
                mode: "run-api-tool",
                catalog_source_commit: snapshot.source.commit,
                catalog_record_count: snapshot.records.len(),
                catalog_content_hash: catalog_hash,
                selected_api: selected,
                resource,
                job: final_job,
                worker,
                attempt: final_attempt,
                span,
                attestation: None,
                failed_attempt_status: Some(error),
            });
        }
    };
}

struct ProbeResult {
    status_code: u16,
    final_url: String,
    elapsed_ms: u64,
    bytes_read: u64,
    body_hash: String,
    truncated: bool,
}

async fn probe_url(url: &str, max_bytes: usize, timeout_secs: u64) -> Result<ProbeResult, String> {
    let client = reqwest::Client::builder()
        .user_agent("hone-orchestratord/0.1 (+https://honemesh.net)")
        .timeout(Duration::from_secs(timeout_secs))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|error| error.to_string())?;
    let started = Instant::now();
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    let status_code = response.status().as_u16();
    let final_url = response.url().to_string();
    let mut body = Vec::new();
    let mut truncated = false;

    while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
        if body.len() + chunk.len() > max_bytes {
            let remaining = max_bytes.saturating_sub(body.len());
            body.extend_from_slice(&chunk[..remaining]);
            truncated = true;
            break;
        }
        body.extend_from_slice(&chunk);
    }

    Ok(ProbeResult {
        status_code,
        final_url,
        elapsed_ms: started.elapsed().as_millis() as u64,
        bytes_read: body.len() as u64,
        body_hash: sha256_hex(&body),
        truncated,
    })
}

fn select_api_record<'a>(
    snapshot: &'a CatalogSnapshot,
    config: &Config,
) -> Option<&'a PublicApiRecord> {
    let categories = vec![config.category.as_str()];
    let results = snapshot.search(CatalogQuery {
        text: Some(&config.query),
        categories,
        secretless_only: if config.allow_auth { None } else { Some(true) },
        https_only: true,
        limit: None,
    });

    results
        .into_iter()
        .filter(|record| {
            !config.verified_only
                || matches!(
                    record.verification.status,
                    LinkStatus::Alive | LinkStatus::Redirected
                )
        })
        .min_by_key(|record| {
            let exact = !record.name.eq_ignore_ascii_case(&config.query);
            let prior_verified = !matches!(
                record.verification.status,
                LinkStatus::Alive | LinkStatus::Redirected
            );
            (
                exact,
                prior_verified,
                record.risk_flags.len(),
                record.source_line,
            )
        })
}

fn worker_for(worker_id: &str, category: &str) -> RuntimeWorker {
    RuntimeWorker {
        worker_id: worker_id.to_string(),
        account: worker_id.to_string(),
        endpoint: None,
        capabilities: WorkerCapabilities {
            runtime_kinds: BTreeSet::from([RuntimeKind::ApiTool]),
            models: BTreeSet::new(),
            api_categories: BTreeSet::from([category.to_string()]),
            max_concurrent_jobs: 1,
        },
        stake_hunits: 1_000_000,
        status: WorkerStatus::Active,
        metadata: Metadata::new(),
    }
}

fn required_value<'a>(args: &'a [String], index: usize, flag: &str) -> Result<&'a str, String> {
    args.get(index)
        .map(String::as_str)
        .filter(|value| !value.starts_with("--"))
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn sha256_hex(input: impl AsRef<[u8]>) -> String {
    hex::encode(Sha256::digest(input.as_ref()))
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

fn usage(binary: &str) -> String {
    format!(
        "Usage:\n  {binary} run-api-tool --catalog <snapshot.json> [--category Weather] [--query Open-Meteo] [--worker local-api-worker] [--runtime hone-api-tool-demo] [--max-bytes 16384] [--timeout-secs 20] [--verified-only] [--allow-auth] [--out report.json]\n\nExample:\n  {binary} run-api-tool --catalog /mnt/btcpc-storage/catalogs/public-apis.snapshot.json --category Weather --query Open-Meteo --out /tmp/api-tool-report.json"
    )
}
