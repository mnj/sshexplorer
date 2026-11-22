//! A list of keys and what they do: the Keys in Tool card's rows
//! (REDA docs/spec/v0.1/app.md#keys-in-tool).
//!
//! Two columns: each key as a key chip, then its action in body text, the actions lined
//! up in one column.
//!
//! ```ignore
//! widgets::key_list(ui, id.with("rows"), &[("W".into(), "Wire".into())]);
//! ```

use egui::{Align2, Grid, Response, Role, Sense, Ui, Widget, WidgetInfo};

use super::paint::{self, State};
use super::tooltip::{key_chip, key_chip_size};
use crate::theme::TokensExt as _;

/// Draws `rows` of `(key, action)`, the actions in one column.
pub fn key_list(ui: &mut Ui, id_salt: impl egui::AsIdSalt, rows: &[(String, String)]) {
    let t = ui.tokens();
    Grid::new(egui::IdSalt::new(id_salt))
        .num_columns(2)
        .spacing([t.space.section, t.space.row_y])
        .show(ui, |ui| {
            for (key, action) in rows {
                ui.add(key_chip(key.as_str()));
                let text = t.ty.body.rich(action.as_str()).color(t.color.text.primary);
                ui.label(text);
                ui.end_row();
            }
        });
}

/// A key chip that is a button: the binding editor's, clicked to record new keys.
/// `recording` shows it in the accent while it waits for them.
///
/// ```ignore
/// if ui.add(widgets::key_chip_button("Ctrl+W")).clicked() { .. }
/// ```
pub fn key_chip_button(keys: impl Into<String>) -> KeyChipButton {
    KeyChipButton {
        keys: keys.into(),
        recording: false,
    }
}

#[must_use = "add it with ui.add"]
pub struct KeyChipButton {
    keys: String,
    recording: bool,
}

impl KeyChipButton {
    pub fn recording(mut self, recording: bool) -> Self {
        self.recording = recording;
        self
    }
}

impl Widget for KeyChipButton {
    fn ui(self, ui: &mut Ui) -> Response {
        let t = ui.tokens();
        let size = key_chip_size(ui, &t, &self.keys);
        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
        let enabled = ui.is_enabled();
        response.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, &self.keys));
        if !ui.is_rect_visible(rect) {
            return response;
        }
        let s = paint::seen(&response, State::Live);
        let (under, ink) = if self.recording {
            (t.color.accent.subtle, t.color.accent.text)
        } else {
            (t.color.fill.control, t.color.text.muted)
        };
        let fill = paint::state_fill(&t, under, s);
        ui.painter().rect_filled(rect, t.radius.mark, fill);
        paint::text_in(
            ui,
            rect,
            Align2::CENTER_CENTER,
            &t.ty.small,
            &self.keys,
            ink,
        );
        if s.focus {
            paint::focus_ring(ui, &t, rect, t.radius.mark);
        }
        response
    }
}
