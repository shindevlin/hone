//! Microphones on this computer as listening sources, through ffmpeg:
//! Windows DirectShow, macOS AVFoundation, Linux ALSA. Same pipeline as a camera: audio only,
//! RAM ring, speech gate, Perch; nothing is recorded.

use serde::Serialize;
use std::process::{Command, Stdio};

#[derive(Serialize, Clone, Debug)]
pub struct MicDevice {
    /// What ffmpeg opens (DirectShow name, AVFoundation index, ALSA plughw:C,D).
    pub spec: String,
    pub name: String,
}

pub const RATE: u32 = 32_000;

fn ffmpeg_stderr(args: &[&str]) -> String {
    let mut c = Command::new(crate::pipeline::ffmpeg_bin());
    c.args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    c.output().map(|o| String::from_utf8_lossy(&o.stderr).into_owned()).unwrap_or_default()
}

/// DirectShow: lines like `[dshow @ ..] "Microphone (Realtek(R) Audio)" (audio)`.
pub fn parse_dshow(s: &str) -> Vec<MicDevice> {
    let re = regex::Regex::new(r#""([^"]+)"\s*\(audio\)"#).unwrap();
    re.captures_iter(s).map(|c| MicDevice { spec: c[1].to_string(), name: c[1].to_string() }).collect()
}

/// AVFoundation: the lines after "AVFoundation audio devices:", `[..] [0] MacBook Pro Microphone`.
pub fn parse_avfoundation(s: &str) -> Vec<MicDevice> {
    let re = regex::Regex::new(r"\]\s*\[(\d+)\]\s*(.+)$").unwrap();
    s.split("AVFoundation audio devices:").nth(1).unwrap_or("").lines()
        .filter_map(|l| re.captures(l.trim()).map(|c| MicDevice { spec: format!(":{}", &c[1]), name: c[2].trim().to_string() }))
        .collect()
}

/// /proc/asound/pcm: `01-00: USB Audio : USB Audio : capture 1`.
pub fn parse_asound(pcm: &str, cards: &str) -> Vec<MicDevice> {
    let card_name = |n: u32| -> Option<String> {
        cards.lines().find_map(|l| {
            let t = l.trim_start();
            let (num, rest) = t.split_once(' ')?;
            (num.parse::<u32>().ok()? == n).then(|| rest.split(" - ").nth(1).unwrap_or(rest).trim().to_string())
        })
    };
    pcm.lines().filter(|l| l.contains("capture")).filter_map(|l| {
        let (id, rest) = l.split_once(':')?;
        let (c, d) = id.split_once('-')?;
        let (c, d): (u32, u32) = (c.trim().parse().ok()?, d.trim().parse().ok()?);
        let pname = rest.split(':').next().unwrap_or("").trim().to_string();
        Some(MicDevice { spec: format!("plughw:{c},{d}"), name: card_name(c).unwrap_or(pname) })
    }).collect()
}

pub fn list() -> Vec<MicDevice> {
    if cfg!(windows) {
        parse_dshow(&ffmpeg_stderr(&["-hide_banner", "-list_devices", "true", "-f", "dshow", "-i", "dummy"]))
    } else if cfg!(target_os = "macos") {
        parse_avfoundation(&ffmpeg_stderr(&["-hide_banner", "-f", "avfoundation", "-list_devices", "true", "-i", ""]))
    } else {
        parse_asound(&std::fs::read_to_string("/proc/asound/pcm").unwrap_or_default(), &std::fs::read_to_string("/proc/asound/cards").unwrap_or_default())
    }
}

/// ffmpeg input arguments for a microphone.
pub fn input_args(spec: &str) -> Vec<String> {
    let (f, i) = if cfg!(windows) {
        ("dshow", format!("audio={spec}"))
    } else if cfg!(target_os = "macos") {
        ("avfoundation", spec.to_string())
    } else {
        ("alsa", spec.to_string())
    };
    vec!["-f".into(), f.into(), "-i".into(), i]
}

/// Listen ~4 s to rate the microphone (background level). Audio stays in memory and is dropped.
pub fn probe(spec: &str) -> Result<Option<f64>, String> {
    let mut a: Vec<String> = ["-nostdin", "-hide_banner", "-loglevel", "error"].iter().map(|s| s.to_string()).collect();
    a.extend(input_args(spec));
    a.extend(["-t", "4", "-ac", "1", "-ar", "32000", "-f", "f32le", "pipe:1"].iter().map(|s| s.to_string()));
    let mut c = Command::new(crate::pipeline::ffmpeg_bin());
    c.args(&a).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    let o = c.output().map_err(|e| format!("ffmpeg: {e}"))?;
    let mut x: Vec<f32> = o.stdout.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
    if x.len() < RATE as usize {
        return Err("We couldn't hear anything from this microphone. Check that it's plugged in and allowed in your privacy settings.".into());
    }
    let nf = crate::audio::noise_floor_db(&x, RATE);
    x.iter_mut().for_each(|v| *v = 0.0);
    Ok(nf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_device_lists() {
        let d = parse_dshow("[dshow @ 0x1] \"HD Webcam\" (video)\n[dshow @ 0x1] \"Microphone (Realtek(R) Audio)\" (audio)\n[dshow @ 0x1]   Alternative name \"@device_cm_{33D9}\"");
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].spec, "Microphone (Realtek(R) Audio)");
        let m = parse_avfoundation("AVFoundation video devices:\n[x] [0] FaceTime HD Camera\nAVFoundation audio devices:\n[AVFoundation indev @ 0x7f] [0] MacBook Pro Microphone\n[AVFoundation indev @ 0x7f] [1] USB Mic");
        assert_eq!(m.iter().map(|d| d.spec.as_str()).collect::<Vec<_>>(), vec![":0", ":1"]);
        assert_eq!(m[0].name, "MacBook Pro Microphone");
        let a = parse_asound("00-00: bcm2835 : bcm2835 : playback 8\n01-00: USB Audio : USB Audio : playback 1 : capture 1\n",
            " 0 [Headphones     ]: bcm2835_headpho - bcm2835 Headphones\n 1 [Device         ]: USB-Audio - USB PnP Sound Device\n");
        assert_eq!(a.len(), 1);
        assert_eq!((a[0].spec.as_str(), a[0].name.as_str()), ("plughw:1,0", "USB PnP Sound Device"));
    }
}
