mod caster;
mod chain;
mod config;
mod rtcm3;

use crate::config::Config;
use anyhow::Result;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::process::Command;
use tracing::{info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("btcpc_gnss_capture=info".parse()?)
                .add_directive("warn".parse()?),
        )
        .init();

    let config = Config::from_env();

    info!("BTCPC GNSS Capture v0.1.0");
    info!("device: {}  geodnet: {}:{}", config.device_ip, config.geodnet_ip, config.geodnet_port);
    info!("listen port: {}  miner: {}  chain interval: {} frames",
        config.listen_port, config.miner, config.chain_interval);
    if config.forward { info!("caster forwarding: enabled"); }

    let arp_procs: Arc<Mutex<Vec<tokio::process::Child>>> = Arc::new(Mutex::new(Vec::new()));

    // ARP spoof — runs continuously, restarts on exit
    if config.arp_spoof && !config.gateway_ip.is_empty() {
        info!("ARP spoof: {} ↔ {} on {}", config.device_ip, config.gateway_ip, config.iface);
        if let Err(e) = enable_ip_forward() { warn!("ip_forward: {e}"); }
        install_dnat(&config);
        spawn_arp_loop(config.iface.clone(), config.device_ip.clone(),
            config.gateway_ip.clone(), Arc::clone(&arp_procs));
    } else {
        install_dnat(&config);
    }

    // Spawn NTRIP caster forwarding tasks
    let caster_txs: Vec<caster::FrameSender> = config.casters.iter()
        .filter(|c| c.enabled)
        .map(|c| caster::spawn_caster(c.clone()))
        .collect();

    let listener = TcpListener::bind(("0.0.0.0", config.listen_port)).await?;
    info!("Fake GEODNET server listening on :{}", config.listen_port);

    // Clone config for cleanup task
    let cleanup_config = config.clone();
    let cleanup_procs = Arc::clone(&arp_procs);
    tokio::spawn(async move {
        shutdown_signal().await;
        info!("Shutting down — removing DNAT");
        remove_dnat(&cleanup_config);
        let mut procs = cleanup_procs.lock().unwrap();
        for p in procs.iter_mut() { let _ = p.start_kill(); }
        std::process::exit(0);
    });

    // Shared position lock: once a 1005 frame is seen, the ECEF position is stored.
    // Subsequent sessions must match within 1m (10000 units = 1m in 0.1mm units).
    let position_lock: Arc<Mutex<Option<(i64, i64, i64)>>> = Arc::new(Mutex::new(None));

    loop {
        let (socket, addr) = listener.accept().await?;
        let src_ip = addr.ip().to_string();

        // Reject connections that don't come from the configured device IP.
        // (The DNAT rule already limits this at the kernel level, but defence in depth.)
        if !config.device_ip.is_empty() && src_ip != config.device_ip {
            warn!("Rejected connection from {} — expected {}", src_ip, config.device_ip);
            continue;
        }

        info!("NTRIP connection from {}", addr);
        let cfg = config.clone();
        let txs = caster_txs.clone();
        let pos_lock = Arc::clone(&position_lock);
        tokio::spawn(handle_connection(socket, cfg, txs, pos_lock));
    }
}

/// Parse miner key from NTRIP SOURCE or Basic auth header.
///
/// NTRIP SOURCE protocol (used by Hyfix):
///   "SOURCE <password> <mountpoint>\r\nSource-Sign: <hex_sig>\r\n..."
///   password is the device serial number (miner key).
///
/// NTRIP CLIENT Basic auth (standard):
///   "Authorization: Basic <base64(user:pass)>"
///
/// Returns (miner_key, source_sign_hex).
fn parse_ntrip_miner_key(header: &[u8]) -> (Option<String>, Option<String>) {
    let text = match std::str::from_utf8(header) {
        Ok(t) => t,
        Err(_) => return (None, None),
    };

    let mut miner_key = None;
    let mut source_sign = None;

    for line in text.lines() {
        // NTRIP SOURCE line: "SOURCE <password> <mountpoint>"
        if line.starts_with("SOURCE ") {
            let parts: Vec<&str> = line.splitn(3, ' ').collect();
            if parts.len() >= 2 {
                miner_key = Some(parts[1].to_owned());
            }
        }
        // Hyfix cryptographic signature
        if line.to_lowercase().starts_with("source-sign:") {
            source_sign = Some(line["source-sign:".len()..].trim().to_owned());
        }
        // Standard Basic auth fallback
        let lower = line.to_lowercase();
        if lower.starts_with("authorization:") && miner_key.is_none() {
            let rest = line["authorization:".len()..].trim();
            if let Some(b64) = rest.strip_prefix("Basic ").or_else(|| rest.strip_prefix("basic ")) {
                use base64::Engine as _;
                if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(b64.trim()) {
                    if let Ok(creds) = String::from_utf8(decoded) {
                        miner_key = creds.splitn(2, ':').nth(1).map(str::to_owned);
                    }
                }
            }
        }
    }

    (miner_key, source_sign)
}

async fn handle_connection(
    mut socket: tokio::net::TcpStream,
    config: Config,
    caster_txs: Vec<caster::FrameSender>,
    position_lock: Arc<Mutex<Option<(i64, i64, i64)>>>,
) {
    let mut buf: Vec<u8> = Vec::new();
    let mut raw = [0u8; 8192];
    let mut ntrip_acked = false;
    let mut frame_count: u64 = 0;
    let mut total_bytes: u64 = 0;
    let mut recorder = chain::ChainRecorder::new(config.clone());

    loop {
        match socket.read(&mut raw).await {
            Ok(0) => { info!("NTRIP client disconnected"); break; }
            Ok(n) => {
                buf.extend_from_slice(&raw[..n]);

                if !ntrip_acked {
                    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let header = buf[..pos].to_vec();

                        // Log the raw NTRIP header (first line only) for protocol discovery.
                        if let Ok(text) = std::str::from_utf8(&header) {
                            let first_line = text.lines().next().unwrap_or("").trim();
                            info!("NTRIP header first line: {:?}", first_line);
                            for line in text.lines().skip(1) {
                                if !line.trim().is_empty() {
                                    info!("NTRIP header: {:?}", line.trim());
                                }
                            }
                        }

                        // Validate miner key + log Source-Sign if device_sn configured.
                        let (auth_key, sign_hex) = parse_ntrip_miner_key(&header);
                        if !config.device_sn.is_empty() {
                            match auth_key {
                                Some(ref key) if key == &config.device_sn => {
                                    if let Some(ref sig) = sign_hex {
                                        info!("NTRIP auth OK: SN={} Source-Sign={}…", key, &sig[..16.min(sig.len())]);
                                    } else {
                                        info!("NTRIP auth OK: SN={} (no Source-Sign)", key);
                                    }
                                }
                                Some(ref key) => {
                                    warn!("NTRIP auth REJECTED: key '{}' != expected '{}'", key, config.device_sn);
                                    let _ = socket.write_all(b"ERROR - Bad Password\r\n").await;
                                    break;
                                }
                                None => {
                                    warn!("NTRIP: no auth key in header — allowing (set BTCPC_GNSS_DEVICE_SN to enforce)");
                                }
                            }
                        } else if let Some(ref sig) = sign_hex {
                            info!("NTRIP Source-Sign present ({}…) — set BTCPC_GNSS_DEVICE_SN to enforce", &sig[..16.min(sig.len())]);
                        }

                        if socket.write_all(b"ICY 200 OK\r\n\r\n").await.is_err() { break; }
                        ntrip_acked = true;
                        info!("NTRIP handshake complete — streaming RTCM3");
                        buf.drain(..pos + 4);
                    }
                    continue;
                }

                for frame in rtcm3::parse_frames(&mut buf) {
                    frame_count += 1;
                    total_bytes += frame.payload_bytes as u64;

                    // Position lock: extract from type 1005 and validate it hasn't moved.
                    if let Some(pos) = rtcm3::extract_1005_position(&frame) {
                        let mut lock = position_lock.lock().unwrap();
                        match *lock {
                            None => {
                                info!("Position lock established: ECEF ({}, {}, {})", pos.0, pos.1, pos.2);
                                *lock = Some(pos);
                            }
                            Some(locked) => {
                                // 10000 units = 1m (units are 0.1mm). Allow 2m drift.
                                let drift = ((pos.0 - locked.0).abs())
                                    .max((pos.1 - locked.1).abs())
                                    .max((pos.2 - locked.2).abs());
                                if drift > 20000 {
                                    warn!(
                                        "Position drift detected! Δ={:.2}m — dropping frame (possible spoof)",
                                        drift as f64 / 10000.0
                                    );
                                    continue;
                                }
                            }
                        }
                    }

                    for tx in &caster_txs {
                        let _ = tx.try_send(frame.raw.clone());
                    }

                    if frame_count % config.chain_interval as u64 == 0 {
                        info!("frames={} bytes={} type={}", frame_count, total_bytes, frame.msg_type);
                        recorder.record(frame.msg_type, frame.payload_bytes).await;
                    }
                }
            }
            Err(e) => { warn!("socket read: {}", e); break; }
        }
    }
}

fn install_dnat(cfg: &Config) {
    let rule = dnat_rule(cfg);
    let status = std::process::Command::new("iptables")
        .args(&rule).status();
    match status {
        Ok(s) if s.success() => info!(
            "DNAT: {}→{}:{} → localhost:{}",
            cfg.device_ip, cfg.geodnet_ip, cfg.geodnet_port, cfg.listen_port
        ),
        Ok(_)  => warn!("iptables DNAT install failed (check permissions)"),
        Err(e) => warn!("iptables: {e}"),
    }
}

fn remove_dnat(cfg: &Config) {
    let mut rule = dnat_rule(cfg);
    // Replace -A with -D to delete
    if let Some(pos) = rule.iter().position(|s| s == "-A") {
        rule[pos] = "-D".to_string();
    }
    let _ = std::process::Command::new("iptables").args(&rule).status();
}

fn dnat_rule(cfg: &Config) -> Vec<String> {
    vec![
        "-t".into(), "nat".into(), "-A".into(), "PREROUTING".into(),
        "-i".into(), cfg.iface.clone(),
        "-p".into(), "tcp".into(),
        "-s".into(), cfg.device_ip.clone(),
        "-d".into(), cfg.geodnet_ip.clone(),
        "--dport".into(), cfg.geodnet_port.to_string(),
        "-j".into(), "REDIRECT".into(),
        "--to-ports".into(), cfg.listen_port.to_string(),
    ]
}

fn enable_ip_forward() -> Result<()> {
    std::fs::write("/proc/sys/net/ipv4/ip_forward", "1")?;
    Ok(())
}

fn spawn_arp_loop(
    iface: String, device_ip: String, gateway_ip: String,
    procs: Arc<Mutex<Vec<tokio::process::Child>>>,
) {
    for (target, host) in [
        (device_ip.clone(), gateway_ip.clone()),
        (gateway_ip.clone(), device_ip.clone()),
    ] {
        let iface = iface.clone();
        let procs = Arc::clone(&procs);
        tokio::spawn(async move {
            loop {
                match Command::new("arpspoof")
                    .args(["-i", &iface, "-t", &target, &host])
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                {
                    Ok(child) => {
                        procs.lock().unwrap().push(child);
                        // Wait for it to exit then respawn
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    }
                    Err(e) => {
                        warn!("arpspoof: {e} — apt-get install dsniff");
                        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                    }
                }
            }
        });
    }
}

async fn shutdown_signal() {
    use tokio::signal;
    let ctrl_c = async { signal::ctrl_c().await.expect("ctrl_c") };
    #[cfg(unix)]
    let sigterm = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("sigterm")
            .recv()
            .await
    };
    #[cfg(not(unix))]
    let sigterm = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {}
        _ = sigterm => {}
    }
}
