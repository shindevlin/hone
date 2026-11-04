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
        json!({
            "version": env!("CARGO_PKG_VERSION"),
            "account": c.account, "pending_ref": c.pending_ref, "referral_ack": c.referral_ack,
            "consent_text": account::consent_text(c.account.as_ref().and_then(|a| a.referrer()).or(c.pending_ref.as_deref())),
            "disclaimer": DISCLAIMER, "disclaimer_ack": c.disclaimer_ack_ms.is_some(),
            "stations": c.stations.iter().map(|s| json!({"id": s.id, "label": s.label, "host": s.host, "codec": s.codec, "rate": s.rate, "tier": s.tier,
                "tier_copy": setup::tier_copy(&s.tier), "enabled": s.enabled, "live": l.stations.get(&s.id)})).collect::<Vec<_>>(),
            "located": c.cell9.is_some(), "listening": c.listening, "ready": c.ready(),
            "recent": recent, "reports_made": l.reports_made, "last_report_ms": l.last_report_ms, "last_report_bytes": l.last_report_bytes,
            "map_state": l.map_state, "chain_state": l.chain_state, "chain_queued": l.chain_queued, "chain_accepted": l.chain_accepted,
            "update_state": l.update_state, "model_error": l.model_error, "secret_store": secrets::backend(),
            "station_pub": key_pub, "summary": live::summary(c.listening),
        })
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
                j(json!({"local_ip": me.map(|i| i.to_string()), "cameras": cams}))
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
            "/api/station/add" => {
                let Some((p, u, pw)) = self.probes.lock().unwrap().remove(&s("probe_id")) else { return fail(400, "Please test the camera again.") };
                let (Some(lat), Some(lon)) = (b["lat"].as_f64(), b["lon"].as_f64()) else { return fail(400, "Please set where the camera is.") };
                let Some((c9, c7)) = cells(lat, lon) else { return fail(400, "That location doesn't look right.") };
                let key = match secrets::station_key() {
                    Ok(k) => k,
                    Err(e) => return fail(500, &e),
                };
                let id = setup::station_id(&p.id_source, &p.id_value, &canonical::pub_hex(&key));
                if let Err(e) = secrets::set_camera_creds(&id, &u, &pw) {
                    return fail(500, &e);
                }
                let label = if s("label").is_empty() { p.model.clone().unwrap_or("My camera".into()) } else { s("label") };
                let st = Station { id: id.clone(), label: label.chars().take(40).collect(), host: p.host, port: p.port, path: p.path, onvif: b["onvif"].as_str().map(String::from),
                    codec: p.codec, rate: p.rate, tier: p.tier, id_source: p.id_source, noise_db: p.noise_db, enabled: true, added_ms: now_ms() };
                self.update_cfg(|c| {
                    c.stations.retain(|x| x.id != id);
                    c.stations.push(st);
                    c.cell9 = Some(c9);
                    c.cell7 = Some(c7);
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
        if path == "/" {
            if let Some(t) = query.split('&').find_map(|kv| kv.strip_prefix("t=")) {
                if t == self.token {
                    let cookie = format!("hl={}; HttpOnly; SameSite=Strict; Path=/; Max-Age=31536000", self.token);
                    let _ = req.respond(Response::empty(303).with_header(Header::from_bytes("Location", "/").unwrap()).with_header(Header::from_bytes("Set-Cookie", cookie).unwrap()));
                    return;
                }
            }
            if !self.authed(&req) {
                let _ = req.respond(Response::from_string("Open the HONE listener from its menu (or run: hone-listener open).").with_status_code(403));
                return;
            }
            let _ = req.respond(Response::from_string(INDEX).with_header(Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap())
                .with_header(Header::from_bytes("Content-Security-Policy", "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'").unwrap()));
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
