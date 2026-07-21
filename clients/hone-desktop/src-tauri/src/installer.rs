use std::path::PathBuf;
use std::process::Command;

pub fn install_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".hone")
}

pub fn has_command(cmd: &str) -> bool {
    which::which(cmd).is_ok()
}

pub fn has_hone() -> bool {
    install_dir().join(".git").exists()
}

pub async fn install() -> Result<String, Box<dyn std::error::Error>> {
    let dir = install_dir();
    if has_hone() {
        // Update
        let output = Command::new("git")
            .arg("pull")
            .arg("--ff-only")
            .current_dir(&dir)
            .output()?;
        if !output.status.success() {
            return Err(format!("git pull failed: {}", String::from_utf8_lossy(&output.stderr)).into());
        }
    } else {
        // Clone
        std::fs::create_dir_all(dir.parent().unwrap())?;
        let output = Command::new("git")
            .arg("clone")
            .arg("https://github.com/shindevlin/hone.git")
            .arg(&dir)
            .output()?;
        if !output.status.success() {
            return Err(format!("git clone failed: {}", String::from_utf8_lossy(&output.stderr)).into());
        }
    }

    // npm install
    let output = Command::new("npm")
        .arg("install")
        .arg("--silent")
        .current_dir(&dir)
        .output()?;
    if !output.status.success() {
        return Err(format!("npm install failed: {}", String::from_utf8_lossy(&output.stderr)).into());
    }

    Ok("HONE installed".to_string())
}
