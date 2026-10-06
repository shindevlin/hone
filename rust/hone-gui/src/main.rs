mod api;
mod app;
mod panels;
mod sign;

fn main() {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("HONE")
            .with_inner_size([1200.0, 800.0]),
        ..Default::default()
    };
    // UFL 3.4 Decentralized acceptance gate (UFL Section 9): a first-run
    // license screen; HoneApp is not built until the license is accepted.
    // See docs/LICENSE_ACCEPTANCE.md.
    hone_license_egui::run_native("HONE", options, |ctx| Box::new(app::HoneApp::from_ctx(ctx)))
        .expect("failed to run app");
}
