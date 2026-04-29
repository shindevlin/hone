use anyhow::Result;
use std::collections::HashMap;

pub struct Client {
    base: String,
    account: String,
    posting_key: String,
    http: reqwest::Client,
}

impl Client {
    pub fn new(base: String, account: String, posting_key: String) -> Self {
        Self {
            base,
            account,
            posting_key,
            http: reqwest::Client::new(),
        }
    }

    pub async fn get_refs(&self, repo_id: &str) -> Result<HashMap<String, String>> {
        let parts: Vec<&str> = repo_id.splitn(2, '/').collect();
        let owner = parts[0];
        let repo = parts.get(1).copied().unwrap_or("");
        let url = format!("{}/api/linkgit/repo/{}/{}", self.base, owner, repo);
        let resp: serde_json::Value = self.http.get(&url).send().await?.json().await?;
        let refs = resp["refs"]
            .as_object()
            .map(|m| {
                m.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                    .collect()
            })
            .unwrap_or_default();
        Ok(refs)
    }

    pub async fn update_ref(
        &self,
        repo_id: &str,
        ref_name: &str,
        commit_hash: &str,
        prev_hash: Option<&str>,
    ) -> Result<()> {
        let url = format!("{}/api/linkgit/repo/ref/update", self.base);
        let body = serde_json::json!({
            "repo_id": repo_id,
            "ref_name": ref_name,
            "commit_hash": commit_hash,
            "prev_hash": prev_hash,
            "signed_by": self.account,
            "signature": self.sign_update(repo_id, ref_name, commit_hash),
        });
        self.http.post(&url).json(&body).send().await?;
        Ok(())
    }

    fn sign_update(&self, _repo_id: &str, _ref_name: &str, _commit_hash: &str) -> String {
        // TODO: sign with posting_key using secp256k1
        // Full signing implementation in Phase 2
        let _ = &self.posting_key;
        String::new()
    }

    pub async fn create_repo(
        &self,
        repo_id: &str,
        name: &str,
        visibility: &str,
        hide_key: Option<&str>,
    ) -> Result<()> {
        let url = format!("{}/api/linkgit/repo/create", self.base);
        let body = serde_json::json!({
            "repo_id": repo_id,
            "name": name,
            "visibility": visibility,
            "hide_key": hide_key,
            "signed_by": self.account,
            "signature": "",
        });
        self.http.post(&url).json(&body).send().await?;
        Ok(())
    }
}
