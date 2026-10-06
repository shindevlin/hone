mod api;
mod app;
mod panels;
mod sign;

fn main() {
    // UFL 3.4 Decentralized acceptance gate (UFL Section 9): nothing runs
    // until the license is accepted. See docs/LICENSE_ACCEPTANCE.md.
    hone_license::require_acceptance_or_exit(&hone_license::default_data_dir());

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("HONE")
            .with_inner_size([1200.0, 800.0]),
        ..Default::default()
    };
    eframe::run_native(
        "HONE",
        options,
        Box::new(|cc| Ok(Box::new(app::HoneApp::new(cc)))),
    )
    .expect("failed to run app");
}
