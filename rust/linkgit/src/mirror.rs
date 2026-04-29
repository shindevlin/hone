use anyhow::Result;

#[derive(Debug, Clone)]
pub struct Mirror {
    pub name: String,
    pub url: String,
    pub push: bool,
    pub fetch: bool,
}

/// Read .linkgit/mirrors from the repo root (finds repo root via git rev-parse).
pub fn read_mirrors() -> Result<Vec<Mirror>> {
    let root = repo_root()?;
    let path = root.join(".linkgit/mirrors");
    if !path.exists() {
        return Ok(vec![]);
    }
    let content = std::fs::read_to_string(&path)?;
    parse_mirrors(&content)
}

/// Write mirrors to .linkgit/mirrors.
pub fn write_mirrors(mirrors: &[Mirror]) -> Result<()> {
    let root = repo_root()?;
    let dir = root.join(".linkgit");
    std::fs::create_dir_all(&dir)?;
    let content = serialize_mirrors(mirrors);
    std::fs::write(dir.join("mirrors"), content)?;
    Ok(())
}

fn repo_root() -> Result<std::path::PathBuf> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()?;
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Ok(std::path::PathBuf::from(path))
}

/// Parse a simple TOML-like mirrors file.
/// Format:
/// [mirror.github]
/// url = "https://github.com/owner/repo"
/// push = true
/// fetch = false
fn parse_mirrors(content: &str) -> Result<Vec<Mirror>> {
    let mut mirrors: Vec<Mirror> = vec![];
    let mut current: Option<Mirror> = None;

    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("[mirror.") && line.ends_with(']') {
            if let Some(m) = current.take() {
                mirrors.push(m);
            }
            let name = line.trim_start_matches("[mirror.").trim_end_matches(']').to_string();
            current = Some(Mirror { name, url: String::new(), push: true, fetch: false });
        } else if let Some(ref mut m) = current {
            if let Some(val) = line.strip_prefix("url = ") {
                m.url = val.trim_matches('"').to_string();
            } else if line == "push = true" {
                m.push = true;
            } else if line == "push = false" {
                m.push = false;
            } else if line == "fetch = true" {
                m.fetch = true;
            } else if line == "fetch = false" {
                m.fetch = false;
            }
        }
    }
    if let Some(m) = current {
        mirrors.push(m);
    }
    Ok(mirrors)
}

fn serialize_mirrors(mirrors: &[Mirror]) -> String {
    let mut out = String::new();
    for m in mirrors {
        out.push_str(&format!("[mirror.{}]\n", m.name));
        out.push_str(&format!("url = \"{}\"\n", m.url));
        out.push_str(&format!("push = {}\n", m.push));
        out.push_str(&format!("fetch = {}\n", m.fetch));
        out.push('\n');
    }
    out
}

/// Wire up git remote push URLs based on .linkgit/mirrors.
/// Runs: git remote set-url --add <remote> <url> for each push=true mirror.
/// Also sets the linkgit URL as the primary remote URL.
pub fn apply_mirrors_to_remote(remote: &str, linkgit_url: &str) -> Result<()> {
    // First set the primary URL to linkgit
    std::process::Command::new("git")
        .args(["remote", "set-url", remote, linkgit_url])
        .status()?;

    let mirrors = read_mirrors()?;
    for m in mirrors.iter().filter(|m| m.push) {
        std::process::Command::new("git")
            .args(["remote", "set-url", "--add", remote, &m.url])
            .status()?;
    }
    println!("Mirrors applied to remote '{}':", remote);
    println!("  {} (primary, push+fetch)", linkgit_url);
    for m in mirrors.iter().filter(|m| m.push) {
        println!("  {} (push only)", m.url);
    }
    Ok(())
}
