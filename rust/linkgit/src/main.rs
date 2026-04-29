mod remote;
mod api;
mod objects;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    // args[1] = remote name, args[2] = url (linkgit://owner/repo)
    let url = args.get(2).cloned().unwrap_or_default();
    let (owner, repo) = parse_url(&url)?;

    let api_base = std::env::var("BTCPC_API")
        .unwrap_or_else(|_| "http://localhost:4242".to_string());
    let account = std::env::var("BTCPC_ACCOUNT").unwrap_or_default();
    let posting_key = std::env::var("BTCPC_POSTING_KEY").unwrap_or_default();

    let client = api::Client::new(api_base, account, posting_key);
    remote::run(client, owner, repo).await
}

fn parse_url(url: &str) -> Result<(String, String)> {
    // linkgit://owner/repo
    let path = url.strip_prefix("linkgit://").unwrap_or(url);
    let mut parts = path.splitn(2, '/');
    let owner = parts.next().unwrap_or("").to_string();
    let repo = parts.next().unwrap_or("").to_string();
    Ok((owner, repo))
}
