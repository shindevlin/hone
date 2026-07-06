use crate::api::Client;
use anyhow::Result;
use std::io::{self, BufRead, Write};

pub async fn run(client: Client, owner: String, repo: String) -> Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());

    let repo_id = format!("{}/{}", owner, repo);

    for line in stdin.lock().lines() {
        let line = line?;
        let line = line.trim().to_string();

        if line == "capabilities" {
            writeln!(out, "fetch")?;
            writeln!(out, "push")?;
            writeln!(out, "")?;
            out.flush()?;
        } else if line == "list" || line == "list for-push" {
            // Fetch refs from HONE chain
            let refs = client.get_refs(&repo_id).await.unwrap_or_default();
            for (refname, hash) in &refs {
                writeln!(out, "{} {}", hash, refname)?;
            }
            // HEAD symref
            if refs.contains_key("refs/heads/main") {
                writeln!(out, "@refs/heads/main HEAD")?;
            } else if refs.contains_key("refs/heads/master") {
                writeln!(out, "@refs/heads/master HEAD")?;
            }
            writeln!(out, "")?;
            out.flush()?;
        } else if line.starts_with("fetch ") {
            // git wants us to fetch objects
            // Parse: "fetch <sha1> <refname>"
            // Drain all batched fetch commands, then respond with blank line
            let parts: Vec<&str> = line.splitn(3, ' ').collect();
            if parts.len() >= 2 {
                let _sha = parts[1];
                // Phase 2: retrieve pack data from hone-fs by CID and write to .git/objects
            }
            writeln!(out, "")?;
            out.flush()?;
        } else if line.starts_with("push ") {
            // git wants us to push objects
            // Parse: "push <src>:<dst>"
            let spec = line.strip_prefix("push ").unwrap_or("");
            let mut parts = spec.splitn(2, ':');
            let src = parts.next().unwrap_or("").to_string();
            let dst = parts.next().unwrap_or("").to_string();

            // Resolve the local ref to a commit hash, then submit LinkGitRefUpdate to chain
            let src_hash = resolve_local_ref(&src);
            if let Ok(hash) = src_hash {
                let _ = client.update_ref(&repo_id, &dst, &hash, None).await;
                writeln!(out, "ok {}", dst)?;
            } else {
                writeln!(out, "error {} ref not found", dst)?;
            }
            writeln!(out, "")?;
            out.flush()?;
        } else if line.is_empty() {
            // End of command batch
            break;
        }
    }
    Ok(())
}

fn resolve_local_ref(refname: &str) -> Result<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", refname])
        .output()?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        anyhow::bail!("could not resolve {}", refname)
    }
}
