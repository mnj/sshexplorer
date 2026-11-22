//! Launches the sshexplorer window. It keeps the system title bar.

use sshexplorer::app::{self, App};

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("sshexplorer"),
        ..Default::default()
    };
    eframe::run_native(
        "sshexplorer",
        options,
        Box::new(|cc| {
            app::setup(&cc.egui_ctx);
            Ok(Box::new(App::default()))
        }),
    )
}
