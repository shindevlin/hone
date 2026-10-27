//! Every surface that shows the license terms outside Rust (Android app,
//! installers) must show the same summary as `hone_license::summary()`, and
//! every record they write must be accepted by the Rust gate.

use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn assert_has_summary(rel: &str, quote: impl Fn(&str) -> String) {
    let text = std::fs::read_to_string(repo().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"));
    for line in hone_license::summary().lines().filter(|l| !l.trim().is_empty()) {
        assert!(text.contains(&quote(line)), "{rel} is missing summary line: {line:?}");
    }
}

#[test]
fn android_summary_matches() {
    assert_has_summary("android/app/src/main/java/network/hone/app/LicenseGate.java", |l| {
        format!("\"{}\\n\"", l.replace('\\', "\\\\"))
    });
}

#[test]
fn installer_summaries_match() {
    for rel in ["website/install.sh", "scripts/install.sh"] {
        assert_has_summary(rel, |l| format!("\n{l}\n"));
    }
}

#[test]
fn records_from_other_surfaces_are_accepted() {
    let dir = std::env::temp_dir().join(format!("hone-license-surfaces-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = hone_license::record_path(&dir);
    // Shapes written by the installers, the Android app (JSONObject.toString(2)),
    // and the Flipper apps (device-clock time, no Z).
    for body in [
        "{\n  \"license\": \"UFL\",\n  \"version\": \"3.8\",\n  \"scope\": \"Decentralized\",\n  \"accepted_at\": \"2026-10-06T14:00:00Z\",\n  \"binary\": \"hone-installer\"\n}\n",
        "{\n  \"license\": \"UFL\",\n  \"version\": \"3.8\",\n  \"scope\": \"Decentralized\",\n  \"accepted_at\": \"2026-10-06T14:00:00Z\",\n  \"binary\": \"hone-android\"\n}\n",
        "{\n  \"license\": \"UFL\",\n  \"version\": \"3.8\",\n  \"scope\": \"Decentralized\",\n  \"accepted_at\": \"2026-10-06T08:00:00\",\n  \"binary\": \"hone_sensor\"\n}\n",
    ] {
        std::fs::write(&path, body).unwrap();
        assert_eq!(hone_license::read_record(&path), hone_license::RecordState::Matching, "{body}");
    }
}
