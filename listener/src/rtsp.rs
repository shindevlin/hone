//! Audio-only RTSP camera client, ported from the Android app (station/Rtsp.kt): DESCRIBE,
//! SETUP of the audio track only (RTP interleaved over the RTSP TCP connection), PLAY.
//! The video track is never SETUP, so the camera never sends video. Basic and Digest auth.
//! Credentials go only into Authorization headers; URLs and messages never carry them.
//! Used for the sound-quality probe at enrolment; continuous listening goes through ffmpeg
//! with the same audio-only rule (see pipeline.rs).

use base64::Engine;
use md5::{Digest, Md5};
use rand::RngCore;
use std::collections::HashMap;
use std::io::{BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct RtspError {
    pub code: u16,
    pub msg: String,
}
impl std::fmt::Display for RtspError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.msg)
    }
}
pub fn err(code: u16, msg: impl Into<String>) -> RtspError {
    RtspError { code, msg: msg.into() }
}

#[derive(Debug, Clone)]
pub struct SdpMedia {
    pub kind: String,
    pub payload_type: i32,
    pub encoding: String,
    pub clock_rate: u32,
    pub channels: usize,
    pub control: Option<String>,
    pub fmtp: HashMap<String, String>,
}

pub const SUPPORTED: [&str; 4] = ["PCMA", "PCMU", "L16", "MPEG4-GENERIC"];

pub fn sdp_parse(sdp: &str) -> (Option<String>, Vec<SdpMedia>) {
    let mut session_control = None;
    let mut out: Vec<SdpMedia> = Vec::new();
    let mut cur: Option<SdpMedia> = None;
    for raw in sdp.lines() {
        let l = raw.trim();
        if let Some(rest) = l.strip_prefix("m=") {
            if let Some(m) = cur.take() {
                out.push(m);
            }
            let p: Vec<&str> = rest.split(' ').collect();
            let pt = p.get(3).and_then(|x| x.parse().ok()).unwrap_or(-1);
            let (enc, rate, ch) = match pt {
                0 => ("PCMU", 8000, 1),
                8 => ("PCMA", 8000, 1),
                10 => ("L16", 44100, 2),
                11 => ("L16", 44100, 1),
                _ => ("", 0, 1),
            };
            cur = Some(SdpMedia {
                kind: p[0].to_string(),
                payload_type: pt,
                encoding: enc.into(),
                clock_rate: rate,
                channels: ch,
                control: None,
                fmtp: HashMap::new(),
            });
        } else if let Some(c) = l.strip_prefix("a=control:") {
            match cur.as_mut() {
                None => session_control = Some(c.trim().to_string()),
                Some(m) => m.control = Some(c.trim().to_string()),
            }
        } else if let (Some(r), Some(m)) = (l.strip_prefix("a=rtpmap:"), cur.as_mut()) {
            let mut parts = r.splitn(2, ' ');
            if parts.next().and_then(|x| x.parse::<i32>().ok()) == Some(m.payload_type) {
                let q: Vec<&str> = parts.next().unwrap_or("").split('/').collect();
                m.encoding = q[0].to_uppercase();
                m.clock_rate = q.get(1).and_then(|x| x.parse().ok()).unwrap_or(m.clock_rate);
                m.channels = q.get(2).and_then(|x| x.parse().ok()).unwrap_or(1);
            }
        } else if let (Some(r), Some(m)) = (l.strip_prefix("a=fmtp:"), cur.as_mut()) {
            let mut parts = r.splitn(2, ' ');
            if parts.next().and_then(|x| x.parse::<i32>().ok()) == Some(m.payload_type) {
                for kv in parts.next().unwrap_or("").split(';') {
                    if let Some(i) = kv.find('=') {
                        if i > 0 {
                            m.fmtp.insert(kv[..i].trim().to_lowercase(), kv[i + 1..].trim().to_string());
                        }
                    }
                }
            }
        }
    }
    if let Some(m) = cur.take() {
        out.push(m);
    }
    for m in out.iter_mut() {
        m.encoding = m.encoding.to_uppercase();
    }
    (session_control, out)
}

pub fn pick_audio(media: &[SdpMedia]) -> Option<&SdpMedia> {
    media.iter().filter(|m| m.kind == "audio").find(|m| SUPPORTED.contains(&m.encoding.as_str()))
}

/// AAC AudioSpecificConfig (hex from fmtp config=) -> (sample rate, channels).
pub fn aac_config(hexs: &str) -> Option<(u32, usize)> {
    let b = hex::decode(hexs).ok()?;
    if b.len() < 2 {
        return None;
    }
    let idx = (((b[0] & 0x07) << 1) | ((b[1] & 0x80) >> 7)) as usize;
    let ch = ((b[1] & 0x78) >> 3) as usize;
    const RATES: [u32; 13] = [96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350];
    Some((*RATES.get(idx)?, ch.max(1)))
}

/// Sample rate the track carries (AAC: from its AudioSpecificConfig).
pub fn rate_of(m: &SdpMedia) -> u32 {
    if m.encoding == "MPEG4-GENERIC" {
        m.fmtp.get("config").and_then(|c| aac_config(c)).map(|x| x.0).unwrap_or(m.clock_rate)
    } else {
        m.clock_rate
    }
}

struct Auth {
    user: Option<String>,
    pass: Option<String>,
    scheme: Option<&'static str>,
    realm: String,
    nonce: String,
    qop: Option<String>,
    opaque: Option<String>,
    nc: u32,
}

fn md5hex(s: &str) -> String {
    hex::encode(Md5::digest(s.as_bytes()))
}

fn auth_params(s: &str) -> HashMap<String, String> {
    let re = regex::Regex::new(r#"(\w+)=(?:"([^"]*)"|([^,\s]*))"#).unwrap();
    re.captures_iter(s)
        .map(|c| {
            let v = c.get(2).map(|m| m.as_str()).filter(|x| !x.is_empty()).or(c.get(3).map(|m| m.as_str())).unwrap_or("");
            (c[1].to_lowercase(), v.to_string())
        })
        .collect()
}

impl Auth {
    fn has_creds(&self) -> bool {
        self.user.as_deref().map(|u| !u.is_empty()).unwrap_or(false)
    }
    fn accept(&mut self, headers: &[String]) -> bool {
        if let Some(d) = headers.iter().find(|h| h.trim().to_lowercase().starts_with("digest")) {
            let p = auth_params(d.trim().split_once(' ').map(|x| x.1).unwrap_or(""));
            let Some(new_nonce) = p.get("nonce").cloned() else { return false };
            let stale = p.get("stale").map(|s| s.eq_ignore_ascii_case("true")).unwrap_or(false);
            if self.scheme == Some("Digest") && new_nonce == self.nonce && !stale {
                return false;
            }
            self.scheme = Some("Digest");
            self.realm = p.get("realm").cloned().unwrap_or_default();
            self.nonce = new_nonce;
            self.opaque = p.get("opaque").cloned();
            self.qop = p.get("qop").and_then(|q| q.split(',').map(|x| x.trim()).find(|x| *x == "auth").map(String::from));
            self.nc = 0;
            return true;
        }
        if headers.iter().any(|h| h.trim().to_lowercase().starts_with("basic")) && self.scheme != Some("Basic") {
            self.scheme = Some("Basic");
            return true;
        }
        false
    }
    fn header(&mut self, method: &str, uri: &str) -> Option<String> {
        if !self.has_creds() {
            return None;
        }
        let user = self.user.clone().unwrap_or_default();
        let pass = self.pass.clone().unwrap_or_default();
        match self.scheme {
            Some("Basic") => Some(format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{user}:{pass}")))),
            Some("Digest") => {
                let ha1 = md5hex(&format!("{user}:{}:{pass}", self.realm));
                let ha2 = md5hex(&format!("{method}:{uri}"));
                let mut sb = format!("Digest username=\"{user}\", realm=\"{}\", nonce=\"{}\", uri=\"{uri}\"", self.realm, self.nonce);
                if let Some(q) = self.qop.clone() {
                    self.nc += 1;
                    let ncs = format!("{:08x}", self.nc);
                    let mut cn = [0u8; 8];
                    rand::thread_rng().fill_bytes(&mut cn);
                    let cnonce = hex::encode(cn);
                    let resp = md5hex(&format!("{ha1}:{}:{ncs}:{cnonce}:{q}:{ha2}", self.nonce));
                    sb.push_str(&format!(", response=\"{resp}\", qop={q}, nc={ncs}, cnonce=\"{cnonce}\""));
                } else {
                    sb.push_str(&format!(", response=\"{}\"", md5hex(&format!("{ha1}:{}:{ha2}", self.nonce))));
                }
                if let Some(o) = &self.opaque {
                    sb.push_str(&format!(", opaque=\"{o}\""));
                }
                Some(sb)
            }
            _ => None,
        }
    }
}

pub struct Response {
    pub code: u16,
    pub headers: HashMap<String, Vec<String>>,
    pub body: String,
}
impl Response {
    fn h(&self, n: &str) -> Option<&str> {
        self.headers.get(&n.to_lowercase()).and_then(|v| v.first()).map(|s| s.as_str())
    }
}

/// RTP payload of one packet (payload, timestamp, marker).
pub fn rtp_payload(p: &[u8]) -> Option<&[u8]> {
    if p.len() < 12 || (p[0] & 0xC0) != 0x80 {
        return None;
    }
    let cc = (p[0] & 0x0F) as usize;
    let ext = p[0] & 0x10 != 0;
    let pad = p[0] & 0x20 != 0;
    let mut off = 12 + 4 * cc;
    if ext {
        if p.len() < off + 4 {
            return None;
        }
        off += 4 + 4 * (((p[off + 2] as usize) << 8) | p[off + 3] as usize);
    }
    let mut end = p.len();
    if pad {
        end = end.checked_sub(p[p.len() - 1] as usize)?;
    }
    if off > end {
        return None;
    }
    Some(&p[off..end])
}

pub struct RtspAudioClient {
    host: String,
    port: u16,
    path: String,
    auth: Auth,
    timeout: Duration,
    sock: Option<TcpStream>,
    input: Option<BufReader<TcpStream>>,
    cseq: u32,
    session: Option<String>,
    channel: u8,
    play_url: String,
    pub bytes_in: u64,
    pub video_tracks_skipped: usize,
}

impl RtspAudioClient {
    pub fn new(host: &str, port: u16, path: &str, user: Option<&str>, pass: Option<&str>, timeout_ms: u64) -> Self {
        let path = if path.starts_with('/') { path.to_string() } else { format!("/{path}") };
        Self {
            host: host.into(),
            port,
            path,
            auth: Auth { user: user.map(String::from), pass: pass.map(String::from), scheme: None, realm: String::new(), nonce: String::new(), qop: None, opaque: None, nc: 0 },
            timeout: Duration::from_millis(timeout_ms),
            sock: None,
            input: None,
            cseq: 0,
            session: None,
            channel: 0,
            play_url: String::new(),
            bytes_in: 0,
            video_tracks_skipped: 0,
        }
    }

    /// Stream URL without credentials (safe to show).
    pub fn url(&self) -> String {
        format!("rtsp://{}:{}{}", self.host, self.port, self.path)
    }

    pub fn open(&mut self) -> Result<SdpMedia, RtspError> {
        let addr = (self.host.as_str(), self.port).to_socket_addrs().map_err(|_| err(0, "unreachable"))?.next().ok_or(err(0, "unreachable"))?;
        let s = TcpStream::connect_timeout(&addr, self.timeout).map_err(|_| err(0, "unreachable"))?;
        s.set_read_timeout(Some(self.timeout)).ok();
        s.set_nodelay(true).ok();
        self.input = Some(BufReader::with_capacity(65536, s.try_clone().map_err(|_| err(0, "socket"))?));
        self.sock = Some(s);
        let url = self.url();
        let d = self.request("DESCRIBE", &url, &[("Accept", "application/sdp")])?;
        let base0 = d.h("content-base").or(d.h("content-location")).unwrap_or(&url).to_string();
        let base = if base0.ends_with('/') { base0.clone() } else { format!("{base0}/") };
        let (sess_ctl, medias) = sdp_parse(&d.body);
        self.video_tracks_skipped = medias.iter().filter(|m| m.kind != "audio").count();
        let audio: Vec<&SdpMedia> = medias.iter().filter(|m| m.kind == "audio").collect();
        let a = pick_audio(&medias).cloned().ok_or_else(|| {
            if audio.is_empty() {
                err(415, "this camera stream has no audio track")
            } else {
                err(415, format!("audio codec not supported: {}", audio.iter().map(|m| m.encoding.as_str()).collect::<Vec<_>>().join(", ")))
            }
        })?;
        let setup_url = resolve(&base, a.control.as_deref());
        let st = self.request("SETUP", &setup_url, &[("Transport", "RTP/AVP/TCP;unicast;interleaved=0-1")])?;
        if let Some(v) = st.h("session") {
            self.session = Some(v.split(';').next().unwrap_or("").trim().to_string());
        }
        if let Some(t) = st.h("transport") {
            if let Some(c) = regex::Regex::new(r"interleaved=(\d+)").unwrap().captures(t) {
                self.channel = c[1].parse().unwrap_or(0);
            }
        }
        self.play_url = match sess_ctl.as_deref() {
            Some(c) if c != "*" => resolve(&base, Some(c)),
            _ => base0,
        };
        let pu = self.play_url.clone();
        self.request("PLAY", &pu, &[("Range", "npt=0.000-")])?;
        Ok(a)
    }

    fn send(&mut self, method: &str, uri: &str, extra: &[(&str, &str)]) -> Result<(), RtspError> {
        self.cseq += 1;
        let mut sb = format!("{method} {uri} RTSP/1.0\r\nCSeq: {}\r\nUser-Agent: HONE-listener/1\r\n", self.cseq);
        if let Some(s) = &self.session {
            sb.push_str(&format!("Session: {s}\r\n"));
        }
        if let Some(a) = self.auth.header(method, uri) {
            sb.push_str(&format!("Authorization: {a}\r\n"));
        }
        for (k, v) in extra {
            sb.push_str(&format!("{k}: {v}\r\n"));
        }
        sb.push_str("\r\n");
        self.sock.as_mut().ok_or(err(0, "closed"))?.write_all(sb.as_bytes()).map_err(|_| err(0, "camera closed the connection"))
    }

    fn request(&mut self, method: &str, uri: &str, extra: &[(&str, &str)]) -> Result<Response, RtspError> {
        self.send(method, uri, extra)?;
        let mut r = self.read_response()?;
        if r.code == 401 {
            if !self.auth.has_creds() {
                return Err(err(401, "the camera needs a username and password"));
            }
            let www = r.headers.get("www-authenticate").cloned().unwrap_or_default();
            if self.auth.accept(&www) {
                self.send(method, uri, extra)?;
                r = self.read_response()?;
            }
            if r.code == 401 {
                return Err(err(401, "wrong username or password"));
            }
        }
        if r.code != 200 {
            return Err(err(r.code, format!("{method} failed ({})", r.code)));
        }
        Ok(r)
    }

    fn byte(&mut self) -> Result<u8, RtspError> {
        let mut b = [0u8; 1];
        match self.input.as_mut().ok_or(err(0, "closed"))?.read(&mut b) {
            Ok(1) => Ok(b[0]),
            _ => Err(err(0, "camera closed the connection")),
        }
    }

    fn read_line(&mut self, first: u8) -> Result<String, RtspError> {
        let mut v = Vec::new();
        let mut c = first;
        while c != b'\n' {
            if c != b'\r' {
                v.push(c);
            }
            c = self.byte()?;
        }
        Ok(String::from_utf8_lossy(&v).into_owned())
    }

    fn read_n(&mut self, n: usize) -> Result<Vec<u8>, RtspError> {
        let mut b = vec![0u8; n];
        self.input.as_mut().ok_or(err(0, "closed"))?.read_exact(&mut b).map_err(|_| err(0, "camera closed the connection"))?;
        self.bytes_in += n as u64;
        Ok(b)
    }

    fn read_headers(&mut self, status: &str) -> Result<Response, RtspError> {
        let code = status.split(' ').nth(1).and_then(|x| x.parse().ok()).ok_or(err(0, "bad RTSP reply"))?;
        let mut h: HashMap<String, Vec<String>> = HashMap::new();
        loop {
            let b = self.byte()?;
            let l = self.read_line(b)?;
            if l.is_empty() {
                break;
            }
            if let Some(i) = l.find(':') {
                h.entry(l[..i].trim().to_lowercase()).or_default().push(l[i + 1..].trim().to_string());
            }
        }
        let n: usize = h.get("content-length").and_then(|v| v.first()).and_then(|x| x.parse().ok()).unwrap_or(0);
        let body = if n > 0 { String::from_utf8_lossy(&self.read_n(n)?).into_owned() } else { String::new() };
        Ok(Response { code, headers: h, body })
    }

    fn read_response(&mut self) -> Result<Response, RtspError> {
        loop {
            let b = self.byte()?;
            if b == b'$' {
                self.read_frame()?;
                continue;
            }
            let status = self.read_line(b)?;
            if status.starts_with("RTSP/") {
                return self.read_headers(&status);
            }
        }
    }

    fn read_frame(&mut self) -> Result<(u8, Vec<u8>), RtspError> {
        let ch = self.byte()?;
        let hi = self.byte()? as usize;
        let lo = self.byte()? as usize;
        Ok((ch, self.read_n((hi << 8) | lo)?))
    }

    /// Reads audio RTP payloads for [secs] seconds.
    pub fn stream_for(&mut self, secs: f64, mut on_rtp: impl FnMut(&[u8])) -> Result<(), RtspError> {
        let t0 = Instant::now();
        while t0.elapsed().as_secs_f64() < secs {
            let b = self.byte()?;
            if b == b'$' {
                let (ch, data) = self.read_frame()?;
                if ch == self.channel {
                    if let Some(p) = rtp_payload(&data) {
                        on_rtp(p);
                    }
                }
            } else {
                let status = self.read_line(b)?;
                if status.starts_with("RTSP/") {
                    self.read_headers(&status)?;
                }
            }
        }
        Ok(())
    }

    pub fn close(&mut self) {
        if self.session.is_some() && !self.play_url.is_empty() {
            let pu = self.play_url.clone();
            let _ = self.send("TEARDOWN", &pu, &[]);
        }
        if let Some(s) = self.sock.take() {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
    }
}

fn resolve(base: &str, ctl: Option<&str>) -> String {
    match ctl {
        None | Some("*") => base.trim_end_matches('/').to_string(),
        Some(c) if c.to_lowercase().starts_with("rtsp://") => c.to_string(),
        Some(c) => format!("{base}{}", c.trim_start_matches('/')),
    }
}

/// ESEE first, then common generic paths.
pub const FALLBACK_PATHS: [&str; 5] = ["/ch0_0.264", "/ch0_1.264", "/live", "/stream1", "/h264"];

/// Reconnect delay after [failures] consecutive failures: 5 s doubling to 5 min, with jitter.
pub fn backoff_ms(failures: u32) -> u64 {
    let base = (5_000u64 << failures.min(6)).min(300_000);
    base / 2 + rand::random::<u64>() % (base / 2).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn esee_sdp_picks_pcma_and_skips_video() {
        let sdp = "v=0\r\no=- 0 0 IN IP4 0.0.0.0\r\ns=ESEE\r\na=control:*\r\nm=video 0 RTP/AVP 96\r\na=rtpmap:96 H264/90000\r\na=control:trackID=0\r\nm=audio 0 RTP/AVP 8\r\na=rtpmap:8 PCMA/8000\r\na=control:trackID=1\r\n";
        let (sc, m) = sdp_parse(sdp);
        assert_eq!(sc.as_deref(), Some("*"));
        let a = pick_audio(&m).unwrap();
        assert_eq!((a.encoding.as_str(), a.clock_rate, a.control.as_deref()), ("PCMA", 8000, Some("trackID=1")));
        assert_eq!(m.iter().filter(|x| x.kind != "audio").count(), 1);
    }

    #[test]
    fn aac_config_parses() {
        assert_eq!(aac_config("1588"), Some((8000, 1)));
        assert_eq!(aac_config("1190"), Some((48000, 2)));
    }
}
