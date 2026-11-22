//! A spinner for work in progress, such as a Refresh: egui's, in muted ink at the Glyph
//! size.

use egui::{Response, Ui, Widget};

use super::glyph::BOX;
use crate::theme::TokensExt as _;

/// A spinner; `name` is what AccessKit calls it.
pub fn spinner(name: impl Into<String>) -> Spinner {
    Spinner { name: name.into() }
}

pub struct Spinner {
    name: String,
}

impl Widget for Spinner {
    fn ui(self, ui: &mut Ui) -> Response {
        let ink = ui.tokens().color.text.muted;
        let response = ui.add(egui::Spinner::new().size(BOX).color(ink));
        let name = self.name;
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ProgressIndicator, true, &name));
        response
    }
}
