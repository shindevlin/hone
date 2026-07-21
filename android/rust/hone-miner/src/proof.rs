use sha2::{Sha256, Digest};

/// Deterministic work hash: SHA256(job_id | output | account)
pub fn compute_work_hash(job_id: &str, output: &str, account: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(job_id.as_bytes());
    hasher.update(b"|");
    hasher.update(output.as_bytes());
    hasher.update(b"|");
    hasher.update(account.as_bytes());
    hex::encode(hasher.finalize())
}
