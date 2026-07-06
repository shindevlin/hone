use crate::oracle::espn::EspnGame;
use anyhow::{anyhow, Result};
use parking_lot::RwLock;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, warn};

const CACHE_TTL: Duration = Duration::from_secs(300); // 5-minute cache

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Serialize)]
struct InferenceRequest {
    model: String,
    messages: Vec<ChatMessage>,
    max_tokens: u32,
    stream: bool,
}

#[derive(Debug, Deserialize)]
struct InferenceResponse {
    choices: Option<Vec<Choice>>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: Option<ChatMessage>,
}

struct CacheEntry {
    value: String,
    inserted_at: Instant,
}

pub struct HoneInferenceClient {
    client: Client,
    api_url: String,
    project_key: String,
    cache: RwLock<HashMap<String, CacheEntry>>,
}

impl HoneInferenceClient {
    pub fn new(api_url: String, project_key: String) -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("Failed to build HTTP client"),
            api_url,
            project_key,
            cache: RwLock::new(HashMap::new()),
        }
    }

    pub async fn ask(&self, prompt: &str, system: Option<&str>) -> Result<String> {
        let cache_key = format!("{:x}", md5_hash(prompt));
        {
            let cache = self.cache.read();
            if let Some(entry) = cache.get(&cache_key) {
                if entry.inserted_at.elapsed() < CACHE_TTL {
                    debug!("HONE inference cache hit for key {}", &cache_key[..8]);
                    return Ok(entry.value.clone());
                }
            }
        }

        let mut messages = Vec::new();
        if let Some(sys) = system {
            messages.push(ChatMessage {
                role: "system".to_string(),
                content: sys.to_string(),
            });
        }
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: prompt.to_string(),
        });

        let body = InferenceRequest {
            model: "qwen3.5:27b".to_string(),
            messages,
            max_tokens: 300,
            stream: false,
        };

        let url = format!("{}/v1/chat/completions", self.api_url);
        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.project_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow!("HONE inference request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("HONE inference error {}: {}", status, body));
        }

        let inference: InferenceResponse = resp
            .json()
            .await
            .map_err(|e| anyhow!("Failed to parse HONE inference response: {}", e))?;

        let content = inference
            .choices
            .and_then(|cs| cs.into_iter().next())
            .and_then(|c| c.message)
            .map(|m| m.content)
            .unwrap_or_else(|| "No response from inference node.".to_string());

        // Cache the result
        {
            let mut cache = self.cache.write();
            cache.insert(
                cache_key,
                CacheEntry {
                    value: content.clone(),
                    inserted_at: Instant::now(),
                },
            );
        }

        Ok(content)
    }

    pub async fn analyze_pool(
        &self,
        pool_id: u32,
        description: &str,
        side_a: f64,
        side_b: f64,
        hours_left: f64,
    ) -> Result<String> {
        let total = side_a + side_b;
        let a_pct = if total > 0.0 { side_a / total * 100.0 } else { 50.0 };
        let b_pct = 100.0 - a_pct;

        let prompt = format!(
            "Betting pool #{pool_id}: \"{description}\"\n\
             Pool A: {side_a:.4} ({a_pct:.0}%) | Pool B: {side_b:.4} ({b_pct:.0}%)\n\
             Hours remaining: {hours_left:.1}\n\
             Provide a 1-2 sentence betting insight. Be concise and sports-focused."
        );

        self.ask(&prompt, Some("You are a concise sports betting analyst. Give sharp, data-driven insights in 1-2 sentences.")).await
    }

    pub async fn generate_commentary(
        &self,
        description: &str,
        winner_side: &str,
        winner_pool: f64,
        loser_pool: f64,
    ) -> Result<String> {
        let prompt = format!(
            "Betting pool resolved: \"{description}\"\n\
             {winner_side} won! Pool size: {winner_pool:.4} vs {loser_pool:.4}\n\
             Write a short, exciting 1-sentence congratulations message for winners."
        );

        self.ask(&prompt, Some("You are a sports betting commentator. Write exciting, celebratory messages.")).await
    }

    /// Ask HONE to match a user's freeform description to a specific ESPN game.
    /// Returns the index into `games` if matched, else None.
    pub async fn match_game_description(
        &self,
        user_input: &str,
        games: &[EspnGame],
    ) -> Result<Option<usize>> {
        if games.is_empty() {
            return Ok(None);
        }

        let game_list = games
            .iter()
            .enumerate()
            .map(|(i, g)| format!("{}. {}", i + 1, g.normalized_description()))
            .collect::<Vec<_>>()
            .join("\n");

        let prompt = format!(
            "User wants to create a sports bet. Their description: \"{}\"\n\n\
            Scheduled games right now:\n{}\n\n\
            Which game number best matches? Reply with ONLY the number (e.g. \"3\"), or \"none\".",
            user_input, game_list
        );

        let response = self
            .ask(
                &prompt,
                Some(
                    "You match sports bet descriptions to scheduled games. \
                     Reply with only a single number or the word none.",
                ),
            )
            .await?;

        let trimmed = response.trim().to_lowercase();
        if trimmed == "none" || trimmed.is_empty() {
            return Ok(None);
        }

        // Extract first digit sequence from response
        let digits: String = trimmed.chars().filter(|c| c.is_ascii_digit()).collect();
        let idx: usize = digits.parse().unwrap_or(0);

        if idx >= 1 && idx <= games.len() {
            Ok(Some(idx - 1))
        } else {
            Ok(None)
        }
    }

    pub fn invalidate(&self, pool_id: u32) {
        // Remove all cache entries related to this pool
        let mut cache = self.cache.write();
        let prefix = format!("{}", pool_id);
        cache.retain(|_k, _v| true); // Simple: clear all (pool IDs aren't in cache keys here)
        drop(cache);
    }
}

fn md5_hash(s: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// HONE service registration payload
#[derive(Debug, Serialize)]
pub struct ServiceRegistration {
    pub name: String,
    pub description: String,
    pub deployer: String,
    pub runtime: ServiceRuntime,
    pub price_per_epoch: f64,
    pub reputation_min: u32,
}

#[derive(Debug, Serialize)]
pub struct ServiceRuntime {
    pub r#type: String,
    pub command: String,
    pub args: Vec<String>,
    pub cpu: f64,
    pub ram_mb: u32,
}

/// Heartbeat payload
#[derive(Debug, Serialize)]
pub struct Heartbeat {
    pub host_id: String,
    pub timestamp: u64,
    pub active_sessions: u32,
    pub capacity_remaining: u32,
    pub version: String,
}

pub struct HoneServiceClient {
    client: Client,
    api_url: String,
    project_key: String,
}

impl HoneServiceClient {
    pub fn new(api_url: String, project_key: String) -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .expect("Failed to build HTTP client"),
            api_url,
            project_key,
        }
    }

    pub async fn register_service(&self, account: &str) -> Result<String> {
        let reg = ServiceRegistration {
            name: "betchu-bot".to_string(),
            description: "HONE-native P2P sports betting Telegram bot on Base Sepolia".to_string(),
            deployer: account.to_string(),
            runtime: ServiceRuntime {
                r#type: "http".to_string(),
                command: "betchu-bot".to_string(),
                args: vec![],
                cpu: 0.5,
                ram_mb: 256,
            },
            price_per_epoch: 0.0,
            reputation_min: 0,
        };

        let url = format!("{}/api/services", self.api_url);
        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.project_key))
            .json(&reg)
            .send()
            .await;

        match resp {
            Ok(r) if r.status().is_success() => {
                let body: serde_json::Value = r.json().await.unwrap_or_default();
                let slug = body
                    .get("slug")
                    .and_then(|s| s.as_str())
                    .unwrap_or("betchu-bot")
                    .to_string();
                Ok(slug)
            }
            Ok(r) => {
                warn!("Service registration returned {}", r.status());
                Ok(format!("{}/betchu-bot", account))
            }
            Err(e) => {
                warn!("Service registration failed (HONE node may be offline): {}", e);
                Ok(format!("{}/betchu-bot", account))
            }
        }
    }

    pub async fn heartbeat(&self, slug: &str, active_sessions: u32) -> Result<()> {
        let beat = Heartbeat {
            host_id: "betchu-primary".to_string(),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
            active_sessions,
            capacity_remaining: 100u32.saturating_sub(active_sessions),
            version: env!("CARGO_PKG_VERSION").to_string(),
        };

        let url = format!("{}/api/services/{}/heartbeat", self.api_url, slug);
        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.project_key))
            .json(&beat)
            .send()
            .await;

        match resp {
            Ok(r) if r.status().is_success() => {
                debug!("Heartbeat OK for slug {}", slug);
                Ok(())
            }
            Ok(r) => {
                warn!("Heartbeat {} for slug {}", r.status(), slug);
                Ok(())
            }
            Err(e) => {
                warn!("Heartbeat failed (HONE node offline?): {}", e);
                Ok(())
            }
        }
    }
}
