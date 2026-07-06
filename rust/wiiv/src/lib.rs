//! Wiiv — HONE's decentralized rendering platform.
//!
//! Modality-agnostic render jobs (image / 3D / audio / video / composite) with a
//! milestone-based production lifecycle, worker capability discovery, escrow, and
//! dispute settlement. See `docs/WIIV_PROTOCOL.md` for the full spec.
//!
//! This crate is the Rust-side source of truth for the shapes a future
//! `WiivRenderJobPost` ledger entry serializes. It intentionally mirrors the
//! off-chain MCP render layer in `src/mcp/` (mediaJobSchema.js / videoPlanCompiler.js)
//! so an off-chain plan maps 1:1 onto an on-chain job.
//!
//! Pure data + state-machine logic only: no chain wiring yet. Live posting stays
//! dry-run behind the guardrail described in the protocol doc until scoped auth and
//! chain routes exist.

use serde::{Deserialize, Serialize};

pub mod api;
pub mod comfy;
pub mod worker;

/// Schema version for the canonical render-job document. Bump on breaking changes.
pub const SCHEMA_VERSION: u32 = 1;

/// Reserved protocol accounts (seeded in genesis with no keys).
pub const ACCOUNT_WIIV: &str = "wiiv";
pub const ACCOUNT_WIIV_ESCROW: &str = "wiiv-escrow";

// ── Modalities ──────────────────────────────────────────────────────────────

/// The render modality a job targets. Open set — `Composite` fans out across the
/// others. New modalities are introduced by the capability classes workers
/// advertise, not by changing this enum's contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderModality {
    Image,
    Video,
    Audio,
    Threed,
    Composite,
}

/// Worker capability classes. A job requires some subset; a worker advertises some
/// subset via `WiivWorkerRegister`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    TextInference,
    ImageGeneration,
    VideoGeneration,
    ThreedGeneration,
    VoiceSynthesis,
    MusicGeneration,
    EditingAssembly,
    Upscaling,
    Subtitles,
    Storage,
    Review,
}

/// Deliverable kinds an artifact can carry. Mirrors `DELIVERABLE_KINDS` in the JS
/// schema; must stay in sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliverableKind {
    CreativeBrief,
    Script,
    Storyboard,
    ShotList,
    GeneratedScene,
    GeneratedImage,
    GeneratedModel,
    Voiceover,
    Music,
    SoundDesign,
    EditAssembly,
    Captions,
    ColorGrade,
    Upscale,
    FinalRender,
    ProjectBundle,
    Provenance,
}

// ── Lifecycle ───────────────────────────────────────────────────────────────

/// Top-level render-job status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Drafted,
    Quoted,
    Funded,
    InProduction,
    InReview,
    Accepted,
    Settled,
    Disputed,
    Cancelled,
}

impl JobState {
    /// Legal top-level transitions. Anything not listed is rejected.
    pub fn can_transition_to(self, next: JobState) -> bool {
        use JobState::*;
        matches!(
            (self, next),
            (Drafted, Quoted)
                | (Drafted, Cancelled)
                | (Quoted, Funded)
                | (Quoted, Drafted)
                | (Quoted, Cancelled)
                | (Funded, InProduction)
                | (Funded, Cancelled)
                | (InProduction, InReview)
                | (InProduction, Disputed)
                | (InProduction, Cancelled)
                | (InReview, Accepted)
                | (InReview, InProduction)
                | (InReview, Disputed)
                | (Accepted, Settled)
                | (Accepted, Disputed)
                | (Disputed, InProduction)
                | (Disputed, Settled)
                | (Disputed, Cancelled)
        )
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, JobState::Settled | JobState::Cancelled)
    }
}

/// Per-milestone status. Lets long-form jobs settle piecewise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MilestoneState {
    Pending,
    Active,
    Delivered,
    Accepted,
    RevisionRequested,
    Rejected,
}

impl MilestoneState {
    pub fn can_transition_to(self, next: MilestoneState) -> bool {
        use MilestoneState::*;
        matches!(
            (self, next),
            (Pending, Active)
                | (Active, Delivered)
                | (Active, Rejected)
                | (Delivered, Accepted)
                | (Delivered, RevisionRequested)
                | (Delivered, Rejected)
                | (RevisionRequested, Active)
        )
    }
}

// ── Documents ───────────────────────────────────────────────────────────────

/// A single milestone in a render job's production graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Milestone {
    pub milestone_id: String,
    pub name: String,
    pub deliverable: String,
    #[serde(default)]
    pub deliverable_kinds: Vec<DeliverableKind>,
    #[serde(default)]
    pub required_capabilities: Vec<Capability>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    pub status: MilestoneState,
    #[serde(default)]
    pub revisions: u32,
}

/// Buyer budget/escrow view for a job. `max_hunits` is a hard ceiling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Budget {
    pub max_hunits: Option<u64>,
    #[serde(default)]
    pub committed_hunits: u64,
    #[serde(default)]
    pub released_hunits: u64,
}

/// The canonical render-job document. This is what a `WiivRenderJobPost` serializes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderJob {
    pub schema_version: u32,
    pub job_id: String,
    pub modality: RenderModality,
    pub status: JobState,
    /// Dry-run jobs never touch chain/escrow.
    pub dry_run: bool,
    pub buyer: Option<String>,
    pub title: String,
    pub budget: Budget,
    #[serde(default)]
    pub milestones: Vec<Milestone>,
}

impl RenderJob {
    /// True when every milestone is accepted (job is ready for final acceptance).
    pub fn all_milestones_accepted(&self) -> bool {
        !self.milestones.is_empty()
            && self
                .milestones
                .iter()
                .all(|m| m.status == MilestoneState::Accepted)
    }

    /// Attempt a top-level state transition, enforcing the state machine.
    pub fn transition(&mut self, next: JobState) -> Result<(), String> {
        if self.status.can_transition_to(next) {
            self.status = next;
            Ok(())
        } else {
            Err(format!(
                "illegal job transition {:?} -> {:?}",
                self.status, next
            ))
        }
    }

    /// Attempt a milestone transition, enforcing the milestone state machine and
    /// dependency gating (a milestone may only go active once its dependencies are
    /// accepted).
    pub fn transition_milestone(
        &mut self,
        milestone_id: &str,
        next: MilestoneState,
    ) -> Result<(), String> {
        // Dependency gating is checked before we take a mutable borrow.
        if next == MilestoneState::Active {
            let deps: Vec<String> = self
                .milestones
                .iter()
                .find(|m| m.milestone_id == milestone_id)
                .map(|m| m.depends_on.clone())
                .ok_or_else(|| format!("no such milestone: {milestone_id}"))?;
            for dep in deps {
                let ok = self
                    .milestones
                    .iter()
                    .any(|m| m.milestone_id == dep && m.status == MilestoneState::Accepted);
                if !ok {
                    return Err(format!(
                        "milestone {milestone_id} blocked by unaccepted dependency {dep}"
                    ));
                }
            }
        }

        let m = self
            .milestones
            .iter_mut()
            .find(|m| m.milestone_id == milestone_id)
            .ok_or_else(|| format!("no such milestone: {milestone_id}"))?;
        if !m.status.can_transition_to(next) {
            return Err(format!(
                "illegal milestone transition {:?} -> {:?} for {milestone_id}",
                m.status, next
            ));
        }
        if next == MilestoneState::RevisionRequested {
            m.revisions += 1;
        }
        m.status = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn milestone(id: &str, deps: &[&str]) -> Milestone {
        Milestone {
            milestone_id: id.to_string(),
            name: id.to_string(),
            deliverable: String::new(),
            deliverable_kinds: vec![],
            required_capabilities: vec![],
            depends_on: deps.iter().map(|s| s.to_string()).collect(),
            status: MilestoneState::Pending,
            revisions: 0,
        }
    }

    fn job() -> RenderJob {
        RenderJob {
            schema_version: SCHEMA_VERSION,
            job_id: "media-job_test".to_string(),
            modality: RenderModality::Video,
            status: JobState::Drafted,
            dry_run: true,
            buyer: None,
            title: "test".to_string(),
            budget: Budget {
                max_hunits: Some(1_000_000),
                committed_hunits: 0,
                released_hunits: 0,
            },
            milestones: vec![milestone("m1", &[]), milestone("m2", &["m1"])],
        }
    }

    #[test]
    fn legal_job_transitions() {
        let mut j = job();
        assert!(j.transition(JobState::Quoted).is_ok());
        assert!(j.transition(JobState::Funded).is_ok());
        assert!(j.transition(JobState::InProduction).is_ok());
    }

    #[test]
    fn illegal_job_transition_rejected() {
        let mut j = job();
        assert!(j.transition(JobState::Settled).is_err());
    }

    #[test]
    fn milestone_dependency_gating() {
        let mut j = job();
        // m2 depends on m1; cannot activate until m1 accepted.
        assert!(j.transition_milestone("m2", MilestoneState::Active).is_err());
        j.transition_milestone("m1", MilestoneState::Active).unwrap();
        j.transition_milestone("m1", MilestoneState::Delivered).unwrap();
        j.transition_milestone("m1", MilestoneState::Accepted).unwrap();
        assert!(j.transition_milestone("m2", MilestoneState::Active).is_ok());
    }

    #[test]
    fn revision_counter_increments() {
        let mut j = job();
        j.transition_milestone("m1", MilestoneState::Active).unwrap();
        j.transition_milestone("m1", MilestoneState::Delivered).unwrap();
        j.transition_milestone("m1", MilestoneState::RevisionRequested)
            .unwrap();
        let m = j.milestones.iter().find(|m| m.milestone_id == "m1").unwrap();
        assert_eq!(m.revisions, 1);
    }

    #[test]
    fn all_accepted_gate() {
        let mut j = job();
        assert!(!j.all_milestones_accepted());
        for id in ["m1", "m2"] {
            // walk each milestone to accepted (respecting the m1->m2 dependency)
            let _ = j.transition_milestone(id, MilestoneState::Active);
            let _ = j.transition_milestone(id, MilestoneState::Delivered);
            let _ = j.transition_milestone(id, MilestoneState::Accepted);
        }
        assert!(j.all_milestones_accepted());
    }
}
