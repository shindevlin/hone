//! Email sign-up / sign-in against the HONE indexer (/api/accounts/*), exactly as the Android
//! app does it (account/AccountStore.kt). The station key proves itself with a seat_link
//! envelope, which links this listener to the account so its bird reports count.

use crate::canonical;
use crate::config::{Account, Config};
use crate::secrets;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::time::Duration;

pub const DEVICE_LABEL: &str = "Ornigo listener";

pub fn consent_text(r#ref: Option<&str>) -> String {
    match r#ref {
        Some(r) => format!("Rewards from this app go to the person who invited you ({r}) until you create your own wallet."),
        None => "Rewards from this app go to the HONE community pool until you create your own wallet.".into(),
    }
}

pub fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(10)).timeout(Duration::from_secs(25)).user_agent(concat!("HONE-listener/", env!("CARGO_PKG_VERSION"))).build()
}

fn seat_proof(email: &str) -> Result<Value, String> {
    let k = secrets::station_key()?;
    let p = json!({
        "kind": "seat_link", "seat_pub": canonical::pub_hex(&k),
        "email_sha": hex::encode(Sha256::digest(email.trim().to_lowercase().as_bytes())),
        "label": DEVICE_LABEL, "ts": crate::config::now_ms() / 1000,
    });
    Ok(canonical::envelope(&canonical::encode(&p), "seat", &k))
}

fn call(cfg: &Config, method: &str, path: &str, body: Option<Value>, auth: bool) -> Result<Value, String> {
    let url = format!("{}/accounts{}", cfg.api, path);
    let mut req = agent().request(method, &url).set("Content-Type", "application/json");
    if auth {
        if let Some(t) = secrets::get("session") {
            req = req.set("Authorization", &format!("Bearer {t}"));
        }
    }
    let r = match body {
        Some(b) => req.send_string(&b.to_string()),
        None => req.call(),
    };
    match r {
        Ok(r) => r.into_json::<Value>().map_err(|_| "Something went wrong. Try again.".into()),
        Err(ureq::Error::Status(code, r)) => {
            let j: Value = r.into_json().unwrap_or(json!({}));
            Err(j["error"].as_str().map(String::from).unwrap_or(format!("Something went wrong ({code}). Try again.")))
        }
        Err(_) => Err("No connection. Check your internet and try again.".into()),
    }
}

fn save_session(cfg: &mut Config, j: &Value) -> Result<(), String> {
    if let Some(t) = j["token"].as_str().filter(|t| !t.is_empty()) {
        secrets::set("session", t)?;
    }
    save_profile(cfg, if j["user"].is_object() { &j["user"] } else { j });
    Ok(())
}

fn save_profile(cfg: &mut Config, u: &Value) {
    let mut a = cfg.account.clone().unwrap_or_default();
    if let Some(h) = u["handle"].as_str() {
        a.handle = h.into();
    }
    if let Some(e) = u["email"].as_str() {
        a.email = e.into();
    }
    if let Some(n) = u["name"].as_str() {
        a.name = n.into();
    }
    if let Some(r) = u["ref"].as_str() {
        a.r#ref = r.into();
    }
    if let Some(v) = u["verified"].as_bool() {
        a.verified = v;
    }
    if !a.handle.is_empty() {
        cfg.account = Some(a);
        cfg.pending_ref = None;
    }
}

pub fn ref_check(cfg: &Config, code: &str) -> Result<bool, String> {
    let c = code.trim().to_lowercase();
    if !crate::config::ref_ok(&c) {
        return Ok(false);
    }
    Ok(call(cfg, "GET", &format!("/ref/{c}"), None, false)?["ok"].as_bool().unwrap_or(false))
}

pub fn signup(cfg: &mut Config, name: &str, email: &str, password: &str, r#ref: Option<&str>, consent: bool,
              kind: &str, business: &Value) -> Result<(), String> {
    if kind != "personal" && kind != "business" {
        return Err("Please choose who this account is for: you, or a business.".into());
    }
    let r = r#ref.map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).unwrap_or_else(|| "none".into());
    let ct = consent_text(Some(&r).filter(|x| x.as_str() != "none").map(|s| s.as_str()));
    let mut b = json!({"name": name.trim(), "email": email.trim(), "password": password, "ref": r,
        "consent": consent, "consent_text": ct, "seat": seat_proof(email)?, "kind": kind});
    if kind == "business" {
        b["business"] = json!({"name": business["name"].as_str().unwrap_or("").trim(),
            "places": business["places"].as_array().cloned().unwrap_or_default()});
    }
    let j = call(cfg, "POST", "/signup", Some(b), false)?;
    save_session(cfg, &j)?;
    cfg.referral_ack = consent;
    Ok(())
}

pub fn login(cfg: &mut Config, email: &str, password: &str) -> Result<(), String> {
    let j = call(cfg, "POST", "/login", Some(json!({"email": email.trim(), "password": password, "seat": seat_proof(email)?})), false)?;
    save_session(cfg, &j)
}

pub fn verify(cfg: &mut Config, code: &str) -> Result<(), String> {
    let j = call(cfg, "POST", "/verify", Some(json!({"code": code.trim()})), true)?;
    save_profile(cfg, &j);
    Ok(())
}

pub fn resend(cfg: &Config) -> Result<(), String> {
    call(cfg, "POST", "/resend", Some(json!({})), true).map(|_| ())
}

pub fn me(cfg: &mut Config) -> Result<(), String> {
    let j = call(cfg, "GET", "/me", None, true)?;
    save_profile(cfg, &j);
    Ok(())
}

pub fn logout(cfg: &mut Config) {
    let _ = call(cfg, "POST", "/logout", Some(json!({})), true);
    secrets::delete("session");
    cfg.account = None::<Account>;
    cfg.referral_ack = false;
}

#[cfg(test)]
mod tests {
    #[test]
    fn consent_copy_matches_android() {
        assert_eq!(super::consent_text(Some("maria")), "Rewards from this app go to the person who invited you (maria) until you create your own wallet.");
    }
}
