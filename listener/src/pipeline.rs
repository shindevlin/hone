//! camera audio -> ffmpeg (audio track only) -> resample to 32 kHz -> RAM ring buffer ->
//! voice gate -> Perch v2 (+ its human classes as a second gate) -> voiceprint hook ->
//! 15-minute window -> signed report (report.rs) -> buffer zeroed.
//! No audio is ever written to disk or sent anywhere. Frames with a voice are dropped and
//! only counted.

use crate::audio::LinearResampler;
use crate::config::{now_ms, res_dir, Config, Station};
use crate::perch::{self, Perch};
use crate::voiceprint::{NoopVoiceprint, VoiceprintSink};
use crate::{live, report, secrets, voicegate};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::time::Duration;

pub const WINDOW_MS: u64 = 900_000;
const QUEUE_CAP: usize = 3;

pub struct Frame {
    pub station: String,
    pub tier: String,
    pub t_ms: u64,
    pub samples: Vec<f32>,
}

impl Drop for Frame {
    fn drop(&mut self) {
        self.samples.iter_mut().for_each(|s| *s = 0.0);
    }
}

/// Bounded frame queue: when Perch falls behind the oldest frame is dropped (and counted),
/// so a slow box degrades instead of piling up audio in memory.
#[derive(Default)]
pub struct FrameQueue {
    q: Mutex<VecDeque<Frame>>,
    cv: Condvar,
}

impl FrameQueue {
    pub fn push(&self, f: Frame) {
        let mut q = self.q.lock().unwrap();
        while q.len() >= QUEUE_CAP {
            if let Some(old) = q.pop_front() {
                live::station(&old.station, |s| s.busy_dropped += 1);
            }
        }
        q.push_back(f);
        self.cv.notify_one();
    }
    fn pop(&self, timeout: Duration) -> Option<Frame> {
        let q = self.q.lock().unwrap();
        let (mut q, _) = self.cv.wait_timeout_while(q, timeout, |q| q.is_empty()).unwrap();
        q.pop_front()
    }
}

/// Fixed RAM ring of one Perch frame. Full -> handed off as a frame and zeroed.
pub struct Ring {
    buf: Vec<f32>,
    fill: usize,
}

impl Default for Ring {
    fn default() -> Self {
        Self { buf: vec![0.0; perch::FRAME], fill: 0 }
    }
}

impl Ring {
    pub fn push(&mut self, mut x: &[f32], mut emit: impl FnMut(Vec<f32>)) {
        while !x.is_empty() {
            let n = x.len().min(self.buf.len() - self.fill);
            self.buf[self.fill..self.fill + n].copy_from_slice(&x[..n]);
            self.fill += n;
            x = &x[n..];
            if self.fill == self.buf.len() {
                emit(self.buf.clone());
                self.zero();
            }
        }
    }
    pub fn zero(&mut self) {
        self.buf.iter_mut().for_each(|s| *s = 0.0);
        self.fill = 0;
    }
}

impl Drop for Ring {
    fn drop(&mut self) {
        self.zero();
    }
}

// ── ingest ───────────────────────────────────────────────────────────────────

fn ffmpeg_bin() -> std::path::PathBuf {
    let name = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    let local = res_dir().join(name);
    if local.exists() {
        return local;
    }
    #[cfg(target_os = "macos")]
    for p in ["/opt/homebrew/bin/ffmpeg", "/usr/local/bin/ffmpeg"] {
        if std::path::Path::new(p).exists() {
            return p.into();
        }
    }
    name.into()
}

fn enc(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}

/// ffmpeg arguments: RTSP over TCP, ONLY the audio media is set up
/// (`-allowed_media_types audio`, so the camera is never asked for video), first audio
/// stream mapped, video/subtitle/data disabled, mono float PCM to stdout.
pub fn ffmpeg_args(url: &str, rate: u32) -> Vec<String> {
    let mut a: Vec<String> = ["-nostdin", "-hide_banner", "-loglevel", "error"].iter().map(|s| s.to_string()).collect();
    if url.starts_with("rtsp") {
        a.extend(["-rtsp_transport", "tcp", "-allowed_media_types", "audio", "-timeout", "10000000"].iter().map(|s| s.to_string()));
    }
    a.extend(["-i".to_string(), url.to_string()]);
    a.extend(["-map", "0:a:0", "-vn", "-sn", "-dn", "-ac", "1", "-ar"].iter().map(|s| s.to_string()));
    a.push(rate.to_string());
    a.extend(["-f", "f32le", "pipe:1"].iter().map(|s| s.to_string()));
    a
}

pub fn stream_url(st: &Station, creds: Option<&(String, String)>) -> String {
    let path = if st.path.starts_with('/') { st.path.clone() } else { format!("/{}", st.path) };
    match creds {
        Some((u, p)) if !u.is_empty() => format!("rtsp://{}:{}@{}:{}{}", enc(u), enc(p), st.host, st.port, path),
        _ => format!("rtsp://{}:{}{}", st.host, st.port, path),
    }
}

/// Removes anything that looks like credentials from ffmpeg's error output.
pub fn redact(line: &str, secrets: &[&str]) -> String {
    let mut s = regex::Regex::new(r"rtsp://[^@/\s]*@").unwrap().replace_all(line, "rtsp://***@").into_owned();
    for x in secrets.iter().filter(|x| x.len() >= 2) {
        s = s.replace(*x, "***");
        s = s.replace(&enc(x), "***");
    }
    s.chars().take(200).collect()
}

fn st_ref(s: &Station) -> &Station {
    s
}

pub fn run_station(st: Station, stop: Arc<AtomicBool>, q: Arc<FrameQueue>) {
    let mut failures = 0u32;
    while !stop.load(Ordering::Relaxed) {
        let creds = secrets::camera_creds(&st.id);
        // The login never goes on ffmpeg's command line: a loopback relay adds it (relay.rs).
        let url = match creds.as_ref().filter(|(u, _)| !u.is_empty()) {
            Some((u, p)) => match crate::relay::start(&st.host, st.port, u, p) {
                Ok(lp) => format!("rtsp://127.0.0.1:{lp}{}", if st.path.starts_with('/') { st.path.clone() } else { format!("/{}", st.path) }),
                Err(e) => {
                    log::warn!("relay: {e}");
                    stream_url(st_ref(&st), None)
                }
            },
            None => stream_url(st_ref(&st), None),
        };
        let secret_parts: Vec<String> = creds.iter().flat_map(|(u, p)| [u.clone(), p.clone()]).collect();
        let sp: Vec<&str> = secret_parts.iter().map(|s| s.as_str()).collect();
        let started = std::time::Instant::now();
        let res = ingest_once(&st, &url, &stop, &q, &sp);
        if stop.load(Ordering::Relaxed) {
            break;
        }
        if started.elapsed() > Duration::from_secs(120) {
            failures = 0;
        }
        failures += 1;
        let msg = res.err().unwrap_or_else(|| "camera stream ended".into());
        log::warn!("station {}: {}", &st.id[..6], msg);
        live::station(&st.id, |s| {
            s.running = false;
            s.reachable = Some(!msg.contains("unreachable") && !msg.contains("Connection refused") && !msg.contains("timed out"));
            s.last_error = Some(msg.clone());
        });
        let wait = crate::rtsp::backoff_ms(failures);
        let mut waited = 0;
        while waited < wait && !stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(500));
            waited += 500;
        }
    }
    live::station(&st.id, |s| s.running = false);
}

/// Offline check: run a local audio file through the same ffmpeg -> resample -> ring -> gates ->
/// Perch path, at the given camera rate (8000 = G.711 camera). Returns the window report payload.
pub fn analyze_file(path: &str, rate: u32) -> Result<serde_json::Value, String> {
    let p = path.to_string();
    std::thread::Builder::new().stack_size(PERCH_STACK).spawn(move || analyze_file_inner(&p, rate)).map_err(|e| e.to_string())?
        .join().map_err(|_| "analysis crashed".to_string())?
}

fn analyze_file_inner(path: &str, rate: u32) -> Result<serde_json::Value, String> {
    let st = Station { id: "0000000000000000".into(), label: "file".into(), host: String::new(), port: 0, path: String::new(), onvif: None,
        codec: "file".into(), rate, tier: crate::audio::tier(rate).into(), id_source: "file".into(), noise_db: None, enabled: true, added_ms: 0 };
    let q = Arc::new(FrameQueue::default());
    let stop = AtomicBool::new(false);
    let (tx, _rx) = mpsc::channel();
    let mut a = Analyzer::new(tx, Box::new(NoopVoiceprint));
    let mut child = Command::new(ffmpeg_bin()).args(ffmpeg_args(path, rate)).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null())
        .spawn().map_err(|e| format!("ffmpeg: {e}"))?;
    let mut out = child.stdout.take().unwrap();
    let mut rs = LinearResampler::new(rate, perch::SAMPLE_RATE);
    let mut ring = Ring::default();
    let mut buf = vec![0u8; 1 << 16];
    let mut carry = Vec::new();
    let t0 = now_ms();
    let mut n = 0u64;
    loop {
        let k = out.read(&mut buf).map_err(|e| e.to_string())?;
        if k == 0 {
            break;
        }
        carry.extend_from_slice(&buf[..k]);
        let whole = carry.len() / 4 * 4;
        let pcm: Vec<f32> = carry[..whole].chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        carry.drain(..whole);
        let x = rs.process(&pcm);
        let mut frames = Vec::new();
        ring.push(&x, |samples| frames.push(samples));
        for samples in frames {
            n += 1;
            a.process(Frame { station: st.id.clone(), tier: st.tier.clone(), t_ms: t0, samples });
        }
    }
    let _ = (q, stop);
    let _ = child.wait();
    let l = live::snapshot();
    if let Some(e) = l.model_error {
        return Err(e);
    }
    let w = a.take_windows();
    let (id, tier, w) = w.into_iter().next().ok_or(format!("no full 5 s frame in the input ({n} frames)"))?;
    let mut p = report::payload(&id, &tier, &w);
    p["detections"] = serde_json::json!(l.recent.iter().rev().map(|d| serde_json::json!({"name": d.name, "conf": d.conf})).collect::<Vec<_>>());
    Ok(p)
}

fn ingest_once(st: &Station, url: &str, stop: &AtomicBool, q: &FrameQueue, sp: &[&str]) -> Result<(), String> {
    let mut cmd = Command::new(ffmpeg_bin());
    cmd.args(ffmpeg_args(url, st.rate)).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child: Child = cmd.spawn().map_err(|e| format!("ffmpeg not found ({e})"))?;
    let stderr = child.stderr.take().unwrap();
    let last_err = Arc::new(Mutex::new(String::new()));
    let le = last_err.clone();
    let owned: Vec<String> = sp.iter().map(|s| s.to_string()).collect();
    std::thread::spawn(move || {
        let refs: Vec<&str> = owned.iter().map(|s| s.as_str()).collect();
        for l in BufReader::new(stderr).lines().map_while(Result::ok) {
            *le.lock().unwrap() = redact(&l, &refs);
        }
    });
    let mut out = child.stdout.take().unwrap();
    let mut rs = LinearResampler::new(st.rate, perch::SAMPLE_RATE);
    let mut ring = Ring::default();
    let mut bytes = vec![0u8; 8192];
    let mut carry: Vec<u8> = Vec::new();
    let mut first = true;
    let result = loop {
        if stop.load(Ordering::Relaxed) {
            break Ok(());
        }
        let n = match out.read(&mut bytes) {
            Ok(0) => break Err(last_err.lock().unwrap().clone()),
            Ok(n) => n,
            Err(e) => break Err(format!("read: {e}")),
        };
        if first {
            first = false;
            live::station(&st.id, |s| {
                s.running = true;
                s.reachable = Some(true);
                s.last_error = None;
            });
        }
        carry.extend_from_slice(&bytes[..n]);
        let whole = carry.len() / 4 * 4;
        let pcm: Vec<f32> = carry[..whole].chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        carry.drain(..whole);
        bytes.iter_mut().for_each(|b| *b = 0);
        let x = rs.process(&pcm);
        ring.push(&x, |samples| q.push(Frame { station: st.id.clone(), tier: st.tier.clone(), t_ms: now_ms(), samples }));
    };
    let _ = child.kill();
    let _ = child.wait();
    ring.zero();
    result.map_err(|e| if e.is_empty() { "camera stream ended".to_string() } else { e })
}

// ── analysis ─────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct Win {
    pub index: u64,
    pub frames: u32,
    pub human: u32,
    pub voice_blocks: u32,
    pub species: BTreeMap<String, (f32, u32)>,
    pub block_db: Vec<f64>,
}

/// TFLite/XNNPACK kernels use large stack frames: Windows' default 1 MB main stack is not
/// enough (access violation), so Perch always runs on a thread with this stack.
pub const PERCH_STACK: usize = 16 << 20;

pub fn strict_voice() -> bool {
    std::env::var("HONE_LISTENER_STRICT_VOICE").map(|v| v == "1").unwrap_or(false)
}

pub struct Analyzer {
    perch: Option<Perch>,
    sink: Box<dyn VoiceprintSink>,
    windows: HashMap<String, (String, Win)>,
    reports: mpsc::Sender<report::Report>,
}

impl Analyzer {
    pub fn new(reports: mpsc::Sender<report::Report>, sink: Box<dyn VoiceprintSink>) -> Self {
        Self { perch: None, sink, windows: HashMap::new(), reports }
    }

    fn ensure_perch(&mut self) -> bool {
        if self.perch.is_none() {
            match Perch::load(&res_dir(), 4) {
                Ok(p) => {
                    self.perch = Some(p);
                    live::set(|l| l.model_error = None);
                }
                Err(e) => {
                    live::set(|l| l.model_error = Some(e.clone()));
                    log::error!("perch: {e}");
                    return false;
                }
            }
        }
        true
    }

    /// Open windows, for the offline `analyze` command.
    pub fn take_windows(&mut self) -> Vec<(String, String, Win)> {
        self.windows.drain().map(|(id, (tier, w))| (id, tier, w)).collect()
    }

    /// Closes windows whose quarter-hour has passed.
    pub fn rollover(&mut self, now: u64) {
        let cur = now / WINDOW_MS;
        let done: Vec<String> = self.windows.iter().filter(|(_, (_, w))| w.index < cur).map(|(k, _)| k.clone()).collect();
        for id in done {
            if let Some((tier, w)) = self.windows.remove(&id) {
                self.close(&id, &tier, w);
            }
        }
    }

    fn close(&mut self, station: &str, tier: &str, w: Win) {
        if w.frames == 0 {
            return;
        }
        match report::build(station, tier, &w) {
            Ok(r) => {
                let _ = self.reports.send(r);
            }
            Err(e) => log::error!("report: {e}"),
        }
    }

    pub fn process(&mut self, f: Frame) {
        let widx = f.t_ms / WINDOW_MS;
        let entry = self.windows.entry(f.station.clone()).or_insert_with(|| (f.tier.clone(), Win { index: widx, ..Default::default() }));
        if entry.1.index != widx {
            let old = std::mem::replace(entry, (f.tier.clone(), Win { index: widx, ..Default::default() }));
            self.close(&f.station, &old.0, old.1);
        }
        // Gate 1: voice gate on the raw frame (the phone's tested gate): every 100 ms block with
        // speech, plus 0.3 s each side, is zeroed before Perch or the noise meter sees it; a frame
        // that is mostly speech is dropped whole. HONE_LISTENER_STRICT_VOICE=1 drops any frame
        // with a speech block instead (safer, but it also drops most owl and dove calls).
        let gate = voicegate::analyze(&f.samples);
        if gate.drop || (strict_voice() && gate.has_voice()) {
            let w = &mut self.windows.get_mut(&f.station).unwrap().1;
            w.frames += 1;
            w.human += 1;
            w.voice_blocks += gate.muted_blocks() as u32;
            live::station(&f.station, |s| {
                s.frames += 1;
                s.voices_removed += 1;
                s.last_frame_ms = f.t_ms;
            });
            return; // Frame dropped here: Drop zeroes it.
        }
        let mut f = f;
        let muted = gate.muted_blocks();
        for (b, m) in gate.mask.iter().enumerate() {
            if *m {
                let end = ((b + 1) * voicegate::BLOCK).min(f.samples.len());
                f.samples[b * voicegate::BLOCK..end].iter_mut().for_each(|v| *v = 0.0);
            }
        }
        if muted > 0 {
            self.windows.get_mut(&f.station).unwrap().1.voice_blocks += muted as u32;
            live::station(&f.station, |s| s.voices_removed += 1);
        }
        if !self.ensure_perch() {
            return;
        }
        let perch = self.perch.as_mut().unwrap();
        let probs = match perch.classify(&f.samples) {
            Ok(p) => p.to_vec(),
            Err(e) => {
                live::set(|l| l.model_error = Some(e));
                return;
            }
        };
        let w = &mut self.windows.get_mut(&f.station).unwrap().1;
        w.frames += 1;
        // Gate 2: Perch's own human classes.
        let human = perch.human_idx.iter().map(|&i| probs[i]).fold(0f32, f32::max);
        if human > perch::HUMAN_MAX {
            w.human += 1;
            live::station(&f.station, |s| {
                s.frames += 1;
                s.voices_removed += 1;
                s.last_frame_ms = f.t_ms;
            });
            return;
        }
        let block = (perch::SAMPLE_RATE / 10) as usize;
        for b in 0..perch::FRAME / block {
            if gate.mask.get(b).copied().unwrap_or(false) {
                continue; // muted speech: not measured
            }
            let e: f64 = f.samples[b * block..(b + 1) * block].iter().map(|&v| (v as f64) * (v as f64)).sum();
            w.block_db.push(10.0 * (e / block as f64 + 1e-12).log10());
        }
        let mut top: Vec<usize> = (0..probs.len()).filter(|&i| perch.is_species[i] && probs[i] >= perch::SPECIES_MIN).collect();
        top.sort_by(|a, b| probs[*b].partial_cmp(&probs[*a]).unwrap());
        top.truncate(5);
        for &i in &top {
            let name = perch.names[i].clone();
            let e = w.species.entry(name.clone()).or_insert((0.0, 0));
            e.0 = e.0.max(probs[i]);
            e.1 += 1;
            self.sink.on_detection(&name, &perch.last_embedding, f.t_ms, probs[i]);
            live::detection(live::Detection { t: f.t_ms, station: f.station.clone(), name, conf: (probs[i] * 100.0).round() as u32 });
        }
        perch.last_embedding.iter_mut().for_each(|v| *v = 0.0);
        live::station(&f.station, |s| {
            s.frames += 1;
            s.birds += top.len() as u64;
            s.last_frame_ms = f.t_ms;
        });
    }
}

pub fn spawn_analyzer(q: Arc<FrameQueue>, reports: mpsc::Sender<report::Report>) {
    std::thread::Builder::new()
        .name("hone-perch".into())
        .stack_size(PERCH_STACK)
        .spawn(move || {
            let mut a = Analyzer::new(reports, Box::new(NoopVoiceprint));
            loop {
                if let Some(f) = q.pop(Duration::from_secs(5)) {
                    a.process(f);
                }
                a.rollover(now_ms());
            }
        })
        .unwrap();
}

// ── engine ───────────────────────────────────────────────────────────────────

pub struct Engine {
    q: Arc<FrameQueue>,
    runners: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl Engine {
    pub fn start(reports: mpsc::Sender<report::Report>) -> Arc<Self> {
        let q = Arc::new(FrameQueue::default());
        spawn_analyzer(q.clone(), reports);
        Arc::new(Self { q, runners: Mutex::new(HashMap::new()) })
    }

    /// Start/stop camera listeners to match the settings.
    pub fn sync(&self, cfg: &Config) {
        let want: HashMap<String, Station> = if cfg.listening && cfg.ready() {
            cfg.stations.iter().filter(|s| s.enabled).map(|s| (s.id.clone(), s.clone())).collect()
        } else {
            HashMap::new()
        };
        let mut r = self.runners.lock().unwrap();
        r.retain(|id, stop| {
            let keep = want.contains_key(id);
            if !keep {
                stop.store(true, Ordering::Relaxed);
            }
            keep
        });
        for (id, st) in want {
            if r.contains_key(&id) {
                continue;
            }
            let stop = Arc::new(AtomicBool::new(false));
            r.insert(id.clone(), stop.clone());
            let q = self.q.clone();
            std::thread::Builder::new().name(format!("hone-cam-{}", &id[..6])).spawn(move || run_station(st, stop, q)).unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ffmpeg_never_asks_for_video() {
        let a = ffmpeg_args("rtsp://192.168.1.5:554/ch0_0.264", 8000).join(" ");
        assert!(a.contains("-allowed_media_types audio"));
        assert!(a.contains("-map 0:a:0") && a.contains("-vn"));
        assert!(a.contains("-ar 8000") && a.ends_with("-f f32le pipe:1"));
    }

    #[test]
    fn redacts_credentials() {
        let l = "rtsp://admin:p%40ss@192.168.1.5:554/x: 401 Unauthorized p@ss";
        let r = redact(l, &["admin", "p@ss"]);
        assert!(!r.contains("p@ss") && !r.contains("p%40ss") && !r.contains("admin"), "{r}");
    }

    #[test]
    fn ring_emits_full_frames_and_zeroes() {
        let mut r = Ring::default();
        let mut got = 0;
        r.push(&vec![0.5; perch::FRAME + 10], |f| {
            assert_eq!(f.len(), perch::FRAME);
            got += 1
        });
        assert_eq!(got, 1);
        assert_eq!(r.fill, 10);
        r.zero();
        assert!(r.buf.iter().all(|v| *v == 0.0));
    }
}
