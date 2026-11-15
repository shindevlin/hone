//! First-run UFL 3.8 Decentralized license screen for HONE's egui apps.
//!
//! GUI apps are usually started from a launcher, with no terminal to prompt
//! on, so they use this screen instead of the terminal prompt in
//! `hone-license`. The rules are the same:
//!
//! - A matching record, `--accept-ufl 3.8`, or `HONE_ACCEPT_LICENSE=UFL-3.8-D`:
//!   the app starts directly.
//! - Otherwise the window shows the license summary (version, scope, ICC
//!   arbitration, the US$1 limit, own-risk terms) with the full text one
//!   click away, and two buttons: "I accept" and "Decline and quit".
//! - The real app is not constructed until "I accept" is clicked, so none of
//!   its code runs before acceptance. Declining exits with status 3.
//! - A wrong value of the flag or variable exits with status 3.

use std::path::PathBuf;

use eframe::egui;
use hone_license::{GateError, Mode};

/// Builds the real app once the license is accepted.
pub type MakeApp = Box<dyn FnOnce(&egui::Context) -> Box<dyn eframe::App>>;

/// Like `eframe::run_native`, but behind the license gate. `make` builds the
/// real app from the egui context; it runs only after acceptance.
pub fn run_native(
    app_name: &str,
    options: eframe::NativeOptions,
    make: impl FnOnce(&egui::Context) -> Box<dyn eframe::App> + 'static,
) -> eframe::Result {
    let data_dir = hone_license::default_data_dir();
    let accepted = match hone_license::require_acceptance(&data_dir, Mode::Headless) {
        Ok(_) => true,
        Err(GateError::NotAccepted { .. }) => false,
        Err(e) => {
            eprintln!("{}: {e}", hone_license::current_binary());
            std::process::exit(hone_license::EXIT_NOT_ACCEPTED);
        }
    };
    eframe::run_native(
        app_name,
        options,
        Box::new(move |cc| {
            let app: Box<dyn eframe::App> = if accepted {
                make(&cc.egui_ctx)
            } else {
                Box::new(LicenseGate::new(data_dir, Box::new(make)))
            };
            Ok(app)
        }),
    )
}

/// The gate screen; becomes the real app after acceptance.
pub struct LicenseGate {
    data_dir: PathBuf,
    make: Option<MakeApp>,
    app: Option<Box<dyn eframe::App>>,
    show_full: bool,
}

impl LicenseGate {
    pub fn new(data_dir: PathBuf, make: MakeApp) -> Self {
        Self { data_dir, make: Some(make), app: None, show_full: false }
    }

    fn accept(&mut self, ctx: &egui::Context) {
        let path = hone_license::record_path(&self.data_dir);
        if let Err(e) =
            hone_license::write_record(&path, &hone_license::current_binary(), &hone_license::now_utc())
        {
            // Acceptance stands for this run; it just could not be saved.
            eprintln!(
                "{}: accepted {}, but could not save the record at {} ({e}); you will be asked again next run",
                hone_license::current_binary(),
                hone_license::RULING_LICENSE,
                path.display()
            );
        }
        if let Some(make) = self.make.take() {
            self.app = Some(make(ctx));
        }
    }

    fn gate_ui(&mut self, ui: &mut egui::Ui) {
        egui::Frame::default().inner_margin(16.0).show(ui, |ui| self.gate_body(ui));
    }

    fn gate_body(&mut self, ui: &mut egui::Ui) {
        ui.heading("HONE license");
        ui.label(format!(
            "Before HONE starts, read and accept its license: {} ({}).",
            hone_license::RULING_LICENSE,
            hone_license::SPDX
        ));
        ui.add_space(8.0);

        let footer = 72.0;
        let body_height = (ui.available_height() - footer).max(120.0);
        egui::ScrollArea::vertical()
            .max_height(body_height)
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                let text = if self.show_full {
                    hone_license::LICENSE_TEXT.to_string()
                } else {
                    hone_license::summary()
                };
                ui.label(egui::RichText::new(text).monospace());
            });

        ui.add_space(8.0);
        let toggle = if self.show_full { "Show summary" } else { "Show full license" };
        if ui.button(toggle).clicked() {
            self.show_full = !self.show_full;
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.button(egui::RichText::new("I accept").strong()).clicked() {
                self.accept(ui.ctx());
            }
            if ui.button("Decline and quit").clicked() {
                eprintln!(
                    "{}: license not accepted; HONE did not start.",
                    hone_license::current_binary()
                );
                std::process::exit(hone_license::EXIT_NOT_ACCEPTED);
            }
        });
    }
}

impl eframe::App for LicenseGate {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if let Some(app) = &mut self.app {
            app.logic(ctx, frame);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        if let Some(app) = &mut self.app {
            app.ui(ui, frame);
        } else {
            self.gate_ui(ui);
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if let Some(app) = &mut self.app {
            app.save(storage);
        }
    }

    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        if let Some(app) = &mut self.app {
            app.on_exit(gl);
        }
    }
}
