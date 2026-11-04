//! Peer self-identification (not consensus; never touches chain state).
//!
//! Every hone peer puts a signed identity in its libp2p identify `agent_version`:
//!   `hone-id/1 <base64(json)>` with json = {peer_id, account, kind, roles, version,
//!   commit, platform, sig}. `sig` is ed25519 (hex) over
//!   `hone-peer-id/1|{peer_id}|{account}|{kind}|{roles comma-joined}|{version}|{commit}|{platform}`
//! by the account's registered clock key or posting key, or for a device seat
//! `owner@<pubkey-hex>` by that seat key. The peer_id is signed, so an identity cannot be
//! replayed by another peer. Receivers verify it against chain state, log each new peer
//! once, and list every connected peer at GET /api/net/peers (unidentified peers are
//! listed as unverified, not blocked).
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use ed25519_dalek::{Signature, VerifyingKey};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

pub const PREFIX: &str = "hone-id/1 ";

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Ident {
    pub peer_id: String,
    pub account: String,
    pub kind: String,
    pub roles: Vec<String>,
    pub version: String,
    pub commit: String,
    pub platform: String,
    #[serde(default)]
    pub sig: String,
}

impl Ident {
    pub fn message(&self) -> String {
        format!("hone-peer-id/1|{}|{}|{}|{}|{}|{}|{}", self.peer_id, self.account, self.kind,
            self.roles.join(","), self.version, self.commit, self.platform)
    }
}

/// Our agent_version string. Unsigned (no posting key loaded) falls back to a plain
/// version string, which peers list as unverified.
pub fn agent_version(peer_id: &str, account: &str, roles: Vec<String>) -> String {
    let mut id = Ident {
        peer_id: peer_id.to_owned(), account: account.to_owned(), kind: "node".into(), roles,
        version: env!("CARGO_PKG_VERSION").into(),
        commit: option_env!("HONE_GIT_COMMIT").unwrap_or("").into(),
        platform: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        sig: String::new(),
    };
    match crate::local_submit::sign_bytes(id.message().as_bytes()) {
        Some(s) => {
            id.sig = s;
            format!("{PREFIX}{}", B64.encode(serde_json::to_vec(&id).unwrap_or_default()))
        }
        None => format!("hone-node/{}", env!("CARGO_PKG_VERSION")),
    }
}

/// Parse and verify a peer's agent_version. `keys_for(account)` returns the public keys
/// (hex) allowed to speak for `account`.
pub fn verify(peer_id: &str, agent: &str, keys_for: impl Fn(&str) -> Vec<String>) -> Option<(Ident, bool)> {
    let raw = agent.strip_prefix(PREFIX)?;
    let id: Ident = serde_json::from_slice(&B64.decode(raw.trim()).ok()?).ok()?;
    if id.peer_id != peer_id { return Some((id, false)); }
    let Ok(sig) = hex::decode(&id.sig).ok().and_then(|b| <[u8; 64]>::try_from(b).ok()).ok_or(()) else {
        return Some((id, false));
    };
    let mut keys = keys_for(&id.account);
    if let Some((_, pk)) = hone_types::parse_device_clock_id(&id.account) { keys.push(pk.to_owned()); }
    let msg = id.message();
    let ok = keys.iter().any(|pk| hex::decode(pk).ok()
        .and_then(|b| <[u8; 32]>::try_from(b).ok())
        .and_then(|a| VerifyingKey::from_bytes(&a).ok())
        .map(|vk| vk.verify_strict(msg.as_bytes(), &Signature::from_bytes(&sig)).is_ok())
        .unwrap_or(false));
    Some((id, ok))
}

/// Keys that may speak for `account` per chain state: registered clock key, posting key.
pub fn chain_keys(store: &crate::store::Store, account: &str) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(pk) = store.state_get(&format!("clock_reg:{}", account))
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| v["pubkey"].as_str().map(str::to_owned)) { out.push(pk); }
    if let Ok(Some(a)) = store.get_account(account) {
        if let Some(pk) = a["keys"]["posting"].as_str() { out.push(pk.to_owned()); }
    }
    out
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct PeerRow {
    pub peer_id: String,
    pub address: String,
    pub account: Option<String>,
    pub kind: Option<String>,
    pub roles: Vec<String>,
    pub version: Option<String>,
    pub commit: Option<String>,
    pub platform: Option<String>,
    pub agent: Option<String>,
    pub verified: bool,
}

fn table() -> &'static Mutex<HashMap<String, PeerRow>> {
    static T: OnceLock<Mutex<HashMap<String, PeerRow>>> = OnceLock::new();
    T.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn connected(peer_id: &str, address: &str) {
    let mut t = table().lock().unwrap();
    let r = t.entry(peer_id.to_owned()).or_default();
    r.peer_id = peer_id.to_owned();
    if r.address.is_empty() { r.address = address.to_owned(); }
}

pub fn disconnected(peer_id: &str) { table().lock().unwrap().remove(peer_id); }

/// Record an identify result. Returns the log line the first time a peer is identified.
pub fn identified(peer_id: &str, agent: &str, v: Option<(Ident, bool)>) -> Option<String> {
    let mut t = table().lock().unwrap();
    let r = t.entry(peer_id.to_owned()).or_default();
    r.peer_id = peer_id.to_owned();
    let first = r.agent.is_none();
    r.agent = Some(agent.to_owned());
    if let Some((id, ok)) = v {
        r.account = Some(id.account); r.kind = Some(id.kind); r.roles = id.roles;
        r.version = Some(id.version); r.commit = Some(id.commit); r.platform = Some(id.platform);
        r.verified = ok;
    }
    if !first { return None; }
    let short = if peer_id.len() > 16 { format!("{}…{}", &peer_id[..8], &peer_id[peer_id.len() - 4..]) } else { peer_id.to_owned() };
    Some(match (&r.account, r.verified) {
        (Some(a), true) => format!("[peer] {short} = {a} {} v{} {} {}", r.kind.clone().unwrap_or_default(),
            r.version.clone().unwrap_or_default(), r.platform.clone().unwrap_or_default(), r.address),
        (Some(a), false) => format!("[peer] {short} claims {a} (UNVERIFIED signature) {}", r.address),
        (None, _) => format!("[peer] {short} unidentified (agent '{agent}') {}", r.address),
    })
}

pub fn list() -> Vec<PeerRow> {
    let mut v: Vec<PeerRow> = table().lock().unwrap().values().cloned().collect();
    v.sort_by(|a, b| a.peer_id.cmp(&b.peer_id));
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn signed(sk: &SigningKey, peer: &str, account: &str) -> String {
        let mut id = Ident { peer_id: peer.into(), account: account.into(), kind: "phone".into(),
            roles: vec!["clock".into()], version: "1.0".into(), commit: "abc".into(), platform: "android".into(), sig: String::new() };
        id.sig = hex::encode(sk.sign(id.message().as_bytes()).to_bytes());
        format!("{PREFIX}{}", B64.encode(serde_json::to_vec(&id).unwrap()))
    }

    #[test]
    fn identity_verifies_and_rejects_spoofs() {
        let sk = SigningKey::from_bytes(&[7; 32]);
        let pk = hex::encode(sk.verifying_key().to_bytes());
        let seat = format!("josh@{pk}");
        // A device seat signs with its own seat key: no chain lookup needed.
        let a = signed(&sk, "12D3KooWA", &seat);
        assert!(verify("12D3KooWA", &a, |_| vec![]).unwrap().1);
        // Same identity presented by another peer id: the signed peer_id does not match.
        assert!(!verify("12D3KooWB", &a, |_| vec![]).unwrap().1);
        // A plain account verifies against its chain keys only.
        let n = signed(&sk, "12D3KooWC", "natoshisakamoto");
        assert!(verify("12D3KooWC", &n, |_| vec![pk.clone()]).unwrap().1);
        let other = hex::encode(SigningKey::from_bytes(&[8; 32]).verifying_key().to_bytes());
        assert!(!verify("12D3KooWC", &n, |_| vec![other.clone()]).unwrap().1);
        // Tampered field breaks the signature.
        let mut id: Ident = serde_json::from_slice(&B64.decode(n.strip_prefix(PREFIX).unwrap()).unwrap()).unwrap();
        id.kind = "decide".into();
        let t = format!("{PREFIX}{}", B64.encode(serde_json::to_vec(&id).unwrap()));
        assert!(!verify("12D3KooWC", &t, |_| vec![pk.clone()]).unwrap().1);
        // Not a hone identity at all.
        assert!(verify("12D3KooWD", "rust-libp2p/0.45", |_| vec![]).is_none());
        // Table: logs once, lists unverified peers too.
        connected("12D3KooWE", "/ip4/192.168.68.52/tcp/6943");
        let line = identified("12D3KooWE", "x", None).unwrap();
        assert!(line.contains("unidentified"));
        assert!(identified("12D3KooWE", "x", None).is_none());
        assert!(list().iter().any(|r| r.peer_id == "12D3KooWE" && !r.verified));
        disconnected("12D3KooWE");
    }
}
