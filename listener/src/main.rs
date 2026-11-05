#![cfg_attr(all(windows, feature = "tray"), windows_subsystem = "windows")]
//! HONE Birds camera listener.
//!   hone-listener            run (tray on Windows/macOS builds, headless elsewhere)
//!   hone-listener run        headless service (systemd)
//!   hone-listener open       open the setup/status page in the browser
//!   hone-listener url [--lan] print the page address (with its access token)
//!   hone-listener status     one-line status from the running listener
//!   hone-listener analyze <file> [rate]  offline check of a recording (no network, nothing stored)
//!   hone-listener apply-update   (root, Linux) install a verified staged .deb

mod account;
mod audio;
mod canonical;
mod config;
mod discovery;
mod live;
mod mic;
mod perch;
mod pipeline;
mod relay;
mod report;
mod rtsp;
mod secrets;
mod setup;
mod status;
mod submit;
#[cfg(all(feature = "tray", any(windows, target_os = "macos")))]
mod tray;
mod update;
mod voicegate;
mod voiceprint;
mod voucher;
mod web;

use config::{state_dir, Config, DEFAULT_PORT};
use std::sync::mpsc;

fn preferred_port() -> u16 {
    std::env::var("HONE_LISTENER_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(DEFAULT_PORT)
}

/// The port the running listener chose (saved at start), else the preferred one.
fn port() -> u16 {
    std::fs::read_to_string(state_dir().join("ui-port")).ok().and_then(|p| p.trim().parse().ok()).unwrap_or_else(preferred_port)
}

/// Is THIS install's listener already answering on this port? (Another install, e.g. one in
/// WSL whose localhost is forwarded, must not count.)
fn listener_at(port: u16) -> bool {
    ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(2)).build()
        .get(&format!("http://127.0.0.1:{port}/status")).call().ok()
        .and_then(|r| r.into_json::<serde_json::Value>().ok())
        .map(|j| j["name"] == "hone-listener" && j["instance"].as_str() == Some(status::instance_id().as_str())).unwrap_or(false)
}

fn token_from_disk() -> Option<String> {
    std::fs::read_to_string(state_dir().join("ui-token")).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn page_url(lan: bool) -> Option<String> {
    let t = token_from_disk().or_else(|| secrets::get("ui_token"))?;
    let host = if lan { discovery::local_ip().map(|i| i.to_string()).unwrap_or("127.0.0.1".into()) } else { "127.0.0.1".into() };
    Some(web::url(port(), &t, &host))
}

fn run(tray: bool) {
    let _ = std::fs::create_dir_all(state_dir());
    let want = preferred_port();
    // Windows (Hyper-V/WSL/Docker) reserves whole port ranges, so a failed bind does not mean
    // another listener runs: only a /status that answers as hone-listener means that.
    let mut bound = None;
    for p in (want..want + 20).chain(std::iter::once(0)) {
        match web::bind(p) {
            Ok(s) => {
                let real = s.server_addr().to_ip().map(|a| a.port()).unwrap_or(p);
                bound = Some((s, real));
                break;
            }
            Err(_) if p == want && listener_at(want) => {
                if let Some(u) = page_url(false) {
                    let _ = open::that(u);
                }
                return;
            }
            Err(_) => continue,
        }
    }
    let Some((server, port_now)) = bound else {
        log::error!("could not open a local port for the setup page");
        std::process::exit(1);
    };
    // The port in use, for `hone-listener open/url` and the HONE MCP (version.json local.status_port_file).
    let _ = std::fs::write(state_dir().join("ui-port"), port_now.to_string());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(state_dir().join("ui-port"), std::fs::Permissions::from_mode(0o644));
    }
    if let Err(e) = secrets::station_key() {
        log::error!("station key: {e}");
    }
    let cfg = Config::load();
    voucher::import_from_disk();
    let (tx, rx) = mpsc::channel();
    let engine = pipeline::Engine::start(tx);
    submit::spawn(rx);
    update::spawn();
    status::spawn();
    let first_run = !cfg.ready();
    voiceprint::configure(cfg.cell9.as_deref(), cfg.tz_min.unwrap_or_else(|| cfg.lon_hint.map(voiceprint::tz_from_lon).unwrap_or(0)));
    // Save voiceprints on SIGTERM/SIGINT (systemd stop, launchd, Ctrl-C) and every 10 min.
    let _ = ctrlc::set_handler(|| {
        voiceprint::save_now();
        std::process::exit(0);
    });
    std::thread::spawn(|| loop {
        std::thread::sleep(std::time::Duration::from_secs(600));
        voiceprint::save_now();
    });
    engine.sync(&cfg);
    let app = web::App::new(engine, cfg, port_now);
    web::serve(app.clone(), server);
    log::info!("Ornigo (HONE listener) {} on http://127.0.0.1:{} (state {})", env!("CARGO_PKG_VERSION"), port_now, state_dir().display());
    let url = web::url(app.port, &app.token, "127.0.0.1");
    if first_run && std::env::var("HONE_LISTENER_NO_BROWSER").is_err() && (tray || std::env::var("DISPLAY").is_ok()) {
        let _ = open::that(&url);
    }
    // refresh the account profile (verified flag, referrer) now and then
    {
        let app = app.clone();
        std::thread::spawn(move || loop {
            let mut c = app.cfg.lock().unwrap().clone();
            if c.account.is_some() && account::me(&mut c).is_ok() {
                app.update_cfg(|x| x.account = c.account.clone());
            }
            std::thread::sleep(std::time::Duration::from_secs(6 * 3600));
        });
    }
    #[cfg(all(feature = "tray", any(windows, target_os = "macos")))]
    if tray {
        tray::run(app);
    }
    let _ = tray;
    loop {
        std::thread::park();
    }
}

fn main() {
    let mut lb = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"));
    // GUI builds have no console: keep a small log file next to the settings.
    if cfg!(all(windows, feature = "tray")) || std::env::var("HONE_LISTENER_LOG_FILE").is_ok() {
        let _ = std::fs::create_dir_all(state_dir());
        let p = state_dir().join("listener.log");
        if std::fs::metadata(&p).map(|m| m.len() > 4 << 20).unwrap_or(false) {
            let _ = std::fs::rename(&p, state_dir().join("listener.old.log"));
        }
        if let Ok(f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
            lb.target(env_logger::Target::Pipe(Box::new(f)));
        }
    }
    lb.init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(|s| s.as_str()) {
        None => run(cfg!(all(feature = "tray", any(windows, target_os = "macos")))),
        Some("tray") => run(true),
        Some("run") => run(false),
        Some("open") => match page_url(false) {
            Some(u) => {
                if open::that(&u).is_err() {
                    println!("{u}");
                }
            }
            None => eprintln!("The listener has not started yet."),
        },
        Some("url") => match page_url(args.iter().any(|a| a == "--lan")) {
            Some(u) => println!("{u}"),
            None => eprintln!("The listener has not started yet."),
        },
        Some("status") => {
            let c = Config::load();
            println!("{} · {} camera(s) · signed in: {}", if c.listening { "listening" } else { "paused" }, c.stations.len(), c.account.is_some());
        }
        Some("analyze") => {
            let Some(f) = args.get(1) else {
                eprintln!("analyze <audio file> [rate, default 8000]");
                std::process::exit(2)
            };
            let rate = args.get(2).and_then(|r| r.parse().ok()).unwrap_or(8000);
            match pipeline::analyze_file(f, rate) {
                Ok(p) => println!("{}", serde_json::to_string_pretty(&p).unwrap()),
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1)
                }
            }
        }
        Some("verify-release") => {
            // verify-release <version.json> <version.json.sig> [artifact...]: the updater's own checks
            let (Some(m), Some(sg)) = (args.get(1), args.get(2)) else {
                eprintln!("verify-release <version.json> <version.json.sig> [files...]");
                std::process::exit(2)
            };
            let man = std::fs::read(m).unwrap_or_default();
            let ok = update::verify_manifest(&man, &std::fs::read(sg).unwrap_or_default());
            println!("signature by release key {}…: {}", &update::RELEASE_PUB_HEX[..16], if ok { "valid" } else { "INVALID" });
            let v: serde_json::Value = serde_json::from_slice(&man).unwrap_or_default();
            let mut bad = !ok;
            for f in &args[3..] {
                let name = std::path::Path::new(f).file_name().unwrap().to_string_lossy().to_string();
                let want = v["files"][&name]["sha256"].as_str().unwrap_or("");
                let got = perch::sha256_file(std::path::Path::new(f)).unwrap_or_default();
                println!("{name}: {}", if want == got { "sha256 matches" } else { "MISMATCH" });
                bad |= want != got;
            }
            std::process::exit(if bad { 1 } else { 0 });
        }
        Some("version") | Some("--version") => println!("hone-listener {}", env!("CARGO_PKG_VERSION")),
        Some("apply-update") => match update::apply_staged_deb() {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1)
            }
        },
        Some(x) => {
            eprintln!("unknown command {x}");
            std::process::exit(2)
        }
    }
}
