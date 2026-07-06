use anyhow::{anyhow, Result};
use reqwest::blocking::Client;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::env;

pub struct ApiClient {
    pub base_url: String,
    client: Client,
}

impl ApiClient {
    pub fn new() -> Self {
        let base_url = env::var("HONE_API_URL").ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| crate::session::load().map(|s| s.node_url))
            .unwrap_or_else(|| "http://localhost:4242".to_string());
        Self {
            base_url,
            client: Client::new(),
        }
    }

    pub fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self.client.get(&url).send()?;
        let status = resp.status();
        let body: Value = resp.json()?;
        if !status.is_success() {
            let msg = body
                .get("error")
                .or_else(|| body.get("message"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            return Err(anyhow!("API error {}: {}", status, msg));
        }
        let typed: T = serde_json::from_value(body)?;
        Ok(typed)
    }

    pub fn post<T: DeserializeOwned>(&self, path: &str, body: &Value) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self.client.post(&url).json(body).send()?;
        let status = resp.status();
        let body: Value = resp.json()?;
        if !status.is_success() {
            let msg = body
                .get("error")
                .or_else(|| body.get("message"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            return Err(anyhow!("API error {}: {}", status, msg));
        }
        let typed: T = serde_json::from_value(body)?;
        Ok(typed)
    }
}
