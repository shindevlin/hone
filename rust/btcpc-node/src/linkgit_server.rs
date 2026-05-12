//! LinkGit — smart HTTP git server.
//!
//! Implements git's smart HTTP protocol (v1) so clients can:
//!   git clone https://node:4242/git/owner/repo
//!   git push / git pull
//!
//! Object storage: CF_GIT_OBJECTS in RocksDB, keyed "{repo_id}\0{sha1hex}".
//! Ref state: embedded in the repo JSON at "linkgit:repo:{repo_id}" under .refs.
//! Auth: public repos are freely readable; write and private read require
//!   Authorization: account {account_id}  (account checked against owner/grants).
#![allow(dead_code)]

#![allow(dead_code)]
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Read;
use anyhow::Result;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use sha1::{Sha1, Digest as Sha1Digest};
use std::io::Write;

use crate::store::Store;

// ── Object type constants (git pack format) ───────────────────────────────────

pub const OBJ_COMMIT: u8 = 1;
pub const OBJ_TREE: u8 = 2;
pub const OBJ_BLOB: u8 = 3;
pub const OBJ_TAG: u8 = 4;
pub const OBJ_OFS_DELTA: u8 = 6;
pub const OBJ_REF_DELTA: u8 = 7;

// ── pkt-line codec ────────────────────────────────────────────────────────────

pub fn pkt_line(data: &[u8]) -> Vec<u8> {
    let len = data.len() + 4;
    let mut out = format!("{:04x}", len).into_bytes();
    out.extend_from_slice(data);
    out
}

pub fn pkt_str(s: &str) -> Vec<u8> { pkt_line(s.as_bytes()) }
pub fn pkt_flush() -> Vec<u8> { b"0000".to_vec() }

pub struct PktReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> PktReader<'a> {
    pub fn new(data: &'a [u8]) -> Self { Self { data, pos: 0 } }

    /// Returns `None` on flush packet or EOF; `Some(Ok(bytes))` for data.
    pub fn next(&mut self) -> Option<Result<Vec<u8>>> {
        if self.pos + 4 > self.data.len() { return None; }
        let hex = std::str::from_utf8(&self.data[self.pos..self.pos + 4]).ok()?;
        let len = usize::from_str_radix(hex, 16).ok()?;
        self.pos += 4;
        if len == 0 { return None; } // flush
        if len < 4 { return Some(Err(anyhow::anyhow!("pkt-line len {} < 4", len))); }
        let data_len = len - 4;
        if self.pos + data_len > self.data.len() {
            return Some(Err(anyhow::anyhow!("pkt-line truncated")));
        }
        let out = self.data[self.pos..self.pos + data_len].to_vec();
        self.pos += data_len;
        Some(Ok(out))
    }

    pub fn pos(&self) -> usize { self.pos }
}

// ── Git object SHA1 ───────────────────────────────────────────────────────────

pub fn object_sha1(obj_type: u8, data: &[u8]) -> String {
    let type_str = match obj_type {
        OBJ_COMMIT => "commit",
        OBJ_TREE   => "tree",
        OBJ_BLOB   => "blob",
        OBJ_TAG    => "tag",
        _          => "blob",
    };
    let header = format!("{} {}\0", type_str, data.len());
    let mut h = Sha1::new();
    h.update(header.as_bytes());
    h.update(data);
    hex::encode(h.finalize())
}

pub fn type_name(t: u8) -> &'static str {
    match t {
        OBJ_COMMIT => "commit",
        OBJ_TREE   => "tree",
        OBJ_BLOB   => "blob",
        OBJ_TAG    => "tag",
        _          => "unknown",
    }
}

// ── Pack file parser ──────────────────────────────────────────────────────────

pub struct PackObject {
    pub obj_type: u8,
    pub data: Vec<u8>,
    pub sha: String,
}

pub fn parse_pack(pack_data: &[u8], store: &Store, repo_id: &str) -> Result<Vec<PackObject>> {
    if pack_data.len() < 12 { anyhow::bail!("pack file too short"); }
    if &pack_data[0..4] != b"PACK" { anyhow::bail!("missing PACK magic"); }
    let count = u32::from_be_bytes([pack_data[8], pack_data[9], pack_data[10], pack_data[11]]) as usize;

    struct Raw {
        obj_type: u8,
        data: Vec<u8>,
        base_sha: Option<String>, // REF_DELTA
        base_ofs: Option<usize>,  // OFS_DELTA absolute pack offset of base
        pack_offset: usize,
    }

    let mut raw_entries: Vec<Raw> = Vec::with_capacity(count);
    let mut offset_to_idx: HashMap<usize, usize> = HashMap::new();
    let mut pos = 12usize;
    // Exclude 20-byte trailing checksum from decompressor's view.
    let data_end = pack_data.len().saturating_sub(20);

    for _ in 0..count {
        let pack_offset = pos;

        // Read variable-length type+size header.
        let b0 = *pack_data.get(pos).ok_or_else(|| anyhow::anyhow!("pack truncated at object header"))?;
        pos += 1;
        let obj_type = (b0 >> 4) & 0x07;
        let mut size = (b0 & 0x0f) as usize;
        let mut shift = 4usize;
        let mut cur = b0;
        while cur & 0x80 != 0 {
            cur = *pack_data.get(pos).ok_or_else(|| anyhow::anyhow!("pack truncated in size varint"))?;
            pos += 1;
            size |= ((cur & 0x7f) as usize) << shift;
            shift += 7;
        }
        let _ = size; // zlib handles actual length

        let mut base_sha: Option<String> = None;
        let mut base_ofs: Option<usize> = None;

        match obj_type {
            OBJ_OFS_DELTA => {
                // Negative base offset with MSB +1 bias on continuation bytes.
                let mut b = *pack_data.get(pos).ok_or_else(|| anyhow::anyhow!("pack truncated in ofs header"))?;
                pos += 1;
                let mut ofs = (b & 0x7f) as usize;
                while b & 0x80 != 0 {
                    b = *pack_data.get(pos).ok_or_else(|| anyhow::anyhow!("pack truncated in ofs"))?;
                    pos += 1;
                    ofs = ((ofs + 1) << 7) | (b & 0x7f) as usize;
                }
                base_ofs = Some(pack_offset.checked_sub(ofs).ok_or_else(|| anyhow::anyhow!("ofs_delta underflow"))?);
            }
            OBJ_REF_DELTA => {
                if pos + 20 > pack_data.len() { anyhow::bail!("pack truncated in ref_delta sha"); }
                base_sha = Some(hex::encode(&pack_data[pos..pos + 20]));
                pos += 20;
            }
            _ => {}
        }

        // Decompress zlib stream.
        let mut decoder = ZlibDecoder::new(&pack_data[pos..data_end]);
        let mut decompressed = Vec::new();
        decoder.read_to_end(&mut decompressed)?;
        pos += decoder.total_in() as usize;

        let idx = raw_entries.len();
        offset_to_idx.insert(pack_offset, idx);
        raw_entries.push(Raw { obj_type, data: decompressed, base_sha, base_ofs, pack_offset });
    }

    // Resolve non-delta objects first.
    let mut resolved: Vec<Option<PackObject>> = raw_entries.iter().map(|e| {
        if e.obj_type >= 1 && e.obj_type <= 4 {
            let sha = object_sha1(e.obj_type, &e.data);
            Some(PackObject { obj_type: e.obj_type, data: e.data.clone(), sha })
        } else { None }
    }).collect();

    // Iteratively resolve delta objects (handles chains of deltas).
    let mut progress = true;
    while progress {
        progress = false;
        for i in 0..raw_entries.len() {
            if resolved[i].is_some() { continue; }
            let e = &raw_entries[i];

            let base = if let Some(ref bsha) = e.base_sha {
                // REF_DELTA: base is identified by SHA.
                resolved.iter().find_map(|r| r.as_ref().filter(|o| &o.sha == bsha))
                    .map(|o| (o.obj_type, o.data.clone()))
                    .or_else(|| {
                        store.git_object_get(repo_id, bsha).map(|raw| (raw[0], raw[1..].to_vec()))
                    })
            } else if let Some(base_abs) = e.base_ofs {
                // OFS_DELTA: base is at an absolute pack offset.
                offset_to_idx.get(&base_abs)
                    .and_then(|&bi| resolved[bi].as_ref())
                    .map(|o| (o.obj_type, o.data.clone()))
            } else { None };

            if let Some((base_type, base_data)) = base {
                let out_data = apply_delta(&base_data, &e.data);
                let sha = object_sha1(base_type, &out_data);
                resolved[i] = Some(PackObject { obj_type: base_type, data: out_data, sha });
                progress = true;
            }
        }
    }

    Ok(resolved.into_iter().flatten().collect())
}

// ── Git delta application ─────────────────────────────────────────────────────

fn apply_delta(base: &[u8], delta: &[u8]) -> Vec<u8> {
    let mut pos = 0usize;
    let (_src_size, n) = read_delta_varint(delta, pos); pos += n;
    let (target_size, n) = read_delta_varint(delta, pos); pos += n;
    let mut out = Vec::with_capacity(target_size);

    while pos < delta.len() {
        let cmd = delta[pos]; pos += 1;
        if cmd & 0x80 != 0 {
            // Copy from base.
            let mut copy_off = 0usize;
            let mut copy_len = 0usize;
            if cmd & 0x01 != 0 { copy_off  |= delta[pos] as usize;        pos += 1; }
            if cmd & 0x02 != 0 { copy_off  |= (delta[pos] as usize) << 8; pos += 1; }
            if cmd & 0x04 != 0 { copy_off  |= (delta[pos] as usize) << 16; pos += 1; }
            if cmd & 0x08 != 0 { copy_off  |= (delta[pos] as usize) << 24; pos += 1; }
            if cmd & 0x10 != 0 { copy_len  |= delta[pos] as usize;         pos += 1; }
            if cmd & 0x20 != 0 { copy_len  |= (delta[pos] as usize) << 8;  pos += 1; }
            if cmd & 0x40 != 0 { copy_len  |= (delta[pos] as usize) << 16; pos += 1; }
            if copy_len == 0 { copy_len = 0x10000; }
            let end = (copy_off + copy_len).min(base.len());
            out.extend_from_slice(&base[copy_off..end]);
        } else if cmd > 0 {
            // Insert literal bytes.
            let n = cmd as usize;
            if pos + n <= delta.len() {
                out.extend_from_slice(&delta[pos..pos + n]);
                pos += n;
            }
        }
    }
    out
}

fn read_delta_varint(data: &[u8], mut pos: usize) -> (usize, usize) {
    let start = pos;
    let mut val = 0usize;
    let mut shift = 0u32;
    loop {
        if pos >= data.len() { break; }
        let b = data[pos]; pos += 1;
        val |= ((b & 0x7f) as usize) << shift;
        shift += 7;
        if b & 0x80 == 0 { break; }
    }
    (val, pos - start)
}

// ── Pack builder (for serving clone/fetch) ────────────────────────────────────

pub fn build_pack(objects: &[(u8, Vec<u8>)]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(b"PACK");
    body.extend_from_slice(&2u32.to_be_bytes());
    body.extend_from_slice(&(objects.len() as u32).to_be_bytes());

    for (obj_type, data) in objects {
        // Variable-length type+size header.
        let size = data.len();
        let first = ((*obj_type & 0x07) << 4) | (size & 0x0f) as u8;
        let mut size_rem = size >> 4;
        if size_rem > 0 {
            body.push(first | 0x80);
            loop {
                let byte = (size_rem & 0x7f) as u8;
                size_rem >>= 7;
                if size_rem > 0 { body.push(byte | 0x80); } else { body.push(byte); break; }
            }
        } else {
            body.push(first);
        }

        // Zlib-compress the data.
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
        enc.write_all(data).unwrap_or_default();
        body.extend(enc.finish().unwrap_or_default());
    }

    // Trailing SHA1 checksum of everything.
    let mut h = Sha1::new();
    h.update(&body);
    body.extend_from_slice(&h.finalize());
    body
}

// ── Object graph traversal ────────────────────────────────────────────────────

/// Collect all objects reachable from `want_shas` that are not reachable
/// from (i.e. not in) `have_shas`. Used to build a pack for clone/fetch.
pub fn collect_reachable(
    store: &Store,
    repo_id: &str,
    want_shas: &[String],
    have_shas: &HashSet<String>,
) -> Vec<(u8, Vec<u8>)> {
    let mut visited: HashSet<String> = have_shas.clone();
    let mut queue: VecDeque<String> = want_shas.iter().cloned().collect();
    let mut objects: Vec<(u8, Vec<u8>)> = Vec::new();

    while let Some(sha) = queue.pop_front() {
        if visited.contains(&sha) { continue; }
        visited.insert(sha.clone());

        let Some(raw) = store.git_object_get(repo_id, &sha) else { continue };
        if raw.is_empty() { continue; }
        let (obj_type, data) = (raw[0], raw[1..].to_vec());

        match obj_type {
            OBJ_COMMIT => {
                let (tree_sha, parents) = parse_commit(&data);
                queue.push_back(tree_sha);
                for p in parents { queue.push_back(p); }
            }
            OBJ_TREE => {
                for child_sha in parse_tree(&data) { queue.push_back(child_sha); }
            }
            _ => {}
        }
        objects.push((obj_type, data));
    }
    objects
}

fn parse_commit(data: &[u8]) -> (String, Vec<String>) {
    let text = std::str::from_utf8(data).unwrap_or("");
    let mut tree = String::new();
    let mut parents = Vec::new();
    for line in text.lines() {
        if let Some(s) = line.strip_prefix("tree ")   { tree = s.trim().to_owned(); }
        else if let Some(s) = line.strip_prefix("parent ") { parents.push(s.trim().to_owned()); }
        else if line.is_empty() { break; }
    }
    (tree, parents)
}

fn parse_tree(data: &[u8]) -> Vec<String> {
    let mut shas = Vec::new();
    let mut pos = 0usize;
    while pos < data.len() {
        // Each entry: "{mode} {name}\0{20-byte sha}"
        let nul = match data[pos..].iter().position(|&b| b == 0) {
            Some(n) => pos + n,
            None => break,
        };
        pos = nul + 1;
        if pos + 20 > data.len() { break; }
        shas.push(hex::encode(&data[pos..pos + 20]));
        pos += 20;
    }
    shas
}

// ── Auth ──────────────────────────────────────────────────────────────────────

/// Find a repo by owner+name. Returns (repo_id, repo_json) or None.
pub fn find_repo(store: &Store, owner: &str, name: &str) -> Option<(String, serde_json::Value)> {
    let idx_key = format!("linkgit:byname:{}:{}", owner, name);
    let repo_id = String::from_utf8(store.get_meta(&idx_key)?).ok()?;
    let repo = store.get_meta(&format!("linkgit:repo:{}", repo_id))
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())?;
    Some((repo_id, repo))
}

/// Extract account from `Authorization: account {id}` header.
pub fn auth_account(auth_header: Option<&str>) -> Option<&str> {
    auth_header?.strip_prefix("account ")?.trim().into()
}

pub fn can_read(store: &Store, repo: &serde_json::Value, repo_id: &str, auth: Option<&str>) -> bool {
    if repo["visibility"].as_str() == Some("public") { return true; }
    let account = auth.unwrap_or("");
    if account.is_empty() { return false; }
    repo["owner"].as_str() == Some(account)
        || store.get_meta(&format!("linkgit:access:{}:{}", repo_id, account)).is_some()
}

pub fn can_write(store: &Store, repo: &serde_json::Value, repo_id: &str, auth: Option<&str>) -> bool {
    let account = auth.unwrap_or("");
    if account.is_empty() { return false; }
    repo["owner"].as_str() == Some(account)
        || store.get_meta(&format!("linkgit:access:{}:{}", repo_id, account)).is_some()
}

// ── Response types ────────────────────────────────────────────────────────────

pub struct GitResponse {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

impl GitResponse {
    fn ok(ct: &'static str, body: Vec<u8>) -> Self { Self { status: 200, content_type: ct, body } }
    fn err(status: u16, msg: &str) -> Self {
        Self { status, content_type: "text/plain", body: msg.as_bytes().to_vec() }
    }
}

// ── info/refs ─────────────────────────────────────────────────────────────────

pub fn info_refs(store: &Store, owner: &str, repo_name: &str, service: &str, auth: Option<&str>) -> GitResponse {
    let Some((repo_id, repo)) = find_repo(store, owner, repo_name) else {
        return GitResponse::err(404, "repository not found");
    };
    let is_push = service == "git-receive-pack";
    if is_push {
        if !can_write(store, &repo, &repo_id, auth) {
            return GitResponse::err(401, "authentication required");
        }
    } else if !can_read(store, &repo, &repo_id, auth) {
        return GitResponse::err(401, "authentication required");
    }

    let caps = if is_push {
        "report-status delete-refs ofs-delta agent=git/btcpc"
    } else {
        "ofs-delta agent=git/btcpc"
    };

    let ct: &'static str = if is_push {
        "application/x-git-receive-pack-advertisement"
    } else {
        "application/x-git-upload-pack-advertisement"
    };

    // Read refs from embedded repo JSON.
    let refs_obj = repo["refs"].as_object().cloned().unwrap_or_default();
    let mut refs: Vec<(String, String)> = refs_obj.into_iter()
        .filter_map(|(k, v)| v.as_str().map(|s| (k, s.to_owned())))
        .collect();
    refs.sort_by(|a, b| a.0.cmp(&b.0));

    let mut body = Vec::new();
    let svc_line = format!("# service={}\n", service);
    body.extend(pkt_line(svc_line.as_bytes()));
    body.extend(pkt_flush());

    if refs.is_empty() {
        // Empty repo: advertise zero-sha with capabilities.
        let zero = "0000000000000000000000000000000000000000";
        let line = format!("{} capabilities^{{}}\0{}\n", zero, caps);
        body.extend(pkt_line(line.as_bytes()));
    } else {
        // HEAD points to default branch.
        let head_sha = refs.iter().find(|(r, _)| r == "refs/heads/main")
            .or_else(|| refs.iter().find(|(r, _)| r == "refs/heads/master"))
            .map(|(_, s)| s.as_str())
            .unwrap_or(&refs[0].1);
        let head_line = format!("{} HEAD\0{}\n", head_sha, caps);
        body.extend(pkt_line(head_line.as_bytes()));
        for (ref_name, sha) in &refs {
            let line = format!("{} {}\n", sha, ref_name);
            body.extend(pkt_line(line.as_bytes()));
        }
    }
    body.extend(pkt_flush());
    GitResponse::ok(ct, body)
}

// ── upload-pack (clone / fetch) ───────────────────────────────────────────────

pub fn upload_pack(store: &Store, owner: &str, repo_name: &str, request: &[u8], auth: Option<&str>) -> GitResponse {
    let Some((repo_id, repo)) = find_repo(store, owner, repo_name) else {
        return GitResponse::err(404, "repository not found");
    };
    if !can_read(store, &repo, &repo_id, auth) {
        return GitResponse::err(401, "authentication required");
    }

    let mut reader = PktReader::new(request);
    let mut wants: Vec<String> = Vec::new();
    let mut haves: HashSet<String> = HashSet::new();

    while let Some(Ok(line)) = reader.next() {
        let s = String::from_utf8_lossy(&line);
        let s = s.trim_end_matches('\n');
        // Strip capabilities (after first NUL on the first want line).
        let s = s.split('\0').next().unwrap_or(s);
        if let Some(rest) = s.strip_prefix("want ") {
            let sha = rest.split_whitespace().next().unwrap_or("").to_owned();
            if !sha.is_empty() { wants.push(sha); }
        } else if let Some(sha) = s.strip_prefix("have ") {
            haves.insert(sha.trim().to_owned());
        }
    }

    let objects = collect_reachable(store, &repo_id, &wants, &haves);
    let pack = build_pack(&objects);

    let mut resp = Vec::new();
    resp.extend(pkt_str("NAK\n"));
    resp.extend(pack);
    GitResponse::ok("application/x-git-upload-pack-result", resp)
}

// ── receive-pack (push) ───────────────────────────────────────────────────────

/// Parsed push request: ref updates + raw pack bytes.
pub struct ReceivePackRequest {
    /// (old_sha, new_sha, ref_name)
    pub updates: Vec<(String, String, String)>,
    pub pack: Vec<u8>,
}

pub fn parse_receive_pack(request: &[u8]) -> ReceivePackRequest {
    let mut reader = PktReader::new(request);
    let mut updates = Vec::new();

    while let Some(Ok(line)) = reader.next() {
        let s = String::from_utf8_lossy(&line);
        let s = s.trim_end_matches('\n');
        let s = s.split('\0').next().unwrap_or(s);
        let parts: Vec<&str> = s.splitn(3, ' ').collect();
        if parts.len() == 3 {
            updates.push((parts[0].to_owned(), parts[1].to_owned(), parts[2].to_owned()));
        }
    }

    let pack = request[reader.pos()..].to_vec();
    ReceivePackRequest { updates, pack }
}

/// Unpack and store objects. Returns list of (sha, type, data).
pub fn store_pack(store: &Store, repo_id: &str, pack: &[u8]) -> Result<Vec<PackObject>> {
    if pack.is_empty() { return Ok(vec![]); }
    let objects = parse_pack(pack, store, repo_id)?;
    for obj in &objects {
        store.git_object_put(repo_id, &obj.sha, obj.obj_type, &obj.data)?;
    }
    Ok(objects)
}

/// Build the receive-pack response (status report).
pub fn receive_pack_response(updates: &[(String, String, String)], errors: &HashMap<String, String>) -> Vec<u8> {
    let mut resp = Vec::new();
    resp.extend(pkt_str("unpack ok\n"));
    for (_, _, ref_name) in updates {
        if let Some(err) = errors.get(ref_name) {
            let line = format!("ng {} {}\n", ref_name, err);
            resp.extend(pkt_str(&line));
        } else {
            let line = format!("ok {}\n", ref_name);
            resp.extend(pkt_str(&line));
        }
    }
    resp.extend(pkt_flush());
    resp
}

// ── LinkGit public REST API ───────────────────────────────────────────────────
//
// All data is stored in CF_META (via state_set/state_get/state_scan_prefix).
// Key patterns:
//   git_repo:{owner}/{name}            → RepoMeta JSON
//   git_issue:{owner}/{repo}:{number}  → IssueMeta JSON
//   git_issue_counter:{owner}/{repo}   → u64 LE bytes
//   git_hook:{owner}/{repo}:{uuid}     → HookMeta JSON
//   git_token:{token_id}               → TokenMeta JSON (token stored as sha256 hash)
//
// Authentication: write operations require X-BTCPC-Account + X-BTCPC-Signature
// + X-BTCPC-Timestamp headers.  The signature is ed25519 over the message
// "linkgit:{account}:{timestamp}" using the account's posting key.
// Timestamp must be within 300 seconds of server time to prevent replay.

use crate::api::AppState;
use axum::{
    Router,
    routing::get,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
    response::IntoResponse,
};
use serde::Deserialize;
use sha2::Sha256;
use std::time::{SystemTime, UNIX_EPOCH};

// ── helpers ───────────────────────────────────────────────────────────────────

/// Extract the requesting account from X-BTCPC-Account header.
fn req_account(headers: &HeaderMap) -> Option<String> {
    headers.get("X-BTCPC-Account")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Check that the account header is present, the account has a registered key,
/// and — when X-BTCPC-Signature is provided — the ed25519 signature is valid.
///
/// Signing message: "linkgit:{account}:{timestamp}" (timestamp = Unix seconds).
/// X-BTCPC-Timestamp must be within 300 seconds of server time.
fn require_auth(headers: &HeaderMap, state: &AppState) -> Result<String, (StatusCode, Json<serde_json::Value>)> {
    let account = req_account(headers)
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error": "X-BTCPC-Account header required"}))))?;

    // Verify account exists and has a key.
    let acct_state = state.chain.store.get_account(&account).ok().flatten();
    let key_present = acct_state.is_some()
        || state.chain.store.state_get(&format!("key:{}:active", account)).is_some();
    if !key_present {
        return Err((StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error": "account not found or has no active key"}))));
    }

    // Verify signature when provided.
    if let Some(sig_hex) = headers.get("X-BTCPC-Signature").and_then(|v| v.to_str().ok()) {
        let ts_str = headers.get("X-BTCPC-Timestamp")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error": "X-BTCPC-Timestamp required with X-BTCPC-Signature"}))))?;

        let ts: u64 = ts_str.parse().map_err(|_| {
            (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error": "invalid X-BTCPC-Timestamp"})))
        })?;
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        if now.saturating_sub(ts) > 300 || ts.saturating_sub(now) > 30 {
            return Err((StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error": "X-BTCPC-Timestamp out of acceptable window"}))));
        }

        // Resolve posting key: prefer account state keys.posting, fall back to key:{account}:active.
        let pubkey_hex = acct_state
            .as_ref()
            .and_then(|s| s.get("keys"))
            .and_then(|k| k.get("posting"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .or_else(|| {
                state.chain.store.state_get(&format!("key:{}:active", account))
                    .and_then(|v| String::from_utf8(v).ok())
            })
            .ok_or_else(|| (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error": "account has no posting key registered"}))))?;

        let message = format!("linkgit:{}:{}", account, ts_str);

        let pk_bytes = hex::decode(&pubkey_hex).map_err(|_| {
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "stored key is not valid hex"})))
        })?;
        let pk_array: [u8; 32] = pk_bytes.try_into().map_err(|_| {
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "stored key must be 32 bytes"})))
        })?;
        let verifying_key = ed25519_dalek::VerifyingKey::from_bytes(&pk_array).map_err(|_| {
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "invalid ed25519 public key"})))
        })?;

        let sig_bytes = hex::decode(sig_hex).map_err(|_| {
            (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error": "X-BTCPC-Signature is not valid hex"})))
        })?;
        let sig_array: [u8; 64] = sig_bytes.try_into().map_err(|_| {
            (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error": "X-BTCPC-Signature must be 64 bytes"})))
        })?;
        let signature = ed25519_dalek::Signature::from_bytes(&sig_array);

        use ed25519_dalek::Verifier;
        verifying_key.verify(message.as_bytes(), &signature).map_err(|_| {
            (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error": "signature verification failed"})))
        })?;
    }

    Ok(account)
}

/// Generate a short random hex id (16 bytes = 32 hex chars).
fn random_id() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let bytes: [u8; 16] = rng.gen();
    hex::encode(bytes)
}

/// Sha256 hex of input bytes.
fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    hex::encode(h.finalize())
}

// ── Repos ─────────────────────────────────────────────────────────────────────

/// GET /git/api/repos/:owner — list repos for owner
async fn api_list_repos(
    State(s): State<AppState>,
    Path(owner): Path<String>,
) -> Json<serde_json::Value> {
    let prefix = format!("git_repo:{}/", owner);
    let repos: Vec<serde_json::Value> = s.chain.store.state_scan_prefix(&prefix)
        .into_iter()
        .filter_map(|(_, v)| serde_json::from_slice::<serde_json::Value>(&v).ok())
        .filter(|r| r.get("is_private").and_then(|p| p.as_bool()) != Some(true))
        .collect();
    Json(serde_json::json!({ "owner": owner, "repos": repos, "count": repos.len() }))
}

/// GET /git/api/repos/:owner/:repo — get repo metadata
async fn api_get_repo(
    State(s): State<AppState>,
    Path((owner, repo)): Path<(String, String)>,
) -> impl IntoResponse {
    let key = format!("git_repo:{}/{}", owner, repo);
    match s.chain.store.state_get(&key) {
        Some(v) => match serde_json::from_slice::<serde_json::Value>(&v) {
            Ok(data) => (StatusCode::OK, Json(data)).into_response(),
            Err(_)   => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "corrupt metadata"}))).into_response(),
        },
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "repo not found"}))).into_response(),
    }
}

#[derive(Deserialize)]
struct CreateRepoBody {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    private: bool,
    #[serde(default = "default_branch")]
    default_branch: String,
}
fn default_branch() -> String { "main".to_owned() }

/// POST /git/api/repos/:owner — create a repo
async fn api_create_repo(
    State(s): State<AppState>,
    Path(owner): Path<String>,
    headers: HeaderMap,
    Json(body): Json<CreateRepoBody>,
) -> impl IntoResponse {
    let account = match require_auth(&headers, &s) {
        Ok(a)  => a,
        Err(e) => return e.into_response(),
    };
    if account != owner {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "can only create repos under your own account"}))).into_response();
    }
    let key = format!("git_repo:{}/{}", owner, body.name);
    if s.chain.store.state_get(&key).is_some() {
        return (StatusCode::CONFLICT, Json(serde_json::json!({"error": "repo already exists"}))).into_response();
    }
    let epoch = s.chain.current_epoch();
    let clone_url = format!("https://git.btcpc.net/{}/{}", owner, body.name);
    let meta = serde_json::json!({
        "owner":          owner,
        "name":           body.name,
        "description":    body.description,
        "default_branch": body.default_branch,
        "clone_url":      clone_url,
        "created_epoch":  epoch,
        "is_private":     body.private,
        "stars":          0u64,
    });
    if let Err(e) = s.chain.store.state_set(&key, &serde_json::to_vec(&meta).unwrap_or_default()) {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))).into_response();
    }
    (StatusCode::CREATED, Json(meta)).into_response()
}

/// DELETE /git/api/repos/:owner/:repo — delete a repo
async fn api_delete_repo(
    State(s): State<AppState>,
    Path((owner, repo)): Path<(String, String)>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let account = match require_auth(&headers, &s) {
        Ok(a)  => a,
        Err(e) => return e.into_response(),
    };
    if account != owner {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "only owner can delete repo"}))).into_response();
    }
    let key = format!("git_repo:{}/{}", owner, repo);
    if s.chain.store.state_get(&key).is_none() {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "repo not found"}))).into_response();
    }
    let _ = s.chain.store.state_delete(&key);
    (StatusCode::OK, Json(serde_json::json!({"deleted": true, "repo": repo}))).into_response()
}

#[derive(Deserialize)]
struct UpdateRepoBody {
    description: Option<String>,
    private: Option<bool>,
}

/// PATCH /git/api/repos/:owner/:repo — update description / private flag
async fn api_update_repo(
    State(s): State<AppState>,
    Path((owner, repo)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<UpdateRepoBody>,
) -> impl IntoResponse {
    let account = match require_auth(&headers, &s) {
        Ok(a)  => a,
        Err(e) => return e.into_response(),
    };
    if account != owner {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "only owner can update repo"}))).into_response();
    }
    let key = format!("git_repo:{}/{}", owner, repo);
    let mut meta: serde_json::Value = match s.chain.store.state_get(&key)
        .and_then(|v| serde_json::from_slice(&v).ok())
    {
        Some(m) => m,
        None    => return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "repo not found"}))).into_response(),
    };
    if let Some(desc) = body.description { meta["description"] = serde_json::json!(desc); }
    if let Some(priv_flag) = body.private { meta["is_private"] = serde_json::json!(priv_flag); }
    let _ = s.chain.store.state_set(&key, &serde_json::to_vec(&meta).unwrap_or_default());
    (StatusCode::OK, Json(meta)).into_response()
}

// ── Issues ────────────────────────────────────────────────────────────────────

/// GET /git/api/repos/:owner/:repo/issues — list issues, optional ?state=open|closed
async fn api_list_issues(
    State(s): State<AppState>,
    Path((owner, repo)): Path<(String, String)>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let state_filter = params.get("state").map(String::as_str);
    let prefix = format!("git_issue:{}/{}:", owner, repo);
    let issues: Vec<serde_json::Value> = s.chain.store.state_scan_prefix(&prefix)
        .into_iter()
        .filter_map(|(_, v)| serde_json::from_slice::<serde_json::Value>(&v).ok())
        .filter(|i| {
            if let Some(filter) = state_filter {
                i.get("state").and_then(|st| st.as_str()) == Some(filter)
            } else {
                true
            }
        })
        .collect();
    (StatusCode::OK, Json(serde_json::json!({ "issues": issues, "count": issues.len() }))).into_response()
}

/// GET /git/api/repos/:owner/:repo/issues/:number — single issue
async fn api_get_issue(
    State(s): State<AppState>,
    Path((owner, repo, number)): Path<(String, String, u64)>,
) -> impl IntoResponse {
    let key = format!("git_issue:{}/{}:{}", owner, repo, number);
    match s.chain.store.state_get(&key) {
        Some(v) => match serde_json::from_slice::<serde_json::Value>(&v) {
            Ok(data) => (StatusCode::OK, Json(data)).into_response(),
            Err(_)   => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "corrupt metadata"}))).into_response(),
        },
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "issue not found"}))).into_response(),
    }
}

#[derive(Deserialize)]
struct CreateIssueBody {
    title: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    labels: Vec<String>,
}

/// POST /git/api/repos/:owner/:repo/issues — create issue
async fn api_create_issue(
    State(s): State<AppState>,
    Path((owner, repo)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<CreateIssueBody>,
) -> impl IntoResponse {
    let account = match require_auth(&headers, &s) {
        Ok(a)  => a,
        Err(e) => return e.into_response(),
    };
    // Verify repo exists.
    let repo_key = format!("git_repo:{}/{}", owner, repo);
    if s.chain.store.state_get(&repo_key).is_none() {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "repo not found"}))).into_response();
    }
    // Auto-increment issue number.
    let counter_key = format!("git_issue_counter:{}/{}", owner, repo);
    let number: u64 = s.chain.store.state_get(&counter_key)
        .and_then(|b| b.try_into().ok())
        .map(u64::from_le_bytes)
        .unwrap_or(0) + 1;
    let _ = s.chain.store.state_set(&counter_key, &number.to_le_bytes());

    let epoch = s.chain.current_epoch();
    let meta = serde_json::json!({
        "number":  number,
        "title":   body.title,
        "body":    body.body,
        "labels":  body.labels,
        "state":   "open",
        "author":  account,
        "repo":    format!("{}/{}", owner, repo),
        "created_epoch": epoch,
        "updated_epoch": epoch,
    });
    let issue_key = format!("git_issue:{}/{}:{}", owner, repo, number);
    if let Err(e) = s.chain.store.state_set(&issue_key, &serde_json::to_vec(&meta).unwrap_or_default()) {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))).into_response();
    }
    (StatusCode::CREATED, Json(meta)).into_response()
}

#[derive(Deserialize)]
struct UpdateIssueBody {
    state: Option<String>,
    title: Option<String>,
    body: Option<String>,
}

/// PATCH /git/api/repos/:owner/:repo/issues/:number — update state/title/body
async fn api_update_issue(
    State(s): State<AppState>,
    Path((owner, repo, number)): Path<(String, String, u64)>,
    headers: HeaderMap,
    Json(body): Json<UpdateIssueBody>,
) -> impl IntoResponse {
    let account = match require_auth(&headers, &s) {
        Ok(a)  => a,
        Err(e) => return e.into_response(),
    };
    // Repo owner or issue author can update.
    let issue_key = format!("git_issue:{}/{}:{}", owner, repo, number);
    let mut meta: serde_json::Value = match s.chain.store.state_get(&issue_key)
        .and_then(|v| serde_json::from_slice(&v).ok())
    {
        Some(m) => m,
        None    => return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "issue not found"}))).into_response(),
    };
    let issue_author = meta.get("author").and_then(|a| a.as_str()).unwrap_or("").to_owned();
    if account != owner && account != issue_author {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "not authorized to update this issue"}))).into_response();
    }
    if let Some(st) = body.state {
        if st != "open" && st != "closed" {
            return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "state must be 'open' or 'closed'"}))).into_response();
        }
        meta["state"] = serde_json::json!(st);
    }
    if let Some(t) = body.title { meta["title"] = serde_json::json!(t); }
    if let Some(b) = body.body  { meta["body"]  = serde_json::json!(b); }
    meta["updated_epoch"] = serde_json::json!(s.chain.current_epoch());
    let _ = s.chain.store.state_set(&issue_key, &serde_json::to_vec(&meta).unwrap_or_default());
    (StatusCode::OK, Json(meta)).into_response()
}

// ── Hooks (webhooks) ──────────────────────────────────────────────────────────

/// GET /git/api/repos/:owner/:repo/hooks — list hooks
async fn api_list_hooks(
    State(s): State<AppState>,
    Path((owner, repo)): Path<(String, String)>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let account = match require_auth(&headers, &s) {
        Ok(a)  => a,
        Err(e) => return e.into_response(),
    };
    if account != owner {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "only owner can list hooks"}))).into_response();
    }
    let prefix = format!("git_hook:{}/{}:", owner, repo);
    let hooks: Vec<serde_json::Value> = s.chain.store.state_scan_prefix(&prefix)
        .into_iter()
        .filter_map(|(_, v)| serde_json::from_slice::<serde_json::Value>(&v).ok())
        // Never return the stored secret.
        .map(|mut h| { h.as_object_mut().map(|o| o.remove("secret_hash")); h })
        .collect();
    (StatusCode::OK, Json(serde_json::json!({ "hooks": hooks, "count": hooks.len() }))).into_response()
}

#[derive(Deserialize)]
struct CreateHookBody {
    url: String,
    #[serde(default)]
    events: Vec<String>,
    #[serde(default)]
    secret: String,
}

/// POST /git/api/repos/:owner/:repo/hooks — register a webhook
async fn api_create_hook(
    State(s): State<AppState>,
    Path((owner, repo)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<CreateHookBody>,
) -> impl IntoResponse {
    let account = match require_auth(&headers, &s) {
        Ok(a)  => a,
        Err(e) => return e.into_response(),
    };
    if account != owner {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "only owner can create hooks"}))).into_response();
    }
    if s.chain.store.state_get(&format!("git_repo:{}/{}", owner, repo)).is_none() {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "repo not found"}))).into_response();
    }
    let hook_id = random_id();
    let epoch   = s.chain.current_epoch();
    let secret_hash = if body.secret.is_empty() { String::new() } else { sha256_hex(body.secret.as_bytes()) };
    let meta = serde_json::json!({
        "id":           hook_id,
        "url":          body.url,
        "events":       body.events,
        "secret_hash":  secret_hash,
        "active":       true,
        "created_epoch": epoch,
    });
    let key = format!("git_hook:{}/{}:{}", owner, repo, hook_id);
    if let Err(e) = s.chain.store.state_set(&key, &serde_json::to_vec(&meta).unwrap_or_default()) {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))).into_response();
    }
    // Return without secret_hash in the response.
    let mut resp = meta.clone();
    resp.as_object_mut().map(|o| o.remove("secret_hash"));
    (StatusCode::CREATED, Json(resp)).into_response()
}

/// DELETE /git/api/repos/:owner/:repo/hooks/:id — delete a webhook
async fn api_delete_hook(
    State(s): State<AppState>,
    Path((owner, repo, hook_id)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let account = match require_auth(&headers, &s) {
        Ok(a)  => a,
        Err(e) => return e.into_response(),
    };
    if account != owner {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "only owner can delete hooks"}))).into_response();
    }
    let key = format!("git_hook:{}/{}:{}", owner, repo, hook_id);
    if s.chain.store.state_get(&key).is_none() {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "hook not found"}))).into_response();
    }
    let _ = s.chain.store.state_delete(&key);
    (StatusCode::OK, Json(serde_json::json!({"deleted": true, "id": hook_id}))).into_response()
}

// ── Tokens (CI/bot API tokens) ────────────────────────────────────────────────

#[derive(Deserialize)]
struct CreateTokenBody {
    name: String,
    #[serde(default)]
    scopes: Vec<String>,
    account: String,
}

/// POST /git/api/tokens — create an API token
async fn api_create_token(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateTokenBody>,
) -> impl IntoResponse {
    let account = match require_auth(&headers, &s) {
        Ok(a)  => a,
        Err(e) => return e.into_response(),
    };
    if account != body.account {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "account mismatch"}))).into_response();
    }
    // Generate a random token and store only its hash.
    use rand::Rng;
    let raw: [u8; 32] = rand::thread_rng().gen();
    let token_plain = format!("btcpc_{}", hex::encode(raw));
    let token_hash  = sha256_hex(token_plain.as_bytes());
    let token_id    = hex::encode(&raw[..8]);
    let epoch       = s.chain.current_epoch();

    let meta = serde_json::json!({
        "id":           token_id,
        "name":         body.name,
        "scopes":       body.scopes,
        "account":      account,
        "token_hash":   token_hash,
        "created_epoch": epoch,
        "revoked":      false,
    });
    let key = format!("git_token:{}", token_id);
    if let Err(e) = s.chain.store.state_set(&key, &serde_json::to_vec(&meta).unwrap_or_default()) {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))).into_response();
    }
    // Also keep a per-account index entry for listing.
    let idx_key = format!("git_token_account:{}:{}", account, token_id);
    let _ = s.chain.store.state_set(&idx_key, token_id.as_bytes());

    // Return the plaintext token once — caller must store it.
    let mut resp = meta.clone();
    resp.as_object_mut().map(|o| o.remove("token_hash"));
    resp["token"] = serde_json::json!(token_plain);
    (StatusCode::CREATED, Json(resp)).into_response()
}

/// DELETE /git/api/tokens/:token_id — revoke a token
async fn api_revoke_token(
    State(s): State<AppState>,
    Path(token_id): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let account = match require_auth(&headers, &s) {
        Ok(a)  => a,
        Err(e) => return e.into_response(),
    };
    let key = format!("git_token:{}", token_id);
    let mut meta: serde_json::Value = match s.chain.store.state_get(&key)
        .and_then(|v| serde_json::from_slice(&v).ok())
    {
        Some(m) => m,
        None    => return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "token not found"}))).into_response(),
    };
    if meta.get("account").and_then(|a| a.as_str()) != Some(&account) {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "not your token"}))).into_response();
    }
    meta["revoked"] = serde_json::json!(true);
    let _ = s.chain.store.state_set(&key, &serde_json::to_vec(&meta).unwrap_or_default());
    (StatusCode::OK, Json(serde_json::json!({"revoked": true, "id": token_id}))).into_response()
}

/// GET /git/api/tokens — list tokens for authenticated account
async fn api_list_tokens(
    State(s): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let account = match require_auth(&headers, &s) {
        Ok(a)  => a,
        Err(e) => return e.into_response(),
    };
    let prefix = format!("git_token_account:{}:", account);
    let tokens: Vec<serde_json::Value> = s.chain.store.state_scan_prefix(&prefix)
        .into_iter()
        .filter_map(|(_, id_bytes)| {
            let id = String::from_utf8(id_bytes).ok()?;
            let key = format!("git_token:{}", id);
            s.chain.store.state_get(&key)
                .and_then(|v| serde_json::from_slice::<serde_json::Value>(&v).ok())
        })
        .map(|mut t| { t.as_object_mut().map(|o| o.remove("token_hash")); t })
        .collect();
    (StatusCode::OK, Json(serde_json::json!({ "tokens": tokens, "count": tokens.len() }))).into_response()
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn linkgit_api_router(_state: AppState) -> Router<AppState> {
    Router::new()
        // Repos
        .route("/git/api/repos/:owner",            get(api_list_repos).post(api_create_repo))
        .route("/git/api/repos/:owner/:repo",      get(api_get_repo)
                                                       .patch(api_update_repo)
                                                       .delete(api_delete_repo))
        // Issues
        .route("/git/api/repos/:owner/:repo/issues",
               get(api_list_issues).post(api_create_issue))
        .route("/git/api/repos/:owner/:repo/issues/:number",
               get(api_get_issue).patch(api_update_issue))
        // Hooks
        .route("/git/api/repos/:owner/:repo/hooks",
               get(api_list_hooks).post(api_create_hook))
        .route("/git/api/repos/:owner/:repo/hooks/:id",
               axum::routing::delete(api_delete_hook))
        // Tokens
        .route("/git/api/tokens",    get(api_list_tokens).post(api_create_token))
        .route("/git/api/tokens/:token_id", axum::routing::delete(api_revoke_token))
}
