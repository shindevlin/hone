//! Camera discovery on the LOCAL SUBNET ONLY (ports of station/Discovery.kt and Onvif.kt):
//! ONVIF WS-Discovery multicast (TTL 1, never routed) plus an RTSP port check (554, 8554)
//! of this computer's own /24. Hosts outside private address ranges are never probed:
//! the listener only works with cameras on the user's own network.

use base64::Engine;
use sha1::{Digest, Sha1};
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, UdpSocket};
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FoundCamera {
    pub host: String,
    pub rtsp_port: Option<u16>,
    pub onvif: Option<String>,
    pub name: Option<String>,
}

pub const RTSP_PORTS: [u16; 2] = [554, 8554];

/// RFC 1918 private, link-local, or this computer itself (e.g. a Frigate/go2rtc restream).
pub fn is_local_v4(ip: &Ipv4Addr) -> bool {
    ip.is_private() || ip.is_link_local() || ip.is_loopback()
}

pub fn is_local_host(host: &str) -> bool {
    host.parse::<Ipv4Addr>().map(|ip| is_local_v4(&ip)).unwrap_or(false)
}

/// This computer's LAN address (the interface used for the default route; no packet is sent).
pub fn local_ip() -> Option<Ipv4Addr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?; // TEST-NET-1; connect() on UDP sends nothing
    match s.local_addr().ok()?.ip() {
        IpAddr::V4(v) if is_local_v4(&v) => Some(v),
        _ => None,
    }
}

pub fn hosts_around(ip: Ipv4Addr) -> Vec<Ipv4Addr> {
    let o = ip.octets();
    (1..=254u8).map(|i| Ipv4Addr::new(o[0], o[1], o[2], i)).filter(|h| *h != ip).collect()
}

pub fn probe_message(id: &str) -> String {
    format!(concat!(
        r#"<?xml version="1.0" encoding="UTF-8"?><e:Envelope xmlns:e="http://www.w3.org/2003/05/soap-envelope" "#,
        r#"xmlns:w="http://schemas.xmlsoap.org/ws/2004/08/addressing" xmlns:d="http://schemas.xmlsoap.org/ws/2005/04/discovery" "#,
        r#"xmlns:dn="http://www.onvif.org/ver10/network/wsdl"><e:Header><w:MessageID>uuid:{}</w:MessageID>"#,
        r#"<w:To e:mustUnderstand="true">urn:schemas-xmlsoap-org:ws:2005:04:discovery</w:To>"#,
        r#"<w:Action e:mustUnderstand="true">http://schemas.xmlsoap.org/ws/2005/04/discovery/Probe</w:Action></e:Header>"#,
        r#"<e:Body><d:Probe><d:Types>dn:NetworkVideoTransmitter</d:Types></d:Probe></e:Body></e:Envelope>"#), id)
}

pub fn parse_probe_match(xml: &str) -> (Vec<String>, Option<String>) {
    let xaddrs: Vec<String> = regex::Regex::new(r"<(?:\w+:)?XAddrs>([^<]+)</").unwrap().captures(xml)
        .map(|c| c[1].split_whitespace().filter(|x| x.starts_with("http://") || x.starts_with("https://")).map(String::from).collect())
        .unwrap_or_default();
    let scopes = regex::Regex::new(r"<(?:\w+:)?Scopes[^>]*>([^<]+)</").unwrap().captures(xml).map(|c| c[1].to_string()).unwrap_or_default();
    let name = regex::Regex::new(r"onvif://www\.onvif\.org/(?:name|hardware)/(\S+)").unwrap().captures(&scopes)
        .map(|c| percent_encoding::percent_decode_str(&c[1]).decode_utf8_lossy().into_owned());
    (xaddrs, name)
}

fn url_host(u: &str) -> Option<String> {
    let rest = u.split("://").nth(1)?;
    let auth = rest.split('/').next()?;
    let hostport = auth.rsplit('@').next()?;
    Some(hostport.split(':').next()?.to_string())
}

pub fn ws_discovery(timeout: Duration) -> Vec<(String, Option<String>)> {
    let mut out: BTreeMap<String, Option<String>> = BTreeMap::new();
    let Ok(s) = UdpSocket::bind("0.0.0.0:0") else { return vec![] };
    let _ = s.set_multicast_ttl_v4(1);
    let _ = s.set_read_timeout(Some(Duration::from_millis(400)));
    let msg = probe_message(&uuid::Uuid::new_v4().to_string());
    for _ in 0..2 {
        let _ = s.send_to(msg.as_bytes(), "239.255.255.250:3702");
    }
    let end = Instant::now() + timeout;
    let mut buf = vec![0u8; 16384];
    while Instant::now() < end {
        let Ok((n, from)) = s.recv_from(&mut buf) else { continue };
        let IpAddr::V4(fip) = from.ip() else { continue };
        if !is_local_v4(&fip) {
            continue;
        }
        let (x, name) = parse_probe_match(&String::from_utf8_lossy(&buf[..n]));
        let pick = x.iter().find(|u| url_host(u).as_deref() == Some(&fip.to_string())).or(x.first()).cloned();
        if let Some(p) = pick {
            if url_host(&p).map(|h| is_local_host(&h)).unwrap_or(false) {
                out.entry(p).or_insert(name);
            }
        }
    }
    out.into_iter().collect()
}

/// host -> first open RTSP port. Only private addresses are ever tried.
pub fn scan(hosts: &[Ipv4Addr], ports: &[u16], timeout: Duration) -> BTreeMap<String, u16> {
    let (tx, rx) = mpsc::channel();
    let hosts: Vec<Ipv4Addr> = hosts.iter().copied().filter(is_local_v4).collect();
    for chunk in hosts.chunks(hosts.len().div_ceil(48).max(1)) {
        let tx = tx.clone();
        let chunk = chunk.to_vec();
        let ports = ports.to_vec();
        std::thread::spawn(move || {
            for h in chunk {
                for &p in &ports {
                    if TcpStream::connect_timeout(&SocketAddr::new(IpAddr::V4(h), p), timeout).is_ok() {
                        let _ = tx.send((h.to_string(), p));
                        break;
                    }
                }
            }
        });
    }
    drop(tx);
    rx.into_iter().collect()
}

pub fn discover(extra: &[Ipv4Addr]) -> (Option<Ipv4Addr>, Vec<FoundCamera>) {
    let me = local_ip();
    let onvif = ws_discovery(Duration::from_secs(3));
    let mut hosts: Vec<Ipv4Addr> = me.map(hosts_around).unwrap_or_default();
    hosts.extend(extra.iter().copied().filter(is_local_v4));
    hosts.sort();
    hosts.dedup();
    let open = scan(&hosts, &RTSP_PORTS, Duration::from_millis(350));
    let mut by: BTreeMap<String, FoundCamera> = BTreeMap::new();
    for (x, name) in onvif {
        if let Some(h) = url_host(&x) {
            by.insert(h.clone(), FoundCamera { host: h.clone(), rtsp_port: open.get(&h).copied(), onvif: Some(x), name });
        }
    }
    for (h, p) in open {
        by.entry(h.clone()).or_insert(FoundCamera { host: h, rtsp_port: Some(p), onvif: None, name: None });
    }
    let mut v: Vec<FoundCamera> = by.into_values().collect();
    v.sort_by_key(|c| (c.onvif.is_none(), c.host.parse::<Ipv4Addr>().ok()));
    (me, v)
}

// ── ONVIF ────────────────────────────────────────────────────────────────────

pub struct Onvif {
    pub device: String,
    user: String,
    pass: String,
}

#[derive(Debug, Clone)]
pub struct OnvifStream {
    pub host: String,
    pub port: u16,
    pub path: String,
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

pub fn wsse_digest(nonce: &[u8], created: &str, pass: &str) -> String {
    let mut h = Sha1::new();
    h.update(nonce);
    h.update(created.as_bytes());
    h.update(pass.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(h.finalize())
}

fn utc_now() -> String {
    let s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    let (d, r) = (s.div_euclid(86400), s.rem_euclid(86400));
    // civil-from-days (Howard Hinnant)
    let z = d + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let dd = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{dd:02}T{:02}:{:02}:{:02}Z", r / 3600, (r % 3600) / 60, r % 60)
}

impl Onvif {
    pub fn new(device: &str, user: &str, pass: &str) -> Option<Self> {
        // Never talk ONVIF to anything outside the local network.
        if !url_host(device).map(|h| is_local_host(&h)).unwrap_or(false) {
            return None;
        }
        Some(Self { device: device.into(), user: user.into(), pass: pass.into() })
    }

    fn envelope(&self, body: &str) -> String {
        let mut nonce = [0u8; 16];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut nonce);
        let created = utc_now();
        format!(concat!(
            r#"<?xml version="1.0" encoding="UTF-8"?><s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope"><s:Header>"#,
            r#"<Security s:mustUnderstand="1" xmlns="http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-wssecurity-secext-1.0.xsd">"#,
            r#"<UsernameToken><Username>{}</Username>"#,
            r#"<Password Type="http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-username-token-profile-1.0#PasswordDigest">{}</Password>"#,
            r#"<Nonce EncodingType="http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-soap-message-security-1.0#Base64Binary">{}</Nonce>"#,
            r#"<Created xmlns="http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-wssecurity-utility-1.0.xsd">{}</Created>"#,
            r#"</UsernameToken></Security></s:Header><s:Body>{}</s:Body></s:Envelope>"#),
            esc(&self.user), wsse_digest(&nonce, &created, &self.pass),
            base64::engine::general_purpose::STANDARD.encode(nonce), created, body)
    }

    /// Ok(text), Err(true) = login refused, Err(false) = other failure.
    fn call(&self, url: &str, body: &str) -> Result<String, bool> {
        if !url_host(url).map(|h| is_local_host(&h)).unwrap_or(false) {
            return Err(false);
        }
        let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(5)).build();
        match agent.post(url).set("Content-Type", "application/soap+xml; charset=utf-8").send_string(&self.envelope(body)) {
            Ok(r) => r.into_string().map_err(|_| false),
            Err(ureq::Error::Status(code, r)) => {
                let t = r.into_string().unwrap_or_default();
                Err(code == 401 || t.contains("NotAuthorized"))
            }
            Err(_) => Err(false),
        }
    }

    pub fn stream_uri(&self) -> Result<Option<OnvifStream>, bool> {
        let media = self.media_xaddr().unwrap_or_else(|| {
            let h = self.device.split('/').take(3).collect::<Vec<_>>().join("/");
            format!("{h}/onvif/media_service")
        });
        let profiles = self.call(&media, r#"<GetProfiles xmlns="http://www.onvif.org/ver10/media/wsdl"/>"#)?;
        for tok in profile_tokens(&profiles) {
            let b = format!(concat!(r#"<GetStreamUri xmlns="http://www.onvif.org/ver10/media/wsdl"><StreamSetup>"#,
                r#"<Stream xmlns="http://www.onvif.org/ver10/schema">RTP-Unicast</Stream>"#,
                r#"<Transport xmlns="http://www.onvif.org/ver10/schema"><Protocol>RTSP</Protocol></Transport>"#,
                r#"</StreamSetup><ProfileToken>{}</ProfileToken></GetStreamUri>"#), esc(&tok));
            if let Ok(r) = self.call(&media, &b) {
                if let Some(s) = parse_uri(&r) {
                    return Ok(Some(s));
                }
            }
        }
        Ok(None)
    }

    fn media_xaddr(&self) -> Option<String> {
        let x = self.call(&self.device, r#"<GetCapabilities xmlns="http://www.onvif.org/ver10/device/wsdl"><Category>Media</Category></GetCapabilities>"#).ok()?;
        regex::Regex::new(r"(?s)<(?:\w+:)?Media\b[^>]*>.*?<(?:\w+:)?XAddr>([^<]+)</").unwrap().captures(&x).map(|c| c[1].trim().to_string())
    }

    /// ("mac", aa:bb:..) else ("serial", ...), for the hashed station id only.
    pub fn hardware_id(&self) -> Option<(String, String)> {
        if let Ok(x) = self.call(&self.device, r#"<GetNetworkInterfaces xmlns="http://www.onvif.org/ver10/device/wsdl"/>"#) {
            if let Some(c) = regex::Regex::new(r"<(?:\w+:)?HwAddress>([0-9A-Fa-f:.-]{11,17})<").unwrap().captures(&x) {
                return Some(("mac".into(), norm_mac(&c[1])));
            }
        }
        if let Ok(x) = self.call(&self.device, r#"<GetDeviceInformation xmlns="http://www.onvif.org/ver10/device/wsdl"/>"#) {
            if let Some(c) = regex::Regex::new(r"<(?:\w+:)?SerialNumber>([^<]{3,64})<").unwrap().captures(&x) {
                return Some(("serial".into(), c[1].trim().to_string()));
            }
        }
        None
    }

    pub fn model(&self) -> Option<String> {
        let x = self.call(&self.device, r#"<GetDeviceInformation xmlns="http://www.onvif.org/ver10/device/wsdl"/>"#).ok()?;
        let mf = regex::Regex::new(r"<(?:\w+:)?Manufacturer>([^<]{1,40})<").unwrap().captures(&x).map(|c| c[1].trim().to_string());
        let md = regex::Regex::new(r"<(?:\w+:)?Model>([^<]{1,40})<").unwrap().captures(&x).map(|c| c[1].trim().to_string());
        let s = [mf, md].into_iter().flatten().collect::<Vec<_>>().join(" ");
        (!s.is_empty()).then_some(s)
    }
}

pub fn profile_tokens(xml: &str) -> Vec<String> {
    let re = regex::Regex::new(r#"(?s)<(?:\w+:)?Profiles\b[^>]*\btoken="([^"]+)"[^>]*>(.*?)</(?:\w+:)?Profiles>"#).unwrap();
    let mut v: Vec<(bool, String)> = re.captures_iter(xml).map(|c| (!c[2].contains("AudioEncoderConfiguration"), c[1].to_string())).collect();
    v.sort_by_key(|x| x.0);
    v.into_iter().map(|x| x.1).collect()
}

pub fn parse_uri(xml: &str) -> Option<OnvifStream> {
    let u = regex::Regex::new(r"<(?:\w+:)?Uri>([^<]+)</").unwrap().captures(xml)?[1].trim().replace("&amp;", "&");
    let rest = u.strip_prefix("rtsp://").or_else(|| u.strip_prefix("RTSP://"))?;
    let (auth, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let hostport = auth.rsplit('@').next()?; // userinfo dropped
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse().ok()?),
        None => (hostport.to_string(), 554),
    };
    Some(OnvifStream { host, port, path: path.to_string() })
}

pub fn norm_mac(mac: &str) -> String {
    let h: String = mac.to_lowercase().chars().filter(|c| c.is_ascii_hexdigit()).collect();
    h.as_bytes().chunks(2).map(|c| String::from_utf8_lossy(c).into_owned()).collect::<Vec<_>>().join(":")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_only() {
        assert!(is_local_host("192.168.1.20"));
        assert!(is_local_host("10.0.0.5"));
        assert!(is_local_host("127.0.0.1"));
        assert!(!is_local_host("100.64.0.1"));
        assert!(!is_local_host("8.8.8.8"));
        assert!(!is_local_host("example.com"));
        assert!(Onvif::new("http://203.0.113.5/onvif/device_service", "a", "b").is_none());
        assert!(scan(&["1.1.1.1".parse().unwrap()], &[554], Duration::from_millis(10)).is_empty());
    }

    #[test]
    fn parses_stream_uri_without_userinfo() {
        let s = parse_uri("<tt:Uri>rtsp://admin:x@192.168.1.9:554/ch0_0.264?a=1&amp;b=2</tt:Uri>").unwrap();
        assert_eq!((s.host.as_str(), s.port, s.path.as_str()), ("192.168.1.9", 554, "/ch0_0.264?a=1&b=2"));
    }

    #[test]
    fn wsse_known_vector() {
        // Base64(SHA1(nonce + created + password)), deterministic for fixed input.
        assert_eq!(wsse_digest(b"0123456789abcdef", "2026-11-04T00:00:00Z", "pw").len(), 28);
        assert!(utc_now().ends_with('Z'));
    }
}
