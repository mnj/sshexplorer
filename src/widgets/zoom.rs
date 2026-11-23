//! The zoom toolbar and the zoomable view it drives, shared by image, PDF and video
//! Preview. The model is [`crate::zoom::Zoom`]; these draw it and feed it input.
//!
//! ```ignore
//! widgets::zoom_bar(ui, &mut zoom, Some("1920 × 1080"));
//! let view = widgets::zoom_view(ui.id().with("view"), "Image", &mut zoom, content)
//!     .keys(Some(keymap))
//!     .show(ui);
//! ui.painter_at(view.clip).image(texture.id(), view.content, uv, Color32::WHITE);
//! ```
//!
//! The bar has Fit · 50% · 100% · 200% · 400% as toggles, the pixel dimensions, and the
//! scale when Ctrl+wheel left it between presets. The view takes the space left, centres
//! content smaller than itself, pans larger content by dragging, zooms with Ctrl+wheel
//! around the pointer, offers the same choices on its right-click menu, and takes the
//! zoom keys of the key map when it is given one.

use egui::{Id, Rect, Response, Sense, Ui, Vec2};

use crate::keymap::{Action, KeyMap};
use crate::theme::TokensExt as _;
use crate::zoom::{Level, Zoom};

/// The zoom toolbar: the choices, then `dimensions` ("1920 × 1080") when given.
pub fn zoom_bar(ui: &mut Ui, zoom: &mut Zoom, dimensions: Option<&str>) {
    let t = ui.tokens();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = t.space.gap;
        for level in Level::choices() {
            let on = zoom.level == level;
            if ui.add(super::button(level.label()).flat().on(on)).clicked() {
                zoom.set(level);
            }
        }
        if !Level::choices().contains(&zoom.level) {
            ui.add(super::detail_value("Zoom", zoom.label()));
        }
        if let Some(dimensions) = dimensions {
            ui.label(t.ty.body.rich(dimensions).color(t.color.text.muted));
        }
    });
}

/// The view `content` (in its own pixels) is shown in, named `name` for assistive tech.
pub fn zoom_view<'a>(id: Id, name: &'a str, zoom: &'a mut Zoom, content: Vec2) -> ZoomView<'a> {
    ZoomView {
        id,
        name,
        zoom,
        content,
        keymap: None,
    }
}

#[must_use = "call .show(ui)"]
pub struct ZoomView<'a> {
    id: Id,
    name: &'a str,
    zoom: &'a mut Zoom,
    content: Vec2,
    keymap: Option<&'a KeyMap>,
}

/// Where to paint, after [`ZoomView::show`].
pub struct ZoomShown {
    /// The scaled content on screen; it may reach past `clip`.
    pub content: Rect,
    /// The view: paint clipped to it.
    pub clip: Rect,
    pub response: Response,
}

impl<'a> ZoomView<'a> {
    /// Takes Zoom in, Zoom out and Zoom to Fit from the key map this frame.
    pub fn keys(mut self, keymap: Option<&'a KeyMap>) -> Self {
        self.keymap = keymap;
        self
    }

    pub fn show(self, ui: &mut Ui) -> ZoomShown {
        let ZoomView {
            id,
            name,
            zoom,
            content,
            keymap,
        } = self;
        let t = ui.tokens();
        let size = ui
            .available_size()
            .max(Vec2::splat(t.control_height() * 2.0));
        let (view, _) = ui.allocate_exact_size(size, Sense::hover());
        let fit = Zoom::fit_scale(content, view.size());
        let sense = if zoom.pans(content, view.size()) {
            Sense::click_and_drag()
        } else {
            Sense::click()
        };
        let response = ui.interact(view, id, sense);
        let label = name.to_owned();
        response
            .ctx
            .accesskit_node_builder(response.id, |node| node.set_label(label));

        if let Some(keymap) = keymap {
            ui.input_mut(|i| {
                if keymap.take(i, Action::ZoomIn) {
                    zoom.zoom_in(fit);
                }
                if keymap.take(i, Action::ZoomOut) {
                    zoom.zoom_out(fit);
                }
                if keymap.take(i, Action::ZoomFit) {
                    zoom.fit();
                }
            });
        }
        if response.dragged() {
            zoom.drag(response.drag_delta(), content, view);
        }
        if response.hovered() {
            let (factor, pointer) = ui.input(|i| (i.zoom_delta(), i.pointer.hover_pos()));
            if factor != 1.0
                && let Some(pointer) = pointer
            {
                zoom.wheel(factor, fit, view, pointer, content);
            }
        }
        zoom_menu(&response, zoom);
        if response.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if response.hovered() && zoom.pans(content, view.size()) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
        ZoomShown {
            content: zoom.layout(content, view),
            clip: view,
            response,
        }
    }
}

/// Paints the texture `texture` as the content of a [`ZoomView`], clipped to the view.
pub fn paint_texture(ui: &Ui, shown: &ZoomShown, texture: egui::TextureId) {
    let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    ui.painter_at(shown.clip)
        .image(texture, shown.content, uv, egui::Color32::WHITE);
}

/// The right-click menu on `response` with the zoom toolbar's choices.
pub fn zoom_menu(response: &Response, zoom: &mut Zoom) {
    super::context_menu(response, |ui| {
        for level in Level::choices() {
            let item = super::menu_item(level.label()).checked(zoom.level == level);
            if ui.add(item).clicked() {
                zoom.set(level);
            }
        }
    });
}

/// Paints one PDF page at `rect`, clipped to `clip`: its rendered texture stretched to
/// the page, or blank paper until it is rendered.
pub fn paint_page(ui: &Ui, clip: Rect, rect: Rect, texture: Option<egui::TextureId>) {
    let painter = ui.painter_at(clip);
    match texture {
        Some(texture) => {
            let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            painter.image(texture, rect, uv, egui::Color32::WHITE);
        }
        None => {
            painter.rect_filled(rect, 0.0, egui::Color32::WHITE);
        }
    }
    let t = ui.tokens();
    painter.rect_stroke(
        rect,
        0.0,
        egui::Stroke::new(t.stroke.hairline, t.color.stroke.hairline),
        egui::StrokeKind::Outside,
    );
}
