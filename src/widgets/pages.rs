//! The pages dialog: a modal over the scrim with a title row and close ✕, its pages
//! listed on the left and the chosen page's fields on the right, and a footer with the
//! page's reset button and Close. There is no OK, Apply or Cancel: what the fields change
//! applies at once. The Settings dialog is one.
//!
//! ```ignore
//! let shown = widgets::pages_dialog(id, "Changes apply at once", &PAGES)
//!     .reset_label("Reset Appearance to defaults")
//!     .show(ctx, &mut page, |ui, page| { .. });
//! if shown.close { .. }
//! if shown.reset { .. }
//! ```

use egui::{Context, Id, Modal, ScrollArea, Ui};

use super::Highlight;
use crate::theme::TokensExt as _;
use crate::theme::motion::{self, Look};

/// The page list's width.
pub const PAGE_LIST_WIDTH: f32 = 168.0;
/// The fields' column: its width, and the height every page is given so the dialog does
/// not change size from page to page.
pub const PAGE_WIDTH: f32 = 480.0;
pub const PAGE_HEIGHT: f32 = 400.0;

/// A pages dialog titled `title` over `pages`.
pub fn pages_dialog<'a>(id: Id, title: &'a str, pages: &'a [&'a str]) -> PagesDialog<'a> {
    PagesDialog {
        id,
        title,
        pages,
        reset: "Reset to defaults".to_owned(),
    }
}

#[must_use = "call .show()"]
pub struct PagesDialog<'a> {
    id: Id,
    title: &'a str,
    pages: &'a [&'a str],
    reset: String,
}

/// What the operator did in a pages dialog this frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct PagesShown {
    /// The ✕, Close or Escape: the dialog should close.
    pub close: bool,
    /// The footer's reset button for the page shown.
    pub reset: bool,
}

impl PagesDialog<'_> {
    /// The footer's reset button, such as "Reset Appearance to defaults".
    pub fn reset_label(mut self, label: impl Into<String>) -> Self {
        self.reset = label.into();
        self
    }

    /// Shows the dialog for one frame, with `page` the index of the page shown; a click on
    /// another page's entry changes it. `fields` draws the page it is given.
    pub fn show(
        self,
        ctx: &Context,
        page: &mut usize,
        fields: impl FnOnce(&mut Ui, usize),
    ) -> PagesShown {
        let t = ctx.tokens();
        let mut shown = PagesShown::default();
        let modal = Modal::new(self.id)
            .backdrop_color(t.color.surface.scrim)
            .frame(super::dialog_frame(&t))
            .show(ctx, |ui| {
                ui.set_width(PAGE_LIST_WIDTH + t.space.section + PAGE_WIDTH);
                shown.close |= super::title_row(ui, self.title, true);
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = t.space.section;
                    ui.vertical(|ui| {
                        ui.set_width(PAGE_LIST_WIDTH);
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for (i, name) in self.pages.iter().enumerate() {
                            let selection = if i == *page {
                                Highlight::Live
                            } else {
                                Highlight::None
                            };
                            let row = super::list_row(*name)
                                .selection(selection)
                                .width(PAGE_LIST_WIDTH)
                                .show(ui);
                            if row.response.clicked() {
                                *page = i;
                            }
                        }
                    });
                    ui.vertical(|ui| {
                        ui.set_width(PAGE_WIDTH);
                        ui.set_min_height(PAGE_HEIGHT);
                        ScrollArea::vertical()
                            .id_salt(self.id.with("page"))
                            .max_height(PAGE_HEIGHT)
                            .auto_shrink([false, true])
                            .show(ui, |ui| fields(ui, *page));
                    });
                });
                super::button_row(ui, |ui| {
                    shown.close |= ui.add(super::button("Close").primary()).clicked();
                    shown.reset |= ui.add(super::button(self.reset.as_str())).clicked();
                });
            });
        ctx.accesskit_node_builder(modal.response.id, |node| {
            node.set_role(egui::accesskit::Role::Dialog);
            node.set_modal();
            node.set_label(self.title);
        });
        let look = Look::grow(t.motion.dialog_scale, modal.response.rect.center());
        motion::layer(ctx, modal.response.layer_id, look, t.motion.dialog);
        let escape = modal.is_top_modal
            && !modal.any_popup_open
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        shown.close |= escape;
        shown
    }
}
