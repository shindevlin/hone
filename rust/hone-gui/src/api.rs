use anyhow::Result;
use serde_json::Value;

pub fn get_json(base: &str, path: &str) -> Result<Value> {
    let url = format!("{}{}", base, path);
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()?;
    let resp = client.get(&url).send()?;
    let val: Value = resp.json()?;
    Ok(val)
}

pub fn post_json(base: &str, path: &str, body: &Value) -> Result<Value> {
    let url = format!("{}{}", base, path);
    let client = reqwest::blocking::Client::new();
    let resp = client.post(&url).json(body).send()?;
    let status = resp.status();
    let v: Value = resp.json()?;
    if !status.is_success() {
        let msg = v.get("error").or_else(|| v.get("message"))
            .and_then(|m| m.as_str()).unwrap_or("error");
        return Err(anyhow::anyhow!("HTTP {}: {}", status, msg));
    }
    Ok(v)
}
