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
