//! Rust-native HONE runtime orchestration primitives.
//!
//! This crate intentionally does not import Agent Lightning. It keeps the useful
//! control-plane concepts that HONE needs: resources, jobs, attempts, spans,
//! attestations, worker capabilities, and retry/failover state transitions.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

pub type Metadata = BTreeMap<String, String>;
pub type JsonMap = BTreeMap<String, serde_json::Value>;

#[derive(Debug, Error, Eq, PartialEq)]
pub enum OrchestratorError {
    #[error("worker not found: {0}")]
    WorkerNotFound(String),
    #[error("job not found: {0}")]
    JobNotFound(String),
    #[error("attempt not found: {0}")]
    AttemptNotFound(String),
    #[error("job {0} is not claimable")]
    JobNotClaimable(String),
    #[error("worker {worker_id} lacks capabilities for job {job_id}")]
    CapabilityMismatch { worker_id: String, job_id: String },
    #[error("attempt {0} is already terminal")]
    AttemptAlreadyTerminal(String),
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub enum RuntimeKind {
    HttpService,
    BackgroundWorker,
    Inference,
    ApiTool,
    AgentTask,
    WasmModule,
    OciContainer,
    Other(String),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ResourceKind {
    PromptTemplate,
    ModelWeights,
    ServiceManifest,
    ApiCatalogSnapshot,
    ContainerImage,
    WasmModule,
    Policy,
    Other(String),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeResource {
    pub resource_id: String,
    pub kind: ResourceKind,
    pub content_cid: String,
    pub version: String,
    pub created_at_unix_secs: u64,
    pub metadata: Metadata,
}

impl RuntimeResource {
    pub fn new(
        kind: ResourceKind,
        content_cid: impl AsRef<str>,
        version: impl AsRef<str>,
        created_at_unix_secs: u64,
        metadata: Metadata,
    ) -> Self {
        let content_cid = content_cid.as_ref().to_string();
        let version = version.as_ref().to_string();
        let kind_key = format!("{kind:?}");
        let resource_id = stable_id(
            "resource",
            [kind_key.as_str(), content_cid.as_str(), version.as_str()],
        );
        Self {
            resource_id,
            kind,
            content_cid,
            version,
            created_at_unix_secs,
            metadata,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeRequirements {
    pub runtime_kinds: BTreeSet<RuntimeKind>,
    pub models: BTreeSet<String>,
    pub api_categories: BTreeSet<String>,
    pub min_stake_hunits: u64,
}

impl RuntimeRequirements {
    pub fn for_kind(kind: RuntimeKind) -> Self {
        Self {
            runtime_kinds: BTreeSet::from([kind]),
            models: BTreeSet::new(),
            api_categories: BTreeSet::new(),
            min_stake_hunits: 0,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorkerCapabilities {
    pub runtime_kinds: BTreeSet<RuntimeKind>,
    pub models: BTreeSet<String>,
    pub api_categories: BTreeSet<String>,
    pub max_concurrent_jobs: u32,
}

impl WorkerCapabilities {
    pub fn supports(&self, requirements: &RuntimeRequirements, stake_hunits: u64) -> bool {
        requirements
            .runtime_kinds
            .iter()
            .all(|kind| self.runtime_kinds.contains(kind))
            && requirements
                .models
                .iter()
                .all(|model| self.models.contains(model))
            && requirements
                .api_categories
                .iter()
                .all(|category| self.api_categories.contains(category))
            && stake_hunits >= requirements.min_stake_hunits
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum WorkerStatus {
    Active,
    Draining,
    Suspended,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeWorker {
    pub worker_id: String,
    pub account: String,
    pub endpoint: Option<String>,
    pub capabilities: WorkerCapabilities,
    pub stake_hunits: u64,
    pub status: WorkerStatus,
    pub metadata: Metadata,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum JobStatus {
    Queued,
    Claimed,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    Requeued,
    Challenged,
    Settled,
}

impl JobStatus {
    pub fn is_claimable(&self) -> bool {
        matches!(self, Self::Queued | Self::Requeued)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeJobConfig {
    pub timeout_secs: Option<u64>,
    pub unresponsive_secs: Option<u64>,
    pub max_attempts: u32,
    pub challenge_window_secs: u64,
}

impl Default for RuntimeJobConfig {
    fn default() -> Self {
        Self {
            timeout_secs: Some(300),
            unresponsive_secs: Some(60),
            max_attempts: 1,
            challenge_window_secs: 120,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeJob {
    pub job_id: String,
    pub runtime_id: String,
    pub kind: RuntimeKind,
    pub input_commitment: String,
    pub resource_ids: Vec<String>,
    pub requirements: RuntimeRequirements,
    pub config: RuntimeJobConfig,
    pub status: JobStatus,
    pub created_at_unix_secs: u64,
    pub metadata: Metadata,
}

impl RuntimeJob {
    pub fn new(
        runtime_id: impl AsRef<str>,
        kind: RuntimeKind,
        input_commitment: impl AsRef<str>,
        resource_ids: Vec<String>,
        requirements: RuntimeRequirements,
        config: RuntimeJobConfig,
        nonce: impl AsRef<str>,
        created_at_unix_secs: u64,
        metadata: Metadata,
    ) -> Self {
        let runtime_id = runtime_id.as_ref().to_string();
        let input_commitment = input_commitment.as_ref().to_string();
        let resource_join = resource_ids.join(",");
        let job_id = stable_id(
            "job",
            [
                runtime_id.as_str(),
                &format!("{kind:?}"),
                input_commitment.as_str(),
                resource_join.as_str(),
                nonce.as_ref(),
            ],
        );
        Self {
            job_id,
            runtime_id,
            kind,
            input_commitment,
            resource_ids,
            requirements,
            config,
            status: JobStatus::Queued,
            created_at_unix_secs,
            metadata,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AttemptStatus {
    Preparing,
    Running,
    Failed,
    Succeeded,
    Timeout,
    Unresponsive,
}

impl AttemptStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Failed | Self::Succeeded | Self::Timeout | Self::Unresponsive
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeAttempt {
    pub attempt_id: String,
    pub job_id: String,
    pub sequence_id: u32,
    pub worker_id: String,
    pub status: AttemptStatus,
    pub started_at_unix_secs: u64,
    pub ended_at_unix_secs: Option<u64>,
    pub last_heartbeat_unix_secs: Option<u64>,
    pub trace_root: Option<String>,
    pub attestation_id: Option<String>,
    pub metadata: Metadata,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SpanKind {
    Message,
    ToolCall,
    Inference,
    ApiRequest,
    RuntimeOperation,
    RewardSignal,
    Exception,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeSpan {
    pub span_id: String,
    pub trace_id: String,
    pub parent_span_id: Option<String>,
    pub job_id: String,
    pub attempt_id: String,
    pub kind: SpanKind,
    pub name: String,
    pub started_at_unix_secs: u64,
    pub ended_at_unix_secs: Option<u64>,
    pub attributes: JsonMap,
    pub input_commitment: Option<String>,
    pub output_commitment: Option<String>,
    pub error: Option<String>,
}

impl RuntimeSpan {
    pub fn new(
        trace_id: impl AsRef<str>,
        parent_span_id: Option<String>,
        job_id: impl AsRef<str>,
        attempt_id: impl AsRef<str>,
        kind: SpanKind,
        name: impl AsRef<str>,
        started_at_unix_secs: u64,
        attributes: JsonMap,
    ) -> Self {
        let trace_id = trace_id.as_ref().to_string();
        let job_id = job_id.as_ref().to_string();
        let attempt_id = attempt_id.as_ref().to_string();
        let name = name.as_ref().to_string();
        let span_id = stable_id(
            "span",
            [
                trace_id.as_str(),
                job_id.as_str(),
                attempt_id.as_str(),
                name.as_str(),
                &started_at_unix_secs.to_string(),
            ],
        );
        Self {
            span_id,
            trace_id,
            parent_span_id,
            job_id,
            attempt_id,
            kind,
            name,
            started_at_unix_secs,
            ended_at_unix_secs: None,
            attributes,
            input_commitment: None,
            output_commitment: None,
            error: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResourceUsage {
    pub cpu_ms: u64,
    pub memory_peak_bytes: u64,
    pub network_rx_bytes: u64,
    pub network_tx_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeAttestation {
    pub attestation_id: String,
    pub job_id: String,
    pub attempt_id: String,
    pub worker_id: String,
    pub output_commitment: String,
    pub trace_root: String,
    pub resource_usage: ResourceUsage,
    pub signed_by: Option<String>,
    pub signature: Option<String>,
    pub created_at_unix_secs: u64,
}

pub trait OrchestrationStore {
    fn publish_resource(&mut self, resource: RuntimeResource);
    fn register_worker(&mut self, worker: RuntimeWorker);
    fn enqueue_job(&mut self, job: RuntimeJob);
    fn claim_next(
        &mut self,
        worker_id: &str,
        now_unix_secs: u64,
    ) -> Result<Option<RuntimeAttempt>, OrchestratorError>;
    fn record_span(&mut self, span: RuntimeSpan) -> Result<(), OrchestratorError>;
    fn complete_attempt(
        &mut self,
        attempt_id: &str,
        output_commitment: &str,
        resource_usage: ResourceUsage,
        now_unix_secs: u64,
    ) -> Result<RuntimeAttestation, OrchestratorError>;
    fn fail_attempt(
        &mut self,
        attempt_id: &str,
        status: AttemptStatus,
        now_unix_secs: u64,
    ) -> Result<(), OrchestratorError>;
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct InMemoryOrchestrator {
    pub resources: BTreeMap<String, RuntimeResource>,
    pub workers: BTreeMap<String, RuntimeWorker>,
    pub jobs: BTreeMap<String, RuntimeJob>,
    pub attempts: BTreeMap<String, RuntimeAttempt>,
    pub spans: BTreeMap<String, RuntimeSpan>,
    pub attestations: BTreeMap<String, RuntimeAttestation>,
    attempt_sequences: BTreeMap<String, u32>,
}

impl InMemoryOrchestrator {
    pub fn get_job(&self, job_id: &str) -> Option<&RuntimeJob> {
        self.jobs.get(job_id)
    }

    pub fn get_attempt(&self, attempt_id: &str) -> Option<&RuntimeAttempt> {
        self.attempts.get(attempt_id)
    }
}

impl OrchestrationStore for InMemoryOrchestrator {
    fn publish_resource(&mut self, resource: RuntimeResource) {
        self.resources
            .insert(resource.resource_id.clone(), resource);
    }

    fn register_worker(&mut self, worker: RuntimeWorker) {
        self.workers.insert(worker.worker_id.clone(), worker);
    }

    fn enqueue_job(&mut self, job: RuntimeJob) {
        self.jobs.insert(job.job_id.clone(), job);
    }

    fn claim_next(
        &mut self,
        worker_id: &str,
        now_unix_secs: u64,
    ) -> Result<Option<RuntimeAttempt>, OrchestratorError> {
        let worker = self
            .workers
            .get(worker_id)
            .ok_or_else(|| OrchestratorError::WorkerNotFound(worker_id.to_string()))?;

        if worker.status != WorkerStatus::Active {
            return Ok(None);
        }

        let job_id = self
            .jobs
            .iter()
            .find(|(_, job)| {
                job.status.is_claimable()
                    && worker
                        .capabilities
                        .supports(&job.requirements, worker.stake_hunits)
            })
            .map(|(job_id, _)| job_id.clone());

        let Some(job_id) = job_id else {
            return Ok(None);
        };

        let sequence = self.attempt_sequences.entry(job_id.clone()).or_insert(0);
        *sequence += 1;
        let attempt_id = stable_id(
            "attempt",
            [job_id.as_str(), worker_id, &sequence.to_string()],
        );
        let attempt = RuntimeAttempt {
            attempt_id: attempt_id.clone(),
            job_id: job_id.clone(),
            sequence_id: *sequence,
            worker_id: worker_id.to_string(),
            status: AttemptStatus::Preparing,
            started_at_unix_secs: now_unix_secs,
            ended_at_unix_secs: None,
            last_heartbeat_unix_secs: Some(now_unix_secs),
            trace_root: None,
            attestation_id: None,
            metadata: Metadata::new(),
        };

        let job = self
            .jobs
            .get_mut(&job_id)
            .ok_or_else(|| OrchestratorError::JobNotFound(job_id.clone()))?;
        job.status = JobStatus::Claimed;
        self.attempts.insert(attempt_id, attempt.clone());
        Ok(Some(attempt))
    }

    fn record_span(&mut self, span: RuntimeSpan) -> Result<(), OrchestratorError> {
        let attempt = self
            .attempts
            .get_mut(&span.attempt_id)
            .ok_or_else(|| OrchestratorError::AttemptNotFound(span.attempt_id.clone()))?;
        if attempt.status.is_terminal() {
            return Err(OrchestratorError::AttemptAlreadyTerminal(
                span.attempt_id.clone(),
            ));
        }
        attempt.status = AttemptStatus::Running;
        attempt.last_heartbeat_unix_secs =
            Some(span.ended_at_unix_secs.unwrap_or(span.started_at_unix_secs));
        attempt.trace_root = Some(update_trace_root(
            attempt.trace_root.as_deref(),
            &span.span_id,
        ));
        if let Some(job) = self.jobs.get_mut(&span.job_id) {
            job.status = JobStatus::Running;
        }
        self.spans.insert(span.span_id.clone(), span);
        Ok(())
    }

    fn complete_attempt(
        &mut self,
        attempt_id: &str,
        output_commitment: &str,
        resource_usage: ResourceUsage,
        now_unix_secs: u64,
    ) -> Result<RuntimeAttestation, OrchestratorError> {
        let attempt = self
            .attempts
            .get_mut(attempt_id)
            .ok_or_else(|| OrchestratorError::AttemptNotFound(attempt_id.to_string()))?;
        if attempt.status.is_terminal() {
            return Err(OrchestratorError::AttemptAlreadyTerminal(
                attempt_id.to_string(),
            ));
        }
        let trace_root = attempt
            .trace_root
            .clone()
            .unwrap_or_else(|| stable_id("trace", [attempt.job_id.as_str(), attempt_id]));
        let attestation_id = stable_id(
            "attest",
            [
                attempt.job_id.as_str(),
                attempt_id,
                attempt.worker_id.as_str(),
                output_commitment,
                trace_root.as_str(),
            ],
        );
        let attestation = RuntimeAttestation {
            attestation_id: attestation_id.clone(),
            job_id: attempt.job_id.clone(),
            attempt_id: attempt_id.to_string(),
            worker_id: attempt.worker_id.clone(),
            output_commitment: output_commitment.to_string(),
            trace_root,
            resource_usage,
            signed_by: None,
            signature: None,
            created_at_unix_secs: now_unix_secs,
        };
        attempt.status = AttemptStatus::Succeeded;
        attempt.ended_at_unix_secs = Some(now_unix_secs);
        attempt.attestation_id = Some(attestation_id.clone());
        if let Some(job) = self.jobs.get_mut(&attempt.job_id) {
            job.status = JobStatus::Succeeded;
        }
        self.attestations
            .insert(attestation_id, attestation.clone());
        Ok(attestation)
    }

    fn fail_attempt(
        &mut self,
        attempt_id: &str,
        status: AttemptStatus,
        now_unix_secs: u64,
    ) -> Result<(), OrchestratorError> {
        let attempt = self
            .attempts
            .get_mut(attempt_id)
            .ok_or_else(|| OrchestratorError::AttemptNotFound(attempt_id.to_string()))?;
        if attempt.status.is_terminal() {
            return Err(OrchestratorError::AttemptAlreadyTerminal(
                attempt_id.to_string(),
            ));
        }
        let terminal_status = if status.is_terminal() {
            status
        } else {
            AttemptStatus::Failed
        };
        attempt.status = terminal_status;
        attempt.ended_at_unix_secs = Some(now_unix_secs);
        if let Some(job) = self.jobs.get_mut(&attempt.job_id) {
            let attempts_used = self
                .attempt_sequences
                .get(&attempt.job_id)
                .copied()
                .unwrap_or(1);
            if attempts_used < job.config.max_attempts {
                job.status = JobStatus::Requeued;
            } else {
                job.status = JobStatus::Failed;
            }
        }
        Ok(())
    }
}

pub fn stable_id<'a>(prefix: &str, parts: impl IntoIterator<Item = &'a str>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(prefix.as_bytes());
    for part in parts {
        hasher.update([0x1f]);
        hasher.update(part.as_bytes());
    }
    let digest = hex::encode(hasher.finalize());
    format!("{prefix}_{}", &digest[..32])
}

fn update_trace_root(previous_root: Option<&str>, span_id: &str) -> String {
    match previous_root {
        Some(root) => stable_id("trace", [root, span_id]),
        None => stable_id("trace", [span_id]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worker(worker_id: &str, kinds: &[RuntimeKind]) -> RuntimeWorker {
        RuntimeWorker {
            worker_id: worker_id.to_string(),
            account: format!("{worker_id}-account"),
            endpoint: None,
            capabilities: WorkerCapabilities {
                runtime_kinds: kinds.iter().cloned().collect(),
                models: BTreeSet::new(),
                api_categories: BTreeSet::from(["Weather".to_string()]),
                max_concurrent_jobs: 4,
            },
            stake_hunits: 1_000,
            status: WorkerStatus::Active,
            metadata: Metadata::new(),
        }
    }

    #[test]
    fn stable_job_ids_are_deterministic() {
        let requirements = RuntimeRequirements::for_kind(RuntimeKind::ApiTool);
        let first = RuntimeJob::new(
            "runtime-a",
            RuntimeKind::ApiTool,
            "input-sha256",
            vec!["resource-a".to_string()],
            requirements.clone(),
            RuntimeJobConfig::default(),
            "nonce-1",
            1,
            Metadata::new(),
        );
        let second = RuntimeJob::new(
            "runtime-a",
            RuntimeKind::ApiTool,
            "input-sha256",
            vec!["resource-a".to_string()],
            requirements,
            RuntimeJobConfig::default(),
            "nonce-1",
            999,
            Metadata::new(),
        );
        assert_eq!(first.job_id, second.job_id);
    }

    #[test]
    fn claim_next_respects_worker_capabilities() {
        let mut store = InMemoryOrchestrator::default();
        store.register_worker(worker("api-worker", &[RuntimeKind::ApiTool]));
        store.register_worker(worker("wasm-worker", &[RuntimeKind::WasmModule]));

        let mut requirements = RuntimeRequirements::for_kind(RuntimeKind::ApiTool);
        requirements.api_categories.insert("Weather".to_string());
        let job = RuntimeJob::new(
            "runtime-a",
            RuntimeKind::ApiTool,
            "input-sha256",
            vec![],
            requirements,
            RuntimeJobConfig::default(),
            "nonce-1",
            10,
            Metadata::new(),
        );
        let job_id = job.job_id.clone();
        store.enqueue_job(job);

        assert!(store.claim_next("wasm-worker", 11).unwrap().is_none());
        let attempt = store.claim_next("api-worker", 12).unwrap().unwrap();
        assert_eq!(attempt.job_id, job_id);
        assert_eq!(store.get_job(&job_id).unwrap().status, JobStatus::Claimed);
    }

    #[test]
    fn spans_and_completion_emit_attestation() {
        let mut store = InMemoryOrchestrator::default();
        store.register_worker(worker("api-worker", &[RuntimeKind::ApiTool]));
        let job = RuntimeJob::new(
            "runtime-a",
            RuntimeKind::ApiTool,
            "input-sha256",
            vec![],
            RuntimeRequirements::for_kind(RuntimeKind::ApiTool),
            RuntimeJobConfig::default(),
            "nonce-1",
            10,
            Metadata::new(),
        );
        store.enqueue_job(job);
        let attempt = store.claim_next("api-worker", 12).unwrap().unwrap();
        let span = RuntimeSpan::new(
            "trace-a",
            None,
            &attempt.job_id,
            &attempt.attempt_id,
            SpanKind::ApiRequest,
            "open-meteo request",
            13,
            JsonMap::new(),
        );
        store.record_span(span).unwrap();
        let attestation = store
            .complete_attempt(
                &attempt.attempt_id,
                "output-sha256",
                ResourceUsage {
                    cpu_ms: 10,
                    memory_peak_bytes: 1024,
                    network_rx_bytes: 100,
                    network_tx_bytes: 50,
                },
                20,
            )
            .unwrap();
        assert_eq!(attestation.worker_id, "api-worker");
        assert_eq!(
            store.get_attempt(&attempt.attempt_id).unwrap().status,
            AttemptStatus::Succeeded
        );
    }
}
