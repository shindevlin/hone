mod wallet_create;

fn main() {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("BTCPC Wallet Create")
            .with_inner_size([980.0, 720.0]),
        ..Default::default()
    };

    eframe::run_native(
        "BTCPC Wallet Create",
        options,
        Box::new(|cc| Ok(Box::new(wallet_create::WalletCreateApp::new(cc)))),
    )
    .expect("failed to run BTCPC wallet creation app");
}
