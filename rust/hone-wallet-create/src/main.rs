mod wallet_create;

fn main() {
    // UFL 3.4 Decentralized acceptance gate (UFL Section 9): nothing runs
    // until the license is accepted. See docs/LICENSE_ACCEPTANCE.md.
    hone_license::require_acceptance_or_exit(&hone_license::default_data_dir());

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("HONE Wallet Create")
            .with_inner_size([980.0, 720.0]),
        ..Default::default()
    };

    eframe::run_native(
        "HONE Wallet Create",
        options,
        Box::new(|cc| Ok(Box::new(wallet_create::WalletCreateApp::new(cc)))),
    )
    .expect("failed to run HONE wallet creation app");
}
