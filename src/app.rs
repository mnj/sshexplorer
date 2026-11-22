//! The main window: the toolbar along the top and, once one is open, the Sessions as tabs.
//!
//! Launch opens no Session and lists no Hosts. The toolbar's Hosts control and the
//! Settings gear are drawn here and do nothing yet: the Host dialog and the Settings
//! dialog hang off them.

use crate::theme;
use crate::widgets::{self, Glyph, Tooltip as _};

/// The app's state across frames.
#[derive(Default)]
pub struct App {}

/// Readies `ctx` for the app, before its first frame: the bundled fonts, the tokens and
/// both themes' styles, following the system's light or dark.
pub fn setup(ctx: &egui::Context) {
    theme::install(ctx, egui::ThemePreference::System);
}

impl App {
    /// Draws one frame in `ui`, which covers the window.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        widgets::toolbar(
            ui,
            |ui| {
                ui.add(widgets::button("Hosts").flat());
            },
            |ui| {
                ui.add(widgets::glyph_button(Glyph::Gear).name("Settings"))
                    .tooltip("Settings");
            },
        );
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        App::ui(self, ui);
    }
}
