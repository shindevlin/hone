//! Per-species parameters and the "naming allowed" gate.
//!
//! A species gets a naming offer only if it passed the evaluation in `eval/RESULTS.md`
//! (held-out, across-session precision >= 0.90 for night-level "is back" decisions).
//! Everything else is clustered (so history exists if the gate later opens) but never offered
//! for naming. Unknown species default to closed.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpeciesParams {
    /// Naming offered (gate passed in evaluation).
    pub naming_allowed: bool,
    /// Cosine (centred) to join an existing cluster with one call.
    pub join_tau: f32,
    /// Cosine of a night's aggregate to a bird's centroid to say "is back".
    pub back_tau: f32,
    /// Calls in a night needed before "is back" can be said.
    pub k_min: u32,
    /// Two clusters whose centroids are at least this close merge.
    pub merge_tau: f32,
    /// A cluster whose two halves are further apart than this splits.
    pub split_tau: f32,
    /// Logistic calibration p(same bird | cosine) = 1 / (1 + exp(-a (s - b))).
    pub logit_a: f32,
    pub logit_b: f32,
    /// Where the numbers came from.
    pub source: String,
}

impl SpeciesParams {
    /// Conservative defaults for species without an evaluation. Naming closed.
    pub fn untested() -> Self {
        SpeciesParams {
            naming_allowed: false,
            join_tau: 0.55,
            back_tau: 0.70,
            k_min: 5,
            merge_tau: 0.85,
            split_tau: 0.30,
            logit_a: 10.0,
            logit_b: 0.55,
            source: "untested default".into(),
        }
    }
    pub fn p_same(&self, s: f32) -> f32 {
        1.0 / (1.0 + (-(self.logit_a * (s - self.logit_b))).exp())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GateTable {
    /// Keyed by the binomial name as Perch reports it (e.g. "Pitangus sulphuratus").
    pub species: BTreeMap<String, SpeciesParams>,
}

impl GateTable {
    pub fn get(&self, species: &str) -> SpeciesParams {
        self.species.get(species).cloned().unwrap_or_else(SpeciesParams::untested)
    }
    pub fn naming_allowed(&self, species: &str) -> bool {
        self.species.get(species).map(|p| p.naming_allowed).unwrap_or(false)
    }

    /// The shipped table (from the evaluation). See eval/RESULTS.md.
    pub fn shipped() -> Self {
        serde_json::from_str(include_str!("../eval/gates.json")).expect("eval/gates.json is valid")
    }
}
