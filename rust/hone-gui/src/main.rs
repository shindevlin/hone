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
    eframe::run_native(
        "HONE",
        options,
        Box::new(|cc| Ok(Box::new(app::HoneApp::new(cc)))),
    )
    .expect("failed to run app");
}
