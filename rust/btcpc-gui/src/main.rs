mod api;
mod app;
mod panels;

fn main() {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("BTCPC")
            .with_inner_size([1200.0, 800.0]),
        ..Default::default()
    };
    eframe::run_native(
        "BTCPC",
        options,
        Box::new(|cc| Ok(Box::new(app::BtcpcApp::new(cc)))),
    )
    .expect("failed to run app");
}
