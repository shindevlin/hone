use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;
use url::Url;

pub const CATALOG_SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_PUBLIC_APIS_REPO: &str = "https://github.com/public-apis/public-apis";
pub const DEFAULT_PUBLIC_APIS_README: &str = "README.md";

#[derive(Debug, Error)]
pub enum CatalogError {
    #[error("failed to read {path}: {source}")]
    ReadFile {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to write {path}: {source}")]
    WriteFile {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("malformed markdown row at line {line}: {row}")]
    MalformedRow { line: usize, row: String },
    #[error("invalid URL at line {line}: {url}")]
    InvalidUrl { line: usize, url: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AuthType {
    None,
    ApiKey,
    OAuth,
    UserAgent,
    XMashapeKey,
    Other(String),
}

impl AuthType {
    pub fn from_markdown_cell(cell: &str) -> Self {
        let clean = strip_backticks(cell.trim()).trim().to_string();
        match clean.as_str() {
            "" | "No" => Self::None,
            "apiKey" => Self::ApiKey,
            "OAuth" => Self::OAuth,
            "User-Agent" => Self::UserAgent,
            "X-Mashape-Key" => Self::XMashapeKey,
            other => Self::Other(other.to_string()),
        }
    }

    pub fn requires_secret(&self) -> bool {
        !matches!(self, Self::None)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CorsStatus {
    Yes,
    No,
    Unknown,
}

impl CorsStatus {
    pub fn from_markdown_cell(cell: &str) -> Self {
        match cell.trim() {
            "Yes" => Self::Yes,
            "No" => Self::No,
            _ => Self::Unknown,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum LinkStatus {
    NotChecked,
    InvalidUrl,
    UnsupportedScheme,
    Alive,
    Redirected,
    ClientError,
    ServerError,
    Timeout,
    NetworkError,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LinkVerification {
    pub status: LinkStatus,
    pub checked_at_unix_secs: Option<u64>,
    pub elapsed_ms: Option<u128>,
    pub http_status: Option<u16>,
    pub final_url: Option<String>,
    pub error: Option<String>,
}

impl Default for LinkVerification {
    fn default() -> Self {
        Self {
            status: LinkStatus::NotChecked,
            checked_at_unix_secs: None,
            elapsed_ms: None,
            http_status: None,
            final_url: None,
            error: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CatalogSource {
    pub repo_url: String,
    pub commit: String,
    pub path: String,
    pub fetched_at_unix_secs: u64,
}

impl CatalogSource {
    pub fn new(
        repo_url: impl Into<String>,
        commit: impl Into<String>,
        path: impl Into<String>,
    ) -> Self {
        Self {
            repo_url: repo_url.into(),
            commit: commit.into(),
            path: path.into(),
            fetched_at_unix_secs: now_unix_secs(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PublicApiRecord {
    pub name: String,
    pub category: String,
    pub description: String,
    pub url: String,
    pub auth: AuthType,
    pub https: bool,
    pub cors: CorsStatus,
    pub source_line: usize,
    pub risk_flags: Vec<String>,
    pub verification: LinkVerification,
}

impl PublicApiRecord {
    pub fn is_secretless(&self) -> bool {
        !self.auth.requires_secret()
    }

    pub fn is_llm_tool_candidate(&self) -> bool {
        self.https
            && matches!(
                self.verification.status,
                LinkStatus::NotChecked | LinkStatus::Alive | LinkStatus::Redirected
            )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CatalogSnapshot {
    pub schema_version: u32,
    pub source: CatalogSource,
    pub generated_at_unix_secs: u64,
    pub records: Vec<PublicApiRecord>,
}

impl CatalogSnapshot {
    pub fn search(&self, query: CatalogQuery<'_>) -> Vec<&PublicApiRecord> {
        let normalized_query = query.text.map(normalize_search_text);
        let categories: BTreeSet<String> = query
            .categories
            .iter()
            .map(|category| normalize_search_text(category))
            .collect();

        self.records
            .iter()
            .filter(|record| {
                if !categories.is_empty()
                    && !categories.contains(&normalize_search_text(&record.category))
                {
                    return false;
                }
                if query.https_only && !record.https {
                    return false;
                }
                if let Some(secretless) = query.secretless_only {
                    if secretless && !record.is_secretless() {
                        return false;
                    }
                    if !secretless && record.is_secretless() {
                        return false;
                    }
                }
                if let Some(ref text) = normalized_query {
                    let haystack = normalize_search_text(&format!(
                        "{} {} {} {}",
                        record.name, record.category, record.description, record.url
                    ));
                    if !haystack.contains(text) {
                        return false;
                    }
                }
                true
            })
            .take(query.limit.unwrap_or(usize::MAX))
            .collect()
    }

    pub fn content_hash(&self) -> Result<String, CatalogError> {
        #[derive(Serialize)]
        struct FingerprintRecord<'a> {
            name: &'a str,
            category: &'a str,
            description: &'a str,
            url: &'a str,
            auth: &'a AuthType,
            https: bool,
            cors: &'a CorsStatus,
        }

        #[derive(Serialize)]
        struct Fingerprint<'a> {
            schema_version: u32,
            repo_url: &'a str,
            commit: &'a str,
            path: &'a str,
            records: Vec<FingerprintRecord<'a>>,
        }

        let payload = Fingerprint {
            schema_version: self.schema_version,
            repo_url: &self.source.repo_url,
            commit: &self.source.commit,
            path: &self.source.path,
            records: self
                .records
                .iter()
                .map(|record| FingerprintRecord {
                    name: &record.name,
                    category: &record.category,
                    description: &record.description,
                    url: &record.url,
                    auth: &record.auth,
                    https: record.https,
                    cors: &record.cors,
                })
                .collect(),
        };

        let bytes = serde_json::to_vec(&payload)?;
        Ok(hex::encode(Sha256::digest(bytes)))
    }

    pub fn save_json(&self, path: impl AsRef<Path>) -> Result<(), CatalogError> {
        let path = path.as_ref();
        let bytes = serde_json::to_vec_pretty(self)?;
        std::fs::write(path, bytes).map_err(|source| CatalogError::WriteFile {
            path: path.to_path_buf(),
            source,
        })
    }

    pub fn load_json(path: impl AsRef<Path>) -> Result<Self, CatalogError> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|source| CatalogError::ReadFile {
            path: path.to_path_buf(),
            source,
        })?;
        Ok(serde_json::from_slice(&bytes)?)
    }
}

#[derive(Clone, Debug, Default)]
pub struct CatalogQuery<'a> {
    pub text: Option<&'a str>,
    pub categories: Vec<&'a str>,
    pub secretless_only: Option<bool>,
    pub https_only: bool,
    pub limit: Option<usize>,
}

pub fn parse_public_apis_readme(
    readme: &str,
    source: CatalogSource,
) -> Result<CatalogSnapshot, CatalogError> {
    let mut current_category: Option<String> = None;
    let mut records = Vec::new();

    for (index, raw_line) in readme.lines().enumerate() {
        let line_number = index + 1;
        let line = raw_line.trim();

        if let Some(category) = line.strip_prefix("### ") {
            current_category = Some(category.trim().to_string());
            continue;
        }

        if !line.starts_with("| [") {
            continue;
        }

        let Some(category) = current_category.clone() else {
            continue;
        };

        let cells = split_markdown_table_row(line);
        if cells.len() < 5 {
            return Err(CatalogError::MalformedRow {
                line: line_number,
                row: line.to_string(),
            });
        }

        let (name, url) =
            parse_markdown_link(cells[0].trim()).ok_or_else(|| CatalogError::MalformedRow {
                line: line_number,
                row: line.to_string(),
            })?;

        Url::parse(&url).map_err(|_| CatalogError::InvalidUrl {
            line: line_number,
            url: url.clone(),
        })?;

        let auth = AuthType::from_markdown_cell(&cells[2]);
        let https = cells[3].trim() == "Yes";
        let cors = CorsStatus::from_markdown_cell(&cells[4]);
        let risk_flags = risk_flags_for(&url, &auth, https, &cors);

        records.push(PublicApiRecord {
            name,
            category,
            description: cells[1].trim().to_string(),
            url,
            auth,
            https,
            cors,
            source_line: line_number,
            risk_flags,
            verification: LinkVerification::default(),
        });
    }

    Ok(CatalogSnapshot {
        schema_version: CATALOG_SCHEMA_VERSION,
        source,
        generated_at_unix_secs: now_unix_secs(),
        records,
    })
}

pub fn load_public_apis_from_checkout(
    checkout: impl AsRef<Path>,
) -> Result<CatalogSnapshot, CatalogError> {
    let checkout = checkout.as_ref();
    let readme_path = checkout.join(DEFAULT_PUBLIC_APIS_README);
    let readme =
        std::fs::read_to_string(&readme_path).map_err(|source| CatalogError::ReadFile {
            path: readme_path.clone(),
            source,
        })?;
    let commit = git_head_commit(checkout).unwrap_or_else(|| "unknown".to_string());
    let source = CatalogSource::new(DEFAULT_PUBLIC_APIS_REPO, commit, DEFAULT_PUBLIC_APIS_README);
    parse_public_apis_readme(&readme, source)
}

fn split_markdown_table_row(line: &str) -> Vec<String> {
    line.trim_matches('|')
        .split('|')
        .map(|cell| cell.trim().to_string())
        .collect()
}

fn parse_markdown_link(cell: &str) -> Option<(String, String)> {
    let rest = cell.strip_prefix('[')?;
    let (name, url_part) = rest.split_once("](")?;
    let url = url_part.strip_suffix(')')?.trim().to_string();
    Some((name.trim().to_string(), url))
}

fn strip_backticks(value: &str) -> String {
    value.trim_matches('`').to_string()
}

fn risk_flags_for(url: &str, auth: &AuthType, https: bool, cors: &CorsStatus) -> Vec<String> {
    let mut flags = Vec::new();
    if !https || url.starts_with("http://") {
        flags.push("http_only_or_declared_no_https".to_string());
    }
    if auth.requires_secret() {
        flags.push("requires_secret_or_oauth".to_string());
    }
    if matches!(cors, CorsStatus::Unknown) {
        flags.push("cors_unknown".to_string());
    }
    if url.contains("utm_") {
        flags.push("tracking_params_present".to_string());
    }
    flags
}

fn git_head_commit(checkout: &Path) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(checkout)
        .arg("rev-parse")
        .arg("HEAD")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let commit = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if commit.is_empty() {
        None
    } else {
        Some(commit)
    }
}

fn normalize_search_text(input: &str) -> String {
    input.to_ascii_lowercase()
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

#[cfg(feature = "verify")]
pub fn default_http_client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .user_agent("btcpc-api-catalog/0.1 (+https://btcpc.net)")
        .timeout(std::time::Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
}

#[cfg(feature = "verify")]
pub async fn verify_link(client: &reqwest::Client, record: &PublicApiRecord) -> LinkVerification {
    let started = std::time::Instant::now();
    let checked_at = now_unix_secs();

    let parsed = match Url::parse(&record.url) {
        Ok(url) => url,
        Err(error) => {
            return LinkVerification {
                status: LinkStatus::InvalidUrl,
                checked_at_unix_secs: Some(checked_at),
                elapsed_ms: Some(started.elapsed().as_millis()),
                http_status: None,
                final_url: None,
                error: Some(error.to_string()),
            };
        }
    };

    if parsed.scheme() != "https" && parsed.scheme() != "http" {
        return LinkVerification {
            status: LinkStatus::UnsupportedScheme,
            checked_at_unix_secs: Some(checked_at),
            elapsed_ms: Some(started.elapsed().as_millis()),
            http_status: None,
            final_url: Some(parsed.to_string()),
            error: Some(format!("unsupported URL scheme: {}", parsed.scheme())),
        };
    }

    let mut response = client.head(parsed.clone()).send().await;
    if response
        .as_ref()
        .map(|resp| !resp.status().is_success())
        .unwrap_or(true)
    {
        response = client.get(parsed.clone()).send().await;
    }

    match response {
        Ok(resp) => {
            let status_code = resp.status();
            let final_url = resp.url().to_string();
            let status = if status_code.is_success() {
                if final_url != record.url {
                    LinkStatus::Redirected
                } else {
                    LinkStatus::Alive
                }
            } else if status_code.is_client_error() {
                LinkStatus::ClientError
            } else if status_code.is_server_error() {
                LinkStatus::ServerError
            } else {
                LinkStatus::NetworkError
            };
            LinkVerification {
                status,
                checked_at_unix_secs: Some(checked_at),
                elapsed_ms: Some(started.elapsed().as_millis()),
                http_status: Some(status_code.as_u16()),
                final_url: Some(final_url),
                error: None,
            }
        }
        Err(error) => LinkVerification {
            status: if error.is_timeout() {
                LinkStatus::Timeout
            } else {
                LinkStatus::NetworkError
            },
            checked_at_unix_secs: Some(checked_at),
            elapsed_ms: Some(started.elapsed().as_millis()),
            http_status: error.status().map(|status| status.as_u16()),
            final_url: Some(parsed.to_string()),
            error: Some(error.to_string()),
        },
    }
}

#[cfg(feature = "verify")]
pub async fn verify_snapshot_sequential(
    snapshot: &mut CatalogSnapshot,
    client: &reqwest::Client,
    limit: Option<usize>,
) {
    let limit = limit.unwrap_or(snapshot.records.len());
    for record in snapshot.records.iter_mut().take(limit) {
        record.verification = verify_link(client, record).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
## Index
* [Blockchain](#blockchain)

### Blockchain
API | Description | Auth | HTTPS | CORS
|:---|:---|:---|:---|:---|
| [Chainlink](https://chain.link/developer-resources) | Build hybrid smart contracts with Chainlink | No | Yes | Unknown |
| [Etherscan](https://etherscan.io/apis) | Ethereum explorer API | `apiKey` | Yes | Yes |

### Weather
API | Description | Auth | HTTPS | CORS
|:---|:---|:---|:---|:---|
| [Open-Meteo](https://open-meteo.com/) | Global weather forecast API for non-commercial use | No | Yes | Yes |
"#;

    #[test]
    fn parses_public_apis_markdown() {
        let source = CatalogSource::new(
            DEFAULT_PUBLIC_APIS_REPO,
            "abc123",
            DEFAULT_PUBLIC_APIS_README,
        );
        let snapshot = parse_public_apis_readme(SAMPLE, source).unwrap();
        assert_eq!(snapshot.records.len(), 3);
        assert_eq!(snapshot.records[0].category, "Blockchain");
        assert_eq!(snapshot.records[1].auth, AuthType::ApiKey);
        assert_eq!(snapshot.records[2].cors, CorsStatus::Yes);
    }

    #[test]
    fn filters_search_results() {
        let source = CatalogSource::new(
            DEFAULT_PUBLIC_APIS_REPO,
            "abc123",
            DEFAULT_PUBLIC_APIS_README,
        );
        let snapshot = parse_public_apis_readme(SAMPLE, source).unwrap();
        let results = snapshot.search(CatalogQuery {
            text: Some("weather"),
            categories: vec!["Weather"],
            secretless_only: Some(true),
            https_only: true,
            limit: Some(5),
        });
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "Open-Meteo");
    }

    #[test]
    fn content_hash_is_stable_for_same_source_and_records() {
        let source = CatalogSource::new(
            DEFAULT_PUBLIC_APIS_REPO,
            "abc123",
            DEFAULT_PUBLIC_APIS_README,
        );
        let first = parse_public_apis_readme(SAMPLE, source.clone()).unwrap();
        let second = parse_public_apis_readme(SAMPLE, source).unwrap();
        assert_eq!(
            first.content_hash().unwrap(),
            second.content_hash().unwrap()
        );
    }
}
