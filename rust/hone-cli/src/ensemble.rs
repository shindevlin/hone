use anyhow::Result;
use hone_sdk::KeyPair;
use serde_json::{json, Value};
use std::path::Path;

use crate::api::ApiClient;
use crate::helpers::{next_nonce, node_chain_id, print_resp, resolve_key_file, sign_entry};

pub fn cmd_ensemble_post(
    requester: &str, input_hash: &str, max_fee: u64, n_workers: u64,
    model: Option<String>, key_file: Option<&Path>,
) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, requester)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "EnsembleJobPost", json!({
        "requester": requester, "input_hash": input_hash,
        "max_fee": max_fee, "n_workers": n_workers, "nonce": nonce,
    }));
    let mut body = json!({
        "requester": requester, "input_hash": input_hash,
        "max_fee": max_fee, "n_workers": n_workers,
        "nonce": nonce, "signed_by": requester, "signature": sig,
    });
    if let Some(m) = model { body["model"] = m.into(); }
    let resp: Value = api.post("/api/ensemble/post", &body)?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_ensemble_vote(
    job_id: &str, worker: &str, output_hash: &str, key_file: Option<&Path>,
) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, worker)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "EnsembleVote", json!({
        "job_id": job_id, "worker": worker, "output_hash": output_hash, "nonce": nonce,
    }));
    let resp: Value = api.post("/api/ensemble/vote", &json!({
        "job_id": job_id, "worker": worker, "output_hash": output_hash,
        "nonce": nonce, "signed_by": worker, "signature": sig,
    }))?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_ensemble_get(job_id: &str) -> Result<()> {
    let api = ApiClient::new();
    let resp: Value = api.get(&format!("/api/ensemble/job/{}", job_id))?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_ensemble_list() -> Result<()> {
    let api = ApiClient::new();
    let resp: Value = api.get("/api/ensemble/jobs")?;
    print_resp(&resp);
    Ok(())
}
