//! Storage sync — every node eagerly replicates git objects when a
//! `LinkGitRefUpdate` is gossiped.
//!
//! The owner's node always stores objects locally (written during `git push`).
//! Every other node that hears the gossip checks whether it has the referenced
//! commit objects and fetches them from the peer that sent the gossip if not.
//!
//! This means every public repo is automatically replicated to every connected
//! node — no explicit "pin" step required.

use std::sync::Arc;
use anyhow::Result;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::store::Store;
use crate::linkgit_server::{pkt_line, pkt_flush, parse_pack};

/// A new commit is available on `peer_http` — go fetch it if we don't have it.
pub struct RefAvailable {
    pub repo_id: String,
    pub owner: String,
    pub repo_name: String,
    pub commit_hash: String,
    /// HTTP base URL of the peer that has the objects, e.g. "http://1.2.3.4:4242".
    /// None if we couldn't determine the peer's HTTP address (still logged, skipped).
    pub peer_http: Option<String>,
}

/// Storage sync loop — call `storage_sync::run(store, rx)` as a background task.
pub async fn run(store: Arc<Store>, mut rx: mpsc::Receiver<RefAvailable>) {
    while let Some(notif) = rx.recv().await {
        let short = short_sha(&notif.commit_hash);

        if store.git_object_exists(&notif.repo_id, &notif.commit_hash) {
            debug!("storage_sync: already have {}/{} @{}", notif.owner, notif.repo_name, short);
            continue;
        }

        let Some(peer_http) = notif.peer_http else {
            debug!("storage_sync: no peer URL for {}/{} — skipping", notif.owner, notif.repo_name);
            continue;
        };

        let store2    = store.clone();
        let repo_id   = notif.repo_id.clone();
        let owner     = notif.owner.clone();
        let repo_name = notif.repo_name.clone();
        let sha       = notif.commit_hash.clone();

        tokio::spawn(async move {
            match fetch_objects(&store2, &repo_id, &owner, &repo_name, &sha, &peer_http).await {
                Ok(n)  => info!("storage_sync: replicated {} objects for {}/{} @{}", n, owner, repo_name, short_sha(&sha)),
                Err(e) => warn!("storage_sync: fetch failed {}/{} @{} from {}: {}", owner, repo_name, short_sha(&sha), peer_http, e),
            }
        });
    }
}

/// Fetch git objects for `want_sha` from `peer_http` and store them locally.
async fn fetch_objects(
    store: &Store,
    repo_id: &str,
    owner: &str,
    repo: &str,
    want_sha: &str,
    peer_http: &str,
) -> Result<usize> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()?;

    // POST git-upload-pack: want {sha}  flush  done
    let url = format!("{}/git/{}/{}/git-upload-pack", peer_http.trim_end_matches('/'), owner, repo);
    let mut body = Vec::new();
    body.extend(pkt_line(format!("want {} ofs-delta\n", want_sha).as_bytes()));
    body.extend(pkt_flush());
    body.extend(pkt_line(b"done\n"));

    let resp = client
        .post(&url)
        .header("Content-Type", "application/x-git-upload-pack-request")
        .body(body)
        .send()
        .await?;

    if !resp.status().is_success() {
        anyhow::bail!("upload-pack returned HTTP {}", resp.status());
    }

    let data = resp.bytes().await?;

    // Response: pkt_line("NAK\n") then the PACK stream.
    // Skip forward to the PACK magic bytes.
    let pack_start = data.windows(4)
        .position(|w| w == b"PACK")
        .ok_or_else(|| anyhow::anyhow!("no PACK data in upload-pack response (got {} bytes)", data.len()))?;

    let pack_data = &data[pack_start..];
    let objects = parse_pack(pack_data, store, repo_id)?;
    let count = objects.len();
    for obj in &objects {
        store.git_object_put(repo_id, &obj.sha, obj.obj_type, &obj.data)?;
    }

    Ok(count)
}

fn short_sha(sha: &str) -> &str {
    let end = sha.len().min(8);
    &sha[..end]
}
