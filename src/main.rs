//! Launches the sshexplorer window. It keeps the system title bar.

use std::sync::Arc;

use sshexplorer::app::{self, App};
use sshexplorer::credentials;
use sshexplorer::media;
use sshexplorer::scratch;
use sshexplorer::settings::Store;
use sshexplorer::theme::fonts;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("sshexplorer"),
        ..Default::default()
    };
    let mut app = Store::platform()
        .map_or_else(App::default, App::with_store)
        .operator_ssh()
        .credentials_at(Arc::new(credentials::OsStore))
        .audio_out(media::AudioOut::Device);
    // Preview files a crash left in the Scratch folder.
    scratch::clear_leftovers(&app.settings().preview);
    let installed = fonts::Installed::load();
    eframe::run_native(
        "sshexplorer",
        options,
        Box::new(move |cc| {
            app::setup(&cc.egui_ctx);
            fonts::set_source(&cc.egui_ctx, Arc::new(installed));
            // Before the first frame, so it is drawn in the chosen fonts and scale.
            app.apply(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    )
}
