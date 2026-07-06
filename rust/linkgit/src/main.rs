mod remote;
mod api;
mod objects;
mod mirror;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    // Check for subcommands
    if args.get(1).map(|s| s.as_str()) == Some("mirror") {
        return mirror_cmd(&args[2..]).await;
    }

    // args[1] = remote name, args[2] = url (linkgit://owner/repo)
    let url = args.get(2).cloned().unwrap_or_default();
    let (owner, repo) = parse_url(&url)?;

    let api_base = std::env::var("HONE_API")
        .unwrap_or_else(|_| "http://localhost:4242".to_string());
    let account = std::env::var("HONE_ACCOUNT").unwrap_or_default();
    let posting_key = std::env::var("HONE_POSTING_KEY").unwrap_or_default();

    let client = api::Client::new(api_base, account, posting_key);
    remote::run(client, owner, repo).await
}

async fn mirror_cmd(args: &[String]) -> anyhow::Result<()> {
    use crate::mirror::*;

    let sub = args.get(0).map(|s| s.as_str()).unwrap_or("list");

    match sub {
        "add" => {
            // linkgit mirror add <name> <url> [--no-push] [--fetch]
            let name = args.get(1).cloned().unwrap_or_default();
            let url = args.get(2).cloned().unwrap_or_default();
            if name.is_empty() || url.is_empty() {
                anyhow::bail!("usage: linkgit mirror add <name> <url>");
            }
            let push = !args.contains(&"--no-push".to_string());
            let fetch = args.contains(&"--fetch".to_string());

            let mut mirrors = read_mirrors()?;
            mirrors.retain(|m| m.name != name); // replace if exists
            mirrors.push(Mirror { name: name.clone(), url: url.clone(), push, fetch });
            write_mirrors(&mirrors)?;
            println!("Mirror '{}' added: {}", name, url);
            println!("Run: git add .linkgit/mirrors && git commit -m 'add {} mirror'", name);
        }
        "remove" | "rm" => {
            let name = args.get(1).cloned().unwrap_or_default();
            let mut mirrors = read_mirrors()?;
            mirrors.retain(|m| m.name != name);
            write_mirrors(&mirrors)?;
            println!("Mirror '{}' removed", name);
        }
        "list" | "ls" => {
            let mirrors = read_mirrors()?;
            if mirrors.is_empty() {
                println!("No mirrors configured. Use: linkgit mirror add <name> <url>");
            } else {
                for m in &mirrors {
                    println!("{}: {} (push={}, fetch={})", m.name, m.url, m.push, m.fetch);
                }
            }
        }
        "apply" => {
            // linkgit mirror apply [remote-name] -- wire up git push URLs
            let remote = args.get(1).map(|s| s.as_str()).unwrap_or("origin");
            // Get current linkgit URL from git remote
            let out = std::process::Command::new("git")
                .args(["remote", "get-url", remote])
                .output()?;
            let current_url = String::from_utf8_lossy(&out.stdout).trim().to_string();
            apply_mirrors_to_remote(remote, &current_url)?;
        }
        "sync" => {
            // Push current branch to all mirrors manually
            let mirrors = read_mirrors()?;
            let branch_out = std::process::Command::new("git")
                .args(["rev-parse", "--abbrev-ref", "HEAD"])
                .output()?;
            let branch = String::from_utf8_lossy(&branch_out.stdout).trim().to_string();
            for m in mirrors.iter().filter(|m| m.push) {
                println!("Syncing to {}...", m.name);
                let status = std::process::Command::new("git")
                    .args(["push", &m.url, &branch])
                    .status()?;
                if status.success() {
                    println!("  ✓ {}", m.name);
                } else {
                    println!("  ✗ {} (failed)", m.name);
                }
            }
        }
        _ => {
            println!("Usage: linkgit mirror <add|remove|list|apply|sync> [args]");
        }
    }
    Ok(())
}

fn parse_url(url: &str) -> Result<(String, String)> {
    // linkgit://owner/repo
    let path = url.strip_prefix("linkgit://").unwrap_or(url);
    let mut parts = path.splitn(2, '/');
    let owner = parts.next().unwrap_or("").to_string();
    let repo = parts.next().unwrap_or("").to_string();
    Ok((owner, repo))
}
