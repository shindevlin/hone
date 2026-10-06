mod wallet_create;

fn main() {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("HONE Wallet Create")
            .with_inner_size([980.0, 720.0]),
        ..Default::default()
    };

    // UFL 3.4 Decentralized acceptance gate (UFL Section 9): a first-run
    // license screen; the app is not built until the license is accepted.
    // See docs/LICENSE_ACCEPTANCE.md.
    hone_license_egui::run_native("HONE Wallet Create", options, |ctx| {
        Box::new(wallet_create::WalletCreateApp::from_ctx(ctx))
    })
    .expect("failed to run HONE wallet creation app");
}
