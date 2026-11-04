//! Loopback RTSP auth relay, so the camera login never appears on ffmpeg's command line
//! (process lists are readable by other local users). ffmpeg connects to
//! rtsp://127.0.0.1:<random port>/<path> with no credentials; the relay forwards every byte to
//! the camera, rewrites request URIs to the camera's address and adds the Authorization header
//! (Basic or Digest, the same code as the probe), answering the camera's 401 challenges itself.
//! One ffmpeg session per relay; only connections from this computer are accepted.

use crate::rtsp::Auth;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct Msg {
    head: Vec<String>,
    body: Vec<u8>,
}

enum Unit {
    Frame(Vec<u8>),
    Msg(Msg),
}

fn read_unit(r: &mut BufReader<TcpStream>) -> std::io::Result<Unit> {
    let mut first = [0u8; 1];
    r.read_exact(&mut first)?;
    if first[0] == b'$' {
        let mut h = [0u8; 3];
        r.read_exact(&mut h)?;
        let n = ((h[1] as usize) << 8) | h[2] as usize;
        let mut v = vec![0u8; 4 + n];
        v[0] = b'$';
        v[1..4].copy_from_slice(&h);
        r.read_exact(&mut v[4..])?;
        return Ok(Unit::Frame(v));
    }
    let mut line = vec![first[0]];
    let mut head = Vec::new();
    loop {
        r.read_until(b'\n', &mut line)?;
        if line.is_empty() {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        let l = String::from_utf8_lossy(&line).trim_end_matches(['\r', '\n']).to_string();
        line.clear();
        if l.is_empty() {
            if head.is_empty() {
                continue;
            }
            break;
        }
        head.push(l);
        if head.len() > 64 {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
    }
    let n = head.iter().find_map(|h| h.split_once(':').filter(|(k, _)| k.trim().eq_ignore_ascii_case("content-length")).and_then(|(_, v)| v.trim().parse::<usize>().ok())).unwrap_or(0);
    let mut body = vec![0u8; n.min(1 << 20)];
    r.read_exact(&mut body)?;
    Ok(Unit::Msg(Msg { head, body }))
}

fn header<'a>(m: &'a Msg, name: &str) -> Option<&'a str> {
    m.head.iter().skip(1).find_map(|h| h.split_once(':').filter(|(k, _)| k.trim().eq_ignore_ascii_case(name)).map(|(_, v)| v.trim()))
}

fn encode(m: &Msg) -> Vec<u8> {
    let mut v = m.head.join("\r\n").into_bytes();
    v.extend_from_slice(b"\r\n\r\n");
    v.extend_from_slice(&m.body);
    v
}

/// Start a relay to the camera; returns the local port for ffmpeg.
pub fn start(cam_host: &str, cam_port: u16, user: &str, pass: &str) -> std::io::Result<u16> {
    let l = TcpListener::bind("127.0.0.1:0")?;
    let port = l.local_addr()?.port();
    let (host, user, pass) = (cam_host.to_string(), user.to_string(), pass.to_string());
    std::thread::Builder::new().name("hone-rtsp-relay".into()).spawn(move || {
        let _ = l.set_nonblocking(false);
        // one session: the first local connection
        let Ok((client, peer)) = l.accept() else { return };
        drop(l);
        if !peer.ip().is_loopback() {
            return;
        }
        let Ok(cam) = TcpStream::connect_timeout(&std::net::SocketAddr::new(match host.parse() { Ok(ip) => ip, Err(_) => return }, cam_port), Duration::from_secs(10)) else {
            let _ = client.shutdown(Shutdown::Both);
            return;
        };
        let _ = cam.set_nodelay(true);
        let local = format!("rtsp://127.0.0.1:{port}");
        let remote = format!("rtsp://{host}:{cam_port}");
        let auth = Arc::new(Mutex::new(Auth::new(&user, &pass)));
        let pending: Arc<Mutex<HashMap<String, Msg>>> = Arc::new(Mutex::new(HashMap::new()));
        let cam_w = Arc::new(Mutex::new(cam.try_clone().unwrap()));
        let mut client_w = client.try_clone().unwrap();
        // ffmpeg -> camera
        {
            let (auth, pending, cam_w, local, remote) = (auth.clone(), pending.clone(), cam_w.clone(), local.clone(), remote.clone());
            let mut r = BufReader::new(client.try_clone().unwrap());
            let cam2 = cam.try_clone().unwrap();
            std::thread::spawn(move || {
                while let Ok(u) = read_unit(&mut r) {
                    let bytes = match u {
                        Unit::Frame(f) => f,
                        Unit::Msg(mut m) => {
                            let mut parts: Vec<String> = m.head[0].splitn(3, ' ').map(String::from).collect();
                            if parts.len() == 3 {
                                parts[1] = parts[1].replacen(&local, &remote, 1);
                                m.head[0] = parts.join(" ");
                            }
                            m.head.retain(|h| !h.to_lowercase().starts_with("authorization:"));
                            let (method, uri) = (parts.first().cloned().unwrap_or_default(), parts.get(1).cloned().unwrap_or_default());
                            if let Some(a) = auth.lock().unwrap().header(&method, &uri) {
                                m.head.push(format!("Authorization: {a}"));
                            }
                            let b = encode(&m);
                            if let Some(c) = header(&m, "cseq").map(String::from) {
                                pending.lock().unwrap().insert(c, m);
                            }
                            b
                        }
                    };
                    if cam_w.lock().unwrap().write_all(&bytes).is_err() {
                        break;
                    }
                }
                let _ = cam2.shutdown(Shutdown::Both);
            });
        }
        // camera -> ffmpeg (answers 401 challenges by retrying with credentials)
        let mut r = BufReader::new(cam);
        while let Ok(u) = read_unit(&mut r) {
            let out = match u {
                Unit::Frame(f) => f,
                Unit::Msg(mut m) => {
                    let code = m.head[0].split(' ').nth(1).unwrap_or("").to_string();
                    let cseq = header(&m, "cseq").map(String::from).unwrap_or_default();
                    if code == "401" {
                        let www: Vec<String> = m.head.iter().skip(1).filter_map(|h| h.split_once(':').filter(|(k, _)| k.trim().eq_ignore_ascii_case("www-authenticate")).map(|(_, v)| v.trim().to_string())).collect();
                        let req = pending.lock().unwrap().remove(&cseq);
                        if let Some(mut req) = req {
                            let mut a = auth.lock().unwrap();
                            if a.accept(&www) {
                                let parts: Vec<&str> = req.head[0].splitn(3, ' ').collect();
                                let (method, uri) = (parts[0].to_string(), parts.get(1).copied().unwrap_or("").to_string());
                                req.head.retain(|h| !h.to_lowercase().starts_with("authorization:"));
                                if let Some(h) = a.header(&method, &uri) {
                                    req.head.push(format!("Authorization: {h}"));
                                }
                                drop(a);
                                let b = encode(&req);
                                pending.lock().unwrap().insert(cseq.clone(), req);
                                if cam_w.lock().unwrap().write_all(&b).is_err() {
                                    break;
                                }
                                continue; // ffmpeg never sees the challenge
                            }
                        }
                    } else {
                        pending.lock().unwrap().remove(&cseq);
                    }
                    // keep ffmpeg's view on the relay address
                    for h in m.head.iter_mut().skip(1) {
                        if let Some((k, v)) = h.split_once(':') {
                            if k.trim().eq_ignore_ascii_case("content-base") || k.trim().eq_ignore_ascii_case("content-location") {
                                *h = format!("{}: {}", k.trim(), v.trim().replacen(&remote, &local, 1));
                            }
                        }
                    }
                    encode(&m)
                }
            };
            if client_w.write_all(&out).is_err() {
                break;
            }
        }
        let _ = client_w.shutdown(Shutdown::Both);
    })?;
    Ok(port)
}

#[cfg(test)]
mod tests {
    use crate::rtsp::RtspAudioClient;
    use md5::{Digest, Md5};
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    fn h(s: &str) -> String {
        hex::encode(Md5::digest(s.as_bytes()))
    }

    /// Fake ESEE-like camera: Digest only, video + PCMA audio, records the SETUP URIs.
    fn fake_camera(user: &'static str, pass: &'static str) -> (u16, Arc<Mutex<Vec<String>>>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let setups = Arc::new(Mutex::new(Vec::new()));
        let s2 = setups.clone();
        std::thread::spawn(move || {
            let (s, _) = l.accept().unwrap();
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut w = s;
            loop {
                let mut head = Vec::new();
                loop {
                    let mut line = String::new();
                    if r.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    let t = line.trim_end().to_string();
                    if t.is_empty() {
                        break;
                    }
                    head.push(t);
                }
                let (method, uri) = { let p: Vec<&str> = head[0].split(' ').collect(); (p[0].to_string(), p[1].to_string()) };
                let cseq = head.iter().find(|x| x.to_lowercase().starts_with("cseq")).unwrap().split(':').nth(1).unwrap().trim().to_string();
                let auth = head.iter().find(|x| x.to_lowercase().starts_with("authorization:")).cloned().unwrap_or_default();
                let resp = regex::Regex::new(r#"response="([0-9a-f]+)""#).unwrap().captures(&auth).map(|c| c[1].to_string());
                let ok = resp == Some(h(&format!("{}:abc:{}", h(&format!("{user}:cam:{pass}")), h(&format!("{method}:{uri}")))));
                if !ok {
                    write!(w, "RTSP/1.0 401 Unauthorized\r\nCSeq: {cseq}\r\nWWW-Authenticate: Digest realm=\"cam\", nonce=\"abc\"\r\n\r\n").unwrap();
                    continue;
                }
                match method.as_str() {
                    "DESCRIBE" => {
                        let sdp = "v=0\r\ns=x\r\na=control:*\r\nm=video 0 RTP/AVP 96\r\na=rtpmap:96 H264/90000\r\na=control:trackID=0\r\nm=audio 0 RTP/AVP 8\r\na=rtpmap:8 PCMA/8000\r\na=control:trackID=1\r\n";
                        write!(w, "RTSP/1.0 200 OK\r\nCSeq: {cseq}\r\nContent-Base: rtsp://127.0.0.1:{port}/ch0_0.264/\r\nContent-Length: {}\r\n\r\n{sdp}", sdp.len()).unwrap();
                    }
                    "SETUP" => {
                        s2.lock().unwrap().push(uri.clone());
                        write!(w, "RTSP/1.0 200 OK\r\nCSeq: {cseq}\r\nSession: 42;timeout=60\r\nTransport: RTP/AVP/TCP;unicast;interleaved=0-1\r\n\r\n").unwrap();
                    }
                    "PLAY" => {
                        write!(w, "RTSP/1.0 200 OK\r\nCSeq: {cseq}\r\nSession: 42\r\n\r\n").unwrap();
                        let mut pkt = vec![0x80u8, 8, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1];
                        pkt.extend(vec![0xD5u8; 160]);
                        for _ in 0..20 {
                            let mut f = vec![b'$', 0, (pkt.len() >> 8) as u8, pkt.len() as u8];
                            f.extend(&pkt);
                            if w.write_all(&f).is_err() {
                                return;
                            }
                        }
                    }
                    _ => write!(w, "RTSP/1.0 200 OK\r\nCSeq: {cseq}\r\n\r\n").unwrap(),
                }
            }
        });
        (port, setups)
    }

    #[test]
    fn relay_adds_digest_and_only_audio_is_set_up() {
        let (cam_port, setups) = fake_camera("admin", "s3cret!");
        let lp = super::start("127.0.0.1", cam_port, "admin", "s3cret!").unwrap();
        // the client (standing in for ffmpeg) has NO credentials
        let mut c = RtspAudioClient::new("127.0.0.1", lp, "/ch0_0.264", None, None, 5000);
        let m = c.open().expect("open through relay");
        assert_eq!((m.encoding.as_str(), m.clock_rate), ("PCMA", 8000));
        let mut n = 0;
        let _ = c.stream_for(0.5, |p| n += p.len());
        assert!(n >= 160, "audio arrived: {n}");
        let s = setups.lock().unwrap().clone();
        assert_eq!(s.len(), 1, "only one track set up: {s:?}");
        assert!(s[0].ends_with("trackID=1"), "the audio track: {s:?}");
        assert!(s[0].starts_with(&format!("rtsp://127.0.0.1:{cam_port}/")), "URI rewritten to the camera: {s:?}");
    }

    #[test]
    fn wrong_password_is_not_forwarded_as_success() {
        let (cam_port, _) = fake_camera("admin", "right");
        let lp = super::start("127.0.0.1", cam_port, "admin", "wrong").unwrap();
        let mut c = RtspAudioClient::new("127.0.0.1", lp, "/ch0_0.264", None, None, 3000);
        assert!(c.open().is_err());
    }
}
