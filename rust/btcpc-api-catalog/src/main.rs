use btcpc_api_catalog::{
    default_http_client, load_public_apis_from_checkout, verify_snapshot_sequential, CatalogError,
};
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), CatalogError> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 || args[1] != "snapshot" {
        print_usage(&args[0]);
        return Ok(());
    }

    let checkout = PathBuf::from(&args[2]);
    let out = args.get(3).filter(|arg| !arg.starts_with("--"));
    let verify = args.iter().any(|arg| arg == "--verify");
    let limit = parse_limit(&args);

    let mut snapshot = load_public_apis_from_checkout(&checkout)?;
    if verify {
        let client = default_http_client().expect("failed to construct HTTP client");
        verify_snapshot_sequential(&mut snapshot, &client, limit).await;
    }

    let hash = snapshot.content_hash()?;
    if let Some(out) = out {
        snapshot.save_json(out)?;
        eprintln!("wrote {out}");
        eprintln!("records={} content_hash={hash}", snapshot.records.len());
    } else {
        println!("{}", serde_json::to_string_pretty(&snapshot)?);
        eprintln!("records={} content_hash={hash}", snapshot.records.len());
    }

    Ok(())
}

fn parse_limit(args: &[String]) -> Option<usize> {
    args.windows(2)
        .find(|pair| pair[0] == "--limit")
        .and_then(|pair| pair[1].parse::<usize>().ok())
}

fn print_usage(binary: &str) {
    eprintln!("Usage:");
    eprintln!("  {binary} snapshot <public-apis-checkout> [out.json] [--verify] [--limit N]");
    eprintln!();
    eprintln!("Example:");
    eprintln!("  {binary} snapshot /tmp/public-apis catalog.json --verify --limit 100");
}
