//! `wiiv` — CLI for HONE's rendering platform.
//!
//! Skeleton. Today it can print protocol info and probe the local node. Job
//! posting is intentionally NOT wired: the off-chain MCP render layer (src/mcp/)
//! drives dry-run planning, and live chain routes land behind scoped auth per
//! docs/WIIV_PROTOCOL.md. This binary exists so the crate has a runnable entry
//! point matching the other HONE service crates.

use anyhow::Result;
use wiiv::{Capability, RenderModality};

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("help");

    match cmd {
        "info" => print_info(),
        "modalities" => print_modalities(),
        "node" => probe_node().await?,
        "worker" => worker_cmd(&args[2..]),
        _ => print_help(),
    }
    Ok(())
}

fn print_help() {
    println!(
        "wiiv — HONE rendering platform CLI\n\n\
         Usage:\n  \
         wiiv info               Show protocol summary and safety state\n  \
         wiiv modalities         List supported render modalities\n  \
         wiiv node               Probe the local hone-node (HONE_API)\n  \
         wiiv worker enable <capability>   Opt in to a render capability (dry-run provisioning)\n\n\
         A miner is text-only by default. Rendering models are pulled only when\n\
         you enable the capability, and only if the hardware gate passes.\n\
         Job posting is dry-run only until scoped auth + chain routes exist\n\
         (see docs/WIIV_PROTOCOL.md)."
    );
}

/// Demonstrate the opt-in provisioning flow. Dry-run: uses a stub provider and a
/// simulated hardware profile (real hardware detection + model pull land with the
/// concrete providers). Shows the gate-before-download behavior end to end.
fn worker_cmd(args: &[String]) {
    use wiiv::worker::*;
    use wiiv::{Capability, DeliverableKind};

    let sub = args.first().map(|s| s.as_str()).unwrap_or("help");
    if sub != "enable" {
        println!("usage: wiiv worker enable <capability>   (e.g. video_generation)");
        return;
    }
    let cap = args.get(1).map(|s| s.as_str()).unwrap_or("");
    if cap != "video_generation" {
        println!("only 'video_generation' has a stub provider in this build; got '{cap}'");
        return;
    }

    // A stub provider standing in for a real video model+runtime.
    struct StubVideo(ProviderManifest);
    impl RenderProvider for StubVideo {
        fn manifest(&self) -> &ProviderManifest {
            &self.0
        }
        fn ensure_models(&self) -> Result<u64, String> {
            // Real provider fetches from HONE-FS/URL and verifies sha256.
            Ok(self.0.total_download_mb())
        }
        fn self_test(&self) -> Result<(), String> {
            Ok(())
        }
    }

    let manifest = ProviderManifest {
        capability: Capability::VideoGeneration,
        provider_id: "stub-video-v0".to_string(),
        version: "0.1.0".to_string(),
        models: vec![ModelArtifact {
            cid: Some("bafyvideofake".to_string()),
            url: None,
            sha256: "deadbeef".to_string(),
            size_mb: 6000,
            path: "video/model.gguf".to_string(),
        }],
        runtime: vec![RuntimeDep { name: "ffmpeg".into(), version: "6".into() }],
        min_hardware: MinHardware { vram_mb: 12000, ram_mb: 16000, disk_mb: 20000, requires_gpu: true },
        output_kinds: vec![DeliverableKind::GeneratedScene, DeliverableKind::FinalRender],
    };

    // Simulated local hardware — real builds detect this.
    let hw = Hardware { vram_mb: 24000, ram_mb: 32000, free_disk_mb: 100_000, has_gpu: true };

    println!("enabling video_generation (provider {})", manifest.provider_id);
    println!("  would pull ~{} MB of models after the gate passes", manifest.total_download_mb());
    let result = provision_capability(&StubVideo(manifest), &hw);
    println!("  provision state: {:?}", result.state);
    for note in &result.notes {
        println!("  - {note}");
    }
    if matches!(result.state, ProvisionState::Ready) {
        println!("  → would emit WiivWorkerRegister advertising video_generation");
    }
}

fn print_info() {
    println!("Wiiv — HONE decentralized rendering platform");
    println!("Reserved accounts: {}, {}", wiiv::ACCOUNT_WIIV, wiiv::ACCOUNT_WIIV_ESCROW);
    println!("Schema version: {}", wiiv::SCHEMA_VERSION);
    println!("Live posting: DISABLED (dry-run only until scoped auth + chain routes)");
}

fn print_modalities() {
    for m in [
        RenderModality::Image,
        RenderModality::Video,
        RenderModality::Audio,
        RenderModality::Threed,
        RenderModality::Composite,
    ] {
        println!("{:?}", m);
    }
    // Touch Capability so the type is exercised in the binary too.
    let _caps = [Capability::ImageGeneration, Capability::VideoGeneration];
}

async fn probe_node() -> Result<()> {
    let client = wiiv::api::Client::from_env();
    match client.node_info().await {
        Ok(info) => {
            let chain = info.get("chain_id").and_then(|v| v.as_str()).unwrap_or("?");
            let epoch = info.get("epoch").and_then(|v| v.as_u64()).unwrap_or(0);
            println!("node ok — chain_id={chain} epoch={epoch}");
            println!("signing identity: {}", if client.has_signing_identity() { "present" } else { "none (read-only)" });
        }
        Err(e) => println!("node probe failed: {e}"),
    }
    Ok(())
}
