use std::env;

#[derive(Clone)]
pub struct Caster {
    pub name:       String,
    pub host:       String,
    pub port:       u16,
    pub username:   String,
    pub password:   String,
    pub mountpoint: String,
    pub enabled:    bool,
}

#[derive(Clone)]
pub struct Config {
    pub device_ip:      String,
    pub device_sn:      String,   // serial number — checked against NTRIP Authorization header
    pub geodnet_ip:     String,
    pub geodnet_port:   u16,
    pub listen_port:    u16,
    pub iface:          String,
    pub arp_spoof:      bool,
    pub gateway_ip:     String,
    pub miner:          String,
    pub api_port:       u16,
    pub auth_token:     String,
    pub forward:        bool,
    pub chain_interval: u32,
    pub casters:        Vec<Caster>,
}

impl Config {
    pub fn from_env() -> Self {
        let device_ip = env::var("BTCPC_GNSS_DEVICE_IP")
            .or_else(|_| env::var("BTCPC_GNSS_HOST"))
            .unwrap_or_else(|_| "192.168.68.54".to_string());

        let device_sn = env::var("BTCPC_GNSS_DEVICE_SN").unwrap_or_default();

        let geodnet_ip = env::var("BTCPC_GNSS_GEODNET_IP")
            .unwrap_or_else(|_| "52.8.236.207".to_string());

        let geodnet_port: u16 = env::var("BTCPC_GNSS_GEODNET_PORT")
            .ok().and_then(|s| s.parse().ok()).unwrap_or(2201);

        let listen_port: u16 = env::var("BTCPC_GNSS_LISTEN_PORT")
            .ok().and_then(|s| s.parse().ok()).unwrap_or(geodnet_port);

        let gateway_ip = env::var("BTCPC_GNSS_GATEWAY_IP").unwrap_or_default();

        let arp_spoof = env::var("BTCPC_GNSS_ARP_SPOOF")
            .map(|v| v != "false")
            .unwrap_or(!gateway_ip.is_empty());

        let iface = env::var("BTCPC_GNSS_IFACE")
            .or_else(|_| env::var("BTCPC_GNSS_WIFI_IFACE"))
            .unwrap_or_else(|_| detect_iface(&device_ip));

        let miner = env::var("BTCPC_GNSS_MINER")
            .or_else(|_| env::var("BTCPC_MINER"))
            .unwrap_or_else(|_| "shindevlin".to_string());

        let api_port: u16 = env::var("BTCPC_API_PORT")
            .ok().and_then(|s| s.parse().ok()).unwrap_or(4242);

        let auth_token = env::var("BTCPC_AUTH_TOKEN").unwrap_or_default();

        let forward = env::var("BTCPC_GNSS_FORWARD_CASTERS")
            .map(|s| s == "true").unwrap_or(false);

        let chain_interval: u32 = env::var("BTCPC_GNSS_CHAIN_INTERVAL")
            .ok().and_then(|s| s.parse().ok()).unwrap_or(30);

        let casters = vec![
            Caster {
                name:       "onocoy".to_string(),
                host:       env::var("BTCPC_ONOCOY_HOST").unwrap_or_else(|_| "servers.onocoy.com".to_string()),
                port:       env::var("BTCPC_ONOCOY_PORT").ok().and_then(|s| s.parse().ok()).unwrap_or(2121),
                username:   env::var("BTCPC_ONOCOY_USER").unwrap_or_default(),
                password:   env::var("BTCPC_ONOCOY_PASS").unwrap_or_default(),
                mountpoint: env::var("BTCPC_ONOCOY_MOUNT").unwrap_or_default(),
                enabled:    forward && env::var("BTCPC_ONOCOY_USER").map(|s| !s.is_empty()).unwrap_or(false),
            },
            Caster {
                name:       "rtkdirect".to_string(),
                host:       env::var("BTCPC_RTK_HOST").unwrap_or_else(|_| "ntrip.rtkdirect.com".to_string()),
                port:       env::var("BTCPC_RTK_PORT").ok().and_then(|s| s.parse().ok()).unwrap_or(2101),
                username:   env::var("BTCPC_RTK_USER").unwrap_or_default(),
                password:   env::var("BTCPC_RTK_PASS").unwrap_or_default(),
                mountpoint: env::var("BTCPC_RTK_MOUNT").unwrap_or_default(),
                enabled:    forward && env::var("BTCPC_RTK_USER").map(|s| !s.is_empty()).unwrap_or(false),
            },
        ];

        Self {
            device_ip, device_sn, geodnet_ip, geodnet_port, listen_port,
            iface, arp_spoof, gateway_ip, miner, api_port, auth_token,
            forward, chain_interval, casters,
        }
    }
}

fn detect_iface(device_ip: &str) -> String {
    // Try ip route first
    if let Ok(out) = std::process::Command::new("ip")
        .args(["route", "get", device_ip])
        .output()
    {
        let s = String::from_utf8_lossy(&out.stdout);
        if let Some(pos) = s.find("dev ") {
            let rest = &s[pos + 4..];
            let iface = rest.split_whitespace().next().unwrap_or("").to_string();
            if !iface.is_empty() { return iface; }
        }
    }
    "enp0s31f6".to_string()
}
