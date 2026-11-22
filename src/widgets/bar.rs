//! Bars (REDA docs/spec/v0.1/design-language.md#bars-on-the-canvas): the [`toolbar`]
//! along the window's top, and floating bars in a [`bar_frame`] holding Icon buttons of
//! the control height [`BAR_GAP`] apart.
//!
//! ```ignore
//! let t = ui.tokens();
//! widgets::bar_frame(&t).show(ui, |ui| {
//!     widgets::bar_row(ui, |ui| {
//!         ui.add(widgets::icon_button("wire").bar().on(active));
//!         widgets::bar_separator(ui);
//!         ui.add(widgets::glyph_button(widgets::Glyph::Funnel));
//!     });
//! });
//! ```

use egui::{Align, Frame, InnerResponse, Layout, Margin, Panel, Sense, Stroke, Ui, vec2};

use crate::theme::{Tokens, TokensExt as _};

/// Between the buttons of a bar.
pub const BAR_GAP: f32 = 2.0;
/// The width a bar separator takes.
pub const BAR_SEPARATOR: f32 = 7.0;

/// The toolbar along the top of the window: the base surface with a hairline under it,
/// `toolbar_height` tall, its controls `gap` apart and vertically centred. `leading` lays
/// out from the left end and `trailing` from the right end (the Settings gear).
pub fn toolbar(ui: &mut Ui, leading: impl FnOnce(&mut Ui), trailing: impl FnOnce(&mut Ui)) {
    let t = ui.tokens();
    let frame = Frame::NONE
        .fill(t.color.surface.base)
        .inner_margin(Margin::symmetric(t.space.panel as i8, t.space.bar_y as i8));
    Panel::top("toolbar")
        .frame(frame)
        .resizable(false)
        .exact_size(t.toolbar_height())
        .show(ui, |ui| {
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                leading(ui);
                ui.with_layout(Layout::right_to_left(Align::Center), trailing);
            });
        });
}

/// The floating surface a bar on the canvas sits in: the [`super::floating_frame`], so
/// every bar over the canvas has one margin and one radius.
pub fn bar_frame(t: &Tokens) -> Frame {
    super::floating_frame(t)
}

/// What a [`bar_frame`] adds on each side of its row: the `menu` padding and the rim.
pub fn bar_frame_inset(t: &Tokens) -> f32 {
    t.space.menu + t.stroke.hairline
}

/// The side of a bar's Icon button: the control height.
pub fn bar_button_side(t: &Tokens) -> f32 {
    t.control_height()
}

/// A row of bar items, [`BAR_GAP`] apart.
pub fn bar_row<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = BAR_GAP;
        add(ui)
    })
}

/// A thin vertical hairline between groups of bar buttons, as tall as they are.
pub fn bar_separator(ui: &mut Ui) {
    let height = bar_button_side(&ui.tokens());
    bar_separator_sized(ui, height);
}

/// [`bar_separator`] beside buttons `height` points tall (the quick strip's).
pub fn bar_separator_sized(ui: &mut Ui, height: f32) {
    let t = ui.tokens();
    let (rect, _) = ui.allocate_exact_size(vec2(BAR_SEPARATOR, height), Sense::hover());
    let stroke = Stroke::new(t.stroke.hairline, t.color.stroke.hairline);
    let x = rect.center().x.round() + stroke.width / 2.0;
    let y = rect.y_range().shrink(height * 0.15);
    ui.painter().vline(x, y, stroke);
}
