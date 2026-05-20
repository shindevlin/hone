use axum::{extract::State, http::StatusCode, Json};
use serde_json::{json, Value};
use tracing::warn;

use crate::{app::AppState, models::ImportUrlRequest};

/// POST /api/commerce/import/amazon
pub async fn import_amazon(
    State(_app): State<AppState>,
    Json(body): Json<ImportUrlRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let url = body.url.trim().to_string();
    if url.is_empty() {
        return Err((StatusCode::BAD_REQUEST, Json(json!({"error": "url required"}))));
    }
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return Err((StatusCode::BAD_REQUEST, Json(json!({"error": "url must be https"}))));
    }

    match try_camoufox(&url).await {
        Ok(products) if !products.is_empty() => {
            let count = products.len();
            return Ok(Json(json!({ "products": products, "count": count, "engine": "camoufox" })));
        }
        Ok(_) => warn!("[market] camoufox returned empty — trying http fallback"),
        Err(e) => warn!("[market] camoufox failed: {e}"),
    }

    match try_http_fetch(&url).await {
        Ok(products) => {
            let count = products.len();
            Ok(Json(json!({
                "products": products,
                "count": count,
                "engine": "http-fallback",
                "note": "Amazon may have blocked the request — review and adjust prices before publishing",
            })))
        }
        Err(e) => Err((StatusCode::BAD_GATEWAY, Json(json!({
            "error": "could not fetch product data",
            "detail": e.to_string(),
        })))),
    }
}

async fn try_camoufox(url: &str) -> anyhow::Result<Vec<Value>> {
    use std::time::Duration;
    use tokio::process::Command;

    let venv_python = dirs_next::home_dir()
        .ok_or_else(|| anyhow::anyhow!("no home dir"))?
        .join(".local/share/pipx/venvs/camoufox/bin/python3");

    if !venv_python.exists() {
        anyhow::bail!("camoufox venv not found at {:?}", venv_python);
    }

    let script = dirs_next::home_dir()
        .unwrap_or_default()
        .join("repos/btcpc/scripts/amazon-scrape.py");

    if !script.exists() {
        anyhow::bail!("amazon-scrape.py not found at {:?}", script);
    }

    let output = tokio::time::timeout(
        Duration::from_secs(45),
        Command::new(&venv_python).arg(&script).arg(url).output(),
    )
    .await??;

    if output.stdout.is_empty() {
        anyhow::bail!("camoufox produced no output");
    }

    let parsed: Value = serde_json::from_slice(&output.stdout)?;
    if let Some(err) = parsed.get("error") {
        anyhow::bail!("camoufox: {err}");
    }
    Ok(parsed["products"].as_array().cloned().unwrap_or_default())
}

async fn try_http_fetch(url: &str) -> anyhow::Result<Vec<Value>> {
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36")
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("HTTP {}", resp.status());
    }
    let html = resp.text().await?;
    Ok(parse_amazon_html(&html, url))
}

fn parse_amazon_html(html: &str, source_url: &str) -> Vec<Value> {
    if let Some(products) = parse_jsonld(html) {
        if !products.is_empty() {
            return products;
        }
    }

    let title = extract_meta(html, "og:title")
        .or_else(|| extract_product_title(html))
        .unwrap_or_else(|| "Unknown product".to_string());
    let image = extract_meta(html, "og:image");
    let price = extract_price(html);

    if title != "Unknown product" || price.is_some() {
        return vec![json!({
            "title": title.trim(),
            "price": price,
            "image_url": image,
            "source_url": source_url,
            "token": "BTCPC",
        })];
    }

    vec![]
}

fn parse_jsonld(html: &str) -> Option<Vec<Value>> {
    let marker = r#"type="application/ld+json""#;
    let mut results = vec![];
    let mut rest = html;
    while let Some(start) = rest.find(marker) {
        let after = &rest[start + marker.len()..];
        if let (Some(jstart), Some(jend)) = (after.find('>'), after.find("</script>")) {
            if jend > jstart {
                let json_str = &after[jstart + 1..jend];
                if let Ok(val) = serde_json::from_str::<Value>(json_str) {
                    if val.get("@type").and_then(|t| t.as_str()) == Some("Product") {
                        results.push(json!({
                            "title": val["name"].as_str().unwrap_or(""),
                            "price": val["offers"]["price"].as_f64(),
                            "image_url": val["image"].as_str()
                                .or_else(|| val["image"][0].as_str()),
                            "token": "BTCPC",
                        }));
                    }
                }
            }
        }
        rest = &rest[start + marker.len()..];
    }
    if results.is_empty() { None } else { Some(results) }
}

fn extract_meta(html: &str, property: &str) -> Option<String> {
    let needle = format!("property=\"{}\"", property);
    let pos = html.find(&needle)?;
    let after = &html[pos..];
    let content_pos = after.find("content=\"")?;
    let value_start = content_pos + 9;
    let value_end = after[value_start..].find('"')?;
    Some(after[value_start..value_start + value_end].to_string())
}

fn extract_product_title(html: &str) -> Option<String> {
    let marker = "id=\"productTitle\"";
    let pos = html.find(marker)?;
    let after = &html[pos + marker.len()..];
    let gt = after.find('>')?;
    let end = after.find("</span>")?;
    if end <= gt { return None; }
    let text = after[gt + 1..end].trim().to_string();
    if text.is_empty() { None } else { Some(text) }
}

fn extract_price(html: &str) -> Option<f64> {
    for marker in &[r#"class="a-price-whole""#, r#""priceAmount":"#, r#""price":"#] {
        if let Some(pos) = html.find(marker) {
            let snippet = &html[pos + marker.len()..pos + marker.len() + 30];
            let digits: String = snippet.chars()
                .skip_while(|c| !c.is_ascii_digit())
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if let Ok(n) = digits.parse::<f64>() {
                if n > 0.0 { return Some(n); }
            }
        }
    }
    None
}
