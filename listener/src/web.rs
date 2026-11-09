//! Local setup and status page. Listens on 127.0.0.1 only (or the LAN when
//! HONE_LISTENER_UI=lan, for a headless Pi) and requires the per-install token, which the app
//! puts in the URL it opens; the page then keeps it in a SameSite=Strict cookie. Requests
//! with a foreign Host header are refused (DNS-rebinding guard); writes need X-Hone: 1.

use crate::config::{cells, now_ms, state_dir, Config, Station};
use crate::discovery::{self, FoundCamera};
use crate::pipeline::Engine;
use crate::{account, canonical, live, secrets, setup, submit};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Read;
use std::sync::{Arc, Mutex};
use tiny_http::{Header, Method, Request, Response, Server};

pub const DISCLAIMER: &str = "Human conversation is automatically deleted. The listener only keeps a count of voices it removed. No audio is ever saved or sent; only a short bird report every 15 minutes.";
const INDEX: &str = include_str!("../ui/index.html");
/// Bundled assets: fonts (SIL Open Font License: Atkinson Hyperlegible, Bricolage Grotesque,
/// JetBrains Mono) and the sample dawn (hawfinch by Alexander Kurthy, CC BY-SA 4.0, Wikimedia Commons).
const ASSETS: &[(&str, &str, &[u8])] = &[
    ("atkinson-400.woff2", "font/woff2", include_bytes!("../ui/assets/atkinson-400.woff2")),
    ("atkinson-700.woff2", "font/woff2", include_bytes!("../ui/assets/atkinson-700.woff2")),
    ("bricolage-800.woff2", "font/woff2", include_bytes!("../ui/assets/bricolage-800.woff2")),
    ("jetbrains.woff2", "font/woff2", include_bytes!("../ui/assets/jetbrains.woff2")),
    ("sample-dawn.ogg", "audio/ogg", include_bytes!("../ui/assets/sample-dawn.ogg")),
];
pub const PHONE_URL: &str = "https://honemesh.net/me";

pub fn quality(tier: &str) -> &'static str {
    match tier {
        "A" => "Great",
        "B" => "Good",
        _ => "Basic",
    }
}

fn qr_svg() -> String {
    qrcode::QrCode::new(PHONE_URL.as_bytes()).map(|q| q.render::<qrcode::render::svg::Color>().min_dimensions(160, 160)
        .dark_color(qrcode::render::svg::Color("#0B1310")).light_color(qrcode::render::svg::Color("#F2F6EE")).build()).unwrap_or_default()
}

/// Place search and reverse lookup (OpenStreetMap Nominatim), so nobody types coordinates.
/// Only the search text or the point the user chose is sent; results are towns, not addresses.
fn nominatim(path: &str) -> Result<Value, String> {
    crate::account::agent().get(&format!("https://nominatim.openstreetmap.org/{path}")).set("Accept-Language", "en")
        .call().map_err(|_| "Place search isn't reachable right now. Try again.".to_string())?.into_json().map_err(|_| "Place search failed.".into())
}

fn town_of(a: &Value) -> Option<String> {
    for k in ["town", "city", "village", "suburb", "municipality", "hamlet", "county", "state"] {
        if let Some(s) = a[k].as_str() {
            return Some(s.to_string());
        }
    }
    None
}
const NAMES_FILE: &str = "names_en.json";

pub struct App {
    pub engine: Arc<Engine>,
    pub cfg: Mutex<Config>,
    pub token: String,
    pub port: u16,
    probes: Mutex<HashMap<String, (setup::ProbeResult, String, String)>>,
    names: HashMap<String, String>,
}

pub fn ui_token() -> String {
    if let Some(t) = secrets::get("ui_token") {
        return t;
    }
    let t = hex::encode(rand::random::<[u8; 16]>());
    let _ = secrets::set("ui_token", &t);
    // Readable copy for `hone-listener open` (same user; Linux: hone-listener group).
    let p = state_dir().join("ui-token");
    let _ = std::fs::write(&p, &t);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o640));
    }
    t
}

pub fn url(port: u16, token: &str, host: &str) -> String {
    format!("http://{host}:{port}/?t={token}")
}

fn j(v: Value) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_string(v.to_string()).with_header(Header::from_bytes("Content-Type", "application/json").unwrap()).with_header(Header::from_bytes("Cache-Control", "no-store").unwrap())
}

fn fail(code: u16, msg: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    j(json!({"error": msg})).with_status_code(code)
}

impl App {
    pub fn new(engine: Arc<Engine>, cfg: Config, port: u16) -> Arc<Self> {
        let names = std::fs::read_to_string(crate::config::res_dir().join(NAMES_FILE)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Arc::new(Self { engine, cfg: Mutex::new(cfg), token: ui_token(), port, probes: Mutex::new(HashMap::new()), names })
    }

    pub fn update_cfg<R>(&self, f: impl FnOnce(&mut Config) -> R) -> R {
        let mut c = self.cfg.lock().unwrap();
        let r = f(&mut c);
        let _ = c.save();
        self.engine.sync(&c);
        crate::voiceprint::configure(c.cell9.as_deref(), c.tz_min.unwrap_or_else(|| c.lon_hint.map(crate::voiceprint::tz_from_lon).unwrap_or(0)));
        let snap = c.clone();
        drop(c);
        if snap.account.is_some() {
            std::thread::spawn(move || {
                let _ = crate::status::send(&snap);
            });
        }
        r
    }

    fn state(&self) -> Value {
        let c = self.cfg.lock().unwrap().clone();
        let l = live::snapshot();
        let key_pub = secrets::station_key().map(|k| canonical::pub_hex(&k)).unwrap_or_default();
        let recent: Vec<Value> = l.recent.iter().take(30).map(|d| json!({"t": d.t, "name": d.name, "common": self.names.get(&d.name), "conf": d.conf})).collect();
        let mut out = json!({
            "version": env!("CARGO_PKG_VERSION"),
            "account": c.account, "pending_ref": c.pending_ref, "referral_ack": c.referral_ack,
            "consent_text": account::consent_text(c.account.as_ref().and_then(|a| a.referrer()).or(c.pending_ref.as_deref())),
            "disclaimer": DISCLAIMER, "disclaimer_ack": c.disclaimer_ack_ms.is_some(),
            "stations": c.stations.iter().map(|s| json!({"id": s.id, "kind": if s.kind == "mic" { "mic" } else { "camera" }, "label": s.label, "host": s.host, "codec": s.codec, "rate": s.rate, "tier": s.tier,
                "tier_copy": setup::tier_copy(&s.tier), "enabled": s.enabled, "live": l.stations.get(&s.id)})).collect::<Vec<_>>(),
            "located": c.cell9.is_some(), "listening": c.listening, "ready": c.ready(),
            "recent": recent, "reports_made": l.reports_made, "last_report_ms": l.last_report_ms, "last_report_bytes": l.last_report_bytes,
            "map_state": l.map_state, "chain_state": l.chain_state, "chain_queued": l.chain_queued, "chain_accepted": l.chain_accepted,
            "update_state": l.update_state, "model_error": l.model_error, "secret_store": secrets::backend(),
            "station_pub": key_pub, "summary": live::summary(c.listening),
            "voucher_state": crate::voucher::state(),
            "individuals": crate::voiceprint::view(&|sp| self.names.get(sp).cloned()),
        });
        let names: std::collections::HashMap<&str, &str> = c.stations.iter().map(|s| (s.id.as_str(), s.label.as_str())).collect();
        out["town"] = json!(c.town);
        out["has_account"] = json!(c.account.is_some());
        out["phone_url"] = json!(PHONE_URL);
        out["sources"] = json!(c.stations.iter().map(|s| {
            let x = l.stations.get(&s.id).cloned().unwrap_or_default();
            let bird = x.last_bird.as_ref().filter(|(_, t)| now_ms() - t < 90_000).map(|(n, _)| self.names.get(n).cloned().unwrap_or(n.clone()));
            let details = if s.kind == "mic" { format!("Microphone · {} kHz", s.rate / 1000) } else {
                format!("{} · {} kHz{}", s.codec, s.rate / 1000, s.noise_db.map(|n| format!(" · background {} dB", n.round())).unwrap_or_default()) };
            json!({"id": s.id, "kind": if s.kind == "mic" { "mic" } else { "camera" }, "label": s.label, "quality": quality(&s.tier),
                "details": details, "running": x.running, "error": x.last_error, "levels": x.levels, "bird": bird})
        }).collect::<Vec<_>>());
        let mut seen = std::collections::HashSet::new();
        out["singing_now"] = json!(l.recent.iter().filter(|d| now_ms() - d.t < 90_000).filter(|d| seen.insert(d.name.clone()))
            .map(|d| json!({"name": d.name, "common": self.names.get(&d.name), "source": names.get(d.station.as_str()), "t": d.t})).collect::<Vec<_>>());
        let day0 = now_ms() / 86_400_000 * 86_400_000;
        let week = now_ms().saturating_sub(7 * 86_400_000);
        let life = live::lifelist().lock().unwrap().clone();
        let mut seen = std::collections::HashSet::new();
        let list: Vec<Value> = l.recent.iter().filter(|d| d.t >= day0).filter(|d| seen.insert(d.name.clone()))
            .map(|d| json!({"name": d.name, "common": self.names.get(&d.name), "t": d.t, "new": life.get(&d.name).map(|f| *f >= week).unwrap_or(false)})).collect();
        out["today"] = json!({"list": list, "kinds": seen.len(), "new_week": life.values().filter(|f| **f >= week).count(),
            "voices": if l.voices_day.0 == now_ms() / 86_400_000 { l.voices_day.1 } else { 0 }, "life": life.len()});
        out
    }

    fn body(req: &mut Request) -> Value {
        let mut s = String::new();
        let _ = req.as_reader().take(64 * 1024).read_to_string(&mut s);
        serde_json::from_str(&s).unwrap_or(json!({}))
    }

    fn api(&self, path: &str, b: Value) -> Response<std::io::Cursor<Vec<u8>>> {
        let s = |k: &str| b[k].as_str().unwrap_or("").to_string();
        match path {
            "/api/state" => j(self.state()),
            "/api/ref" => {
                let c = self.cfg.lock().unwrap().clone();
                match account::ref_check(&c, &s("code")) {
                    Ok(ok) => j(json!({"ok": ok, "consent_text": account::consent_text(ok.then(|| s("code").to_lowercase()).as_deref())})),
                    Err(e) => fail(400, &e),
                }
            }
            "/api/signup" => {
                let mut c = self.cfg.lock().unwrap().clone();
                if b["consent"].as_bool() != Some(true) {
                    return fail(400, "Please tick the box to continue.");
                }
                let rf = s("ref");
                match account::signup(&mut c, &s("name"), &s("email"), &s("password"), Some(&rf), true) {
                    Ok(()) => {
                        self.update_cfg(|x| *x = c);
                        j(self.state())
                    }
                    Err(e) => fail(400, &e),
                }
            }
            "/api/login" => {
                let mut c = self.cfg.lock().unwrap().clone();
                match account::login(&mut c, &s("email"), &s("password")) {
                    Ok(()) => {
                        self.update_cfg(|x| *x = c);
                        j(self.state())
                    }
                    Err(e) => fail(400, &e),
                }
            }
            "/api/verify" => {
                let mut c = self.cfg.lock().unwrap().clone();
                match account::verify(&mut c, &s("code")) {
                    Ok(()) => {
                        self.update_cfg(|x| *x = c);
                        j(self.state())
                    }
                    Err(e) => fail(400, &e),
                }
            }
            "/api/resend" => {
                let c = self.cfg.lock().unwrap().clone();
                match account::resend(&c) {
                    Ok(()) => j(json!({"ok": true})),
                    Err(e) => fail(400, &e),
                }
            }
            "/api/referral_ack" => {
                if b["ok"].as_bool() != Some(true) {
                    return fail(400, "Please tick the box to continue.");
                }
                self.update_cfg(|c| c.referral_ack = true);
                j(self.state())
            }
            "/api/discover" => {
                let extra: Vec<std::net::Ipv4Addr> = s("host").parse().ok().into_iter().collect();
                if !s("host").is_empty() && !discovery::is_local_host(&s("host")) {
                    return fail(400, "Only cameras on your own Wi-Fi can be added (use its 192.168.x.x or 10.x.x.x address).");
                }
                let (me, cams) = discovery::discover(&extra);
                let mics = crate::mic::list();
                j(json!({"local_ip": me.map(|i| i.to_string()), "cameras": cams, "mics": mics}))
            }
            "/api/probe" => {
                let cam = FoundCamera { host: s("host"), rtsp_port: b["rtsp_port"].as_u64().map(|p| p as u16), onvif: b["onvif"].as_str().map(String::from), name: b["name"].as_str().map(String::from) };
                match setup::connect(&cam, &s("user"), &s("pass"), 6.0) {
                    Ok(p) => {
                        let id = hex::encode(rand::random::<[u8; 8]>());
                        let out = json!({"probe_id": id, "probe": p, "tier_copy": setup::tier_copy(&p.tier),
                            "privacy": "We only use sound, never video. Anyone talking is deleted instantly."});
                        self.probes.lock().unwrap().insert(id, (p, s("user"), s("pass")));
                        j(out)
                    }
                    Err(e) => fail(400, setup::friendly(&e)),
                }
            }
            "/api/location" => {
                let (Some(lat), Some(lon)) = (b["lat"].as_f64(), b["lon"].as_f64()) else { return fail(400, "Please choose your town.") };
                let Some((c9, c7)) = cells(lat, lon) else { return fail(400, "That place doesn't look right.") };
                let town = b["town"].as_str().map(|t| t.chars().take(60).collect::<String>()).filter(|t| !t.is_empty())
                    .or_else(|| nominatim(&format!("reverse?format=jsonv2&zoom=12&lat={lat:.3}&lon={lon:.3}")).ok().and_then(|v| town_of(&v["address"])));
                self.update_cfg(|c| {
                    c.cell9 = Some(c9);
                    c.cell7 = Some(c7);
                    c.lon_hint = Some(lon);
                    c.town = town;
                });
                j(self.state())
            }
            "/api/place/search" => {
                let q: String = s("q").chars().take(80).collect();
                if q.trim().len() < 2 {
                    return j(json!({"places": []}));
                }
                let enc = percent_encoding::utf8_percent_encode(q.trim(), percent_encoding::NON_ALPHANUMERIC);
                match nominatim(&format!("search?format=jsonv2&addressdetails=1&limit=6&q={enc}")) {
                    Ok(v) => j(json!({"places": v.as_array().cloned().unwrap_or_default().iter().filter_map(|p| {
                        let lat: f64 = p["lat"].as_str()?.parse().ok()?;
                        let lon: f64 = p["lon"].as_str()?.parse().ok()?;
                        let name = town_of(&p["address"]).or(p["name"].as_str().map(String::from))?;
                        let region = [p["address"]["state"].as_str(), p["address"]["country"].as_str()].into_iter().flatten().collect::<Vec<_>>().join(", ");
                        Some(json!({"town": name, "region": region, "lat": lat, "lon": lon}))
                    }).collect::<Vec<_>>()})),
                    Err(e) => fail(502, &e),
                }
            }
            "/api/camera/add" => {
                if self.cfg.lock().unwrap().cell9.is_none() {
                    return fail(400, "Please choose your town first.");
                }
                let cam = FoundCamera { host: s("host"), rtsp_port: b["rtsp_port"].as_u64().map(|p| p as u16), onvif: b["onvif"].as_str().map(String::from), name: b["name"].as_str().map(String::from) };
                let p = match setup::connect(&cam, &s("user"), &s("pass"), 6.0) {
                    Ok(p) => p,
                    Err(e) => return j(json!({"error": setup::friendly(&e), "need_password": e.code == 401})).with_status_code(400),
                };
                let key = match secrets::station_key() {
                    Ok(k) => k,
                    Err(e) => return fail(500, &e),
                };
                let id = setup::station_id(&p.id_source, &p.id_value, &canonical::pub_hex(&key));
                if let Err(e) = secrets::set_camera_creds(&id, &s("user"), &s("pass")) {
                    return fail(500, &e);
                }
                let label = if s("label").is_empty() { p.model.clone().unwrap_or("Camera".into()) } else { s("label") };
                let st = Station { id: id.clone(), kind: "rtsp".into(), label: label.chars().take(40).collect(), host: p.host.clone(), port: p.port, path: p.path.clone(), onvif: cam.onvif.clone(),
                    codec: p.codec.clone(), rate: p.rate, tier: p.tier.clone(), id_source: p.id_source.clone(), noise_db: p.noise_db, enabled: true, added_ms: now_ms() };
                self.update_cfg(|c| {
                    c.stations.retain(|x| x.id != id);
                    c.stations.push(st);
                });
                let mut out = self.state();
                out["added"] = json!({"label": label, "quality": quality(&p.tier), "tier_copy": setup::tier_copy(&p.tier)});
                j(out)
            }
            "/api/mic/add" => {
                let spec = s("spec");
                if !crate::mic::list().iter().any(|m| m.spec == spec) {
                    return fail(400, "That microphone isn't connected any more.");
                }
                if self.cfg.lock().unwrap().cell9.is_none() {
                    return fail(400, "Please set where you are first.");
                }
                let noise = match crate::mic::probe(&spec) {
                    Ok(n) => n,
                    Err(e) => return fail(400, &e),
                };
                let key = match secrets::station_key() {
                    Ok(k) => k,
                    Err(e) => return fail(500, &e),
                };
                let id = setup::station_id("mic", &spec, &canonical::pub_hex(&key));
                let label = if s("label").is_empty() { s("name") } else { s("label") };
                let st = Station { id: id.clone(), kind: "mic".into(), label: label.chars().take(40).collect(), host: String::new(), port: 0, path: spec,
                    onvif: None, codec: "mic".into(), rate: crate::mic::RATE, tier: crate::audio::tier(crate::mic::RATE).into(), id_source: "mic".into(),
                    noise_db: noise, enabled: true, added_ms: now_ms() };
                self.update_cfg(|c| {
                    c.stations.retain(|x| x.id != id);
                    c.stations.push(st);
                });
                let mut out = self.state();
                out["added"] = json!({"label": label, "tier": "A", "tier_copy": setup::tier_copy("A")});
                j(out)
            }
            "/api/station/add" => {
                let Some((p, u, pw)) = self.probes.lock().unwrap().remove(&s("probe_id")) else { return fail(400, "Please test the camera again.") };
                let loc = match (b["lat"].as_f64(), b["lon"].as_f64()) {
                    (Some(lat), Some(lon)) => match cells(lat, lon) {
                        Some((c9, c7)) => Some((c9, c7, lon)),
                        None => return fail(400, "That location doesn't look right."),
                    },
                    _ if self.cfg.lock().unwrap().cell9.is_some() => None,
                    _ => return fail(400, "Please set where the camera is."),
                };
                let key = match secrets::station_key() {
                    Ok(k) => k,
                    Err(e) => return fail(500, &e),
                };
                let id = setup::station_id(&p.id_source, &p.id_value, &canonical::pub_hex(&key));
                if let Err(e) = secrets::set_camera_creds(&id, &u, &pw) {
                    return fail(500, &e);
                }
                let label = if s("label").is_empty() { p.model.clone().unwrap_or("My camera".into()) } else { s("label") };
                let st = Station { id: id.clone(), kind: "rtsp".into(), label: label.chars().take(40).collect(), host: p.host, port: p.port, path: p.path, onvif: b["onvif"].as_str().map(String::from),
                    codec: p.codec, rate: p.rate, tier: p.tier, id_source: p.id_source, noise_db: p.noise_db, enabled: true, added_ms: now_ms() };
                self.update_cfg(|c| {
                    c.stations.retain(|x| x.id != id);
                    c.stations.push(st);
                    if let Some((c9, c7, lon)) = loc {
                        c.cell9 = Some(c9);
                        c.cell7 = Some(c7);
                        c.lon_hint = Some(lon);
                    }
                });
                j(self.state())
            }
            "/api/station/remove" => {
                let id = s("id");
                secrets::delete(&format!("cam:{id}"));
                self.update_cfg(|c| c.stations.retain(|x| x.id != id));
                j(self.state())
            }
            "/api/disclaimer" => {
                if b["ok"].as_bool() != Some(true) {
                    return fail(400, "Please tick the box to continue.");
                }
                self.update_cfg(|c| c.disclaimer_ack_ms = Some(now_ms()));
                j(self.state())
            }
            "/api/start" => {
                let c = self.cfg.lock().unwrap().clone();
                if !c.ready() {
                    return fail(400, "Finish the steps above first.");
                }
                self.update_cfg(|c| c.listening = true);
                j(self.state())
            }
            "/api/tz" => {
                match b["tz_min"].as_i64() {
                    Some(t) if (-720..=840).contains(&t) => {
                        self.update_cfg(|c| c.tz_min = Some(t as i32));
                        j(json!({"ok": true}))
                    }
                    _ => fail(400, "bad time zone"),
                }
            }
            "/api/bird/name" => match crate::voiceprint::name(&s("bird_id"), &s("name")) {
                Ok(()) => j(self.state()),
                Err(e) => fail(400, &e),
            },
            "/api/pause" => {
                self.update_cfg(|c| c.listening = false);
                j(self.state())
            }
            "/api/logout" => {
                let mut c = self.cfg.lock().unwrap().clone();
                account::logout(&mut c);
                c.listening = false;
                self.update_cfg(|x| *x = c);
                j(self.state())
            }
            "/api/update" => {
                let c = self.cfg.lock().unwrap().clone();
                let st = crate::update::check(&c, true);
                live::set(|l| l.update_state = Some(st.clone()));
                j(json!({"update_state": st}))
            }
            _ => fail(404, "not found"),
        }
    }

    fn host_ok(&self, req: &Request) -> bool {
        let host = req.headers().iter().find(|h| h.field.equiv("Host")).map(|h| h.value.as_str().to_string()).unwrap_or_default();
        let lan = std::env::var("HONE_LISTENER_UI").map(|v| v == "lan").unwrap_or(false);
        let (h, p) = host.rsplit_once(':').unwrap_or((host.as_str(), ""));
        if p != self.port.to_string() {
            return false;
        }
        h == "127.0.0.1" || h == "localhost" || (lan && discovery::is_local_host(h))
    }

    fn authed(&self, req: &Request) -> bool {
        req.headers().iter().filter(|h| h.field.equiv("Cookie")).flat_map(|h| h.value.as_str().split(';').map(|x| x.trim().to_string()).collect::<Vec<_>>())
            .any(|c| c == format!("hl={}", self.token))
    }

    pub fn handle(&self, mut req: Request) {
        if !self.host_ok(&req) {
            let _ = req.respond(fail(403, "forbidden"));
            return;
        }
        let url = req.url().to_string();
        let (path, query) = url.split_once('?').unwrap_or((&url, ""));
        if path == "/status" && *req.method() == Method::Get {
            let c = self.cfg.lock().unwrap().clone();
            let _ = req.respond(j(crate::status::local(&c)));
            return;
        }
        if let Some(name) = path.strip_prefix("/a/") {
            match ASSETS.iter().find(|(n, _, _)| *n == name) {
                Some((_, ct, b)) => {
                    let _ = req.respond(Response::from_data(b.to_vec()).with_header(Header::from_bytes("Content-Type", *ct).unwrap())
                        .with_header(Header::from_bytes("Cache-Control", "public, max-age=604800").unwrap()));
                }
                None => {
                    let _ = req.respond(fail(404, "not found"));
                }
            }
            return;
        }
        if path == "/qr.svg" {
            let _ = req.respond(Response::from_string(qr_svg()).with_header(Header::from_bytes("Content-Type", "image/svg+xml").unwrap()));
            return;
        }
        if path == "/" {
            if let Some(t) = query.split('&').find_map(|kv| kv.strip_prefix("t=")) {
                if t == self.token {
                    let cookie = format!("hl={}; HttpOnly; SameSite=Strict; Path=/; Max-Age=31536000", self.token);
                    let _ = req.respond(Response::empty(303).with_header(Header::from_bytes("Location", "/").unwrap()).with_header(Header::from_bytes("Set-Cookie", cookie).unwrap()));
                    return;
                }
            }
            if !self.authed(&req) {
                let _ = req.respond(Response::from_string("Open Ornigo from its menu (or run: hone-listener open).").with_status_code(403));
                return;
            }
            let _ = req.respond(Response::from_string(INDEX).with_header(Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap())
                .with_header(Header::from_bytes("Content-Security-Policy", "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; font-src 'self'; media-src 'self'; frame-ancestors 'none'").unwrap()));
            return;
        }
        if !path.starts_with("/api/") || !self.authed(&req) {
            let _ = req.respond(fail(403, "forbidden"));
            return;
        }
        let write = *req.method() == Method::Post;
        if write && !req.headers().iter().any(|h| h.field.equiv("X-Hone") && h.value.as_str() == "1") {
            let _ = req.respond(fail(403, "forbidden"));
            return;
        }
        let b = if write { Self::body(&mut req) } else { json!({}) };
        let r = self.api(path, b);
        let _ = req.respond(r);
    }
}

pub fn bind(port: u16) -> std::io::Result<Server> {
    let lan = std::env::var("HONE_LISTENER_UI").map(|v| v == "lan").unwrap_or(false);
    let addr = if lan { format!("0.0.0.0:{port}") } else { format!("127.0.0.1:{port}") };
    Server::http(addr).map_err(|e| std::io::Error::new(std::io::ErrorKind::AddrInUse, e.to_string()))
}

pub fn serve(app: Arc<App>, server: Server) {
    let server = Arc::new(server);
    for _ in 0..4 {
        let (s, a) = (server.clone(), app.clone());
        std::thread::spawn(move || {
            for req in s.incoming_requests() {
                a.handle(req);
            }
        });
    }
}

#[allow(dead_code)]
pub fn waiting_copy() -> &'static str {
    submit::WAITING
}
