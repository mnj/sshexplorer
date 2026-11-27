//! What a part of the window shows while it has nothing of its own: the [`welcome`]
//! column of the window with no Session, its [`host_tile`]s, and the [`empty_state`] of
//! a panel.
//!
//! ```ignore
//! widgets::welcome(ui, "sshexplorer", "Browse remote filesystems over SSH", |ui| {
//!     if ui.add(widgets::host_tile("build", "ci@10.0.0.4")).clicked() { .. }
//! });
//! widgets::empty_state(ui, Glyph::Info, "Nothing to preview", "Focus a file to see it here");
//! ```

use egui::{Align, Layout, Rect, Response, Role, Sense, Ui, Widget, WidgetInfo, pos2, vec2};

use super::glyph::{self, Glyph};
use super::paint::{self, State};
use crate::theme::{Text, TokensExt as _};

/// The welcome column's width.
pub const WELCOME_WIDTH: f32 = 420.0;
/// How much larger than the heading style the welcome title is.
const TITLE_SCALE: f32 = 1.5;
/// The side of the disc behind an empty state's Glyph.
const DISC: f32 = 40.0;
/// Above and below a tile's two lines.
const TILE_PAD_Y: f32 = 8.0;
/// Beside a tile's text.
const TILE_PAD_X: f32 = 12.0;
/// The widest an empty state's text runs.
const EMPTY_WIDTH: f32 = 280.0;

fn scaled(style: &Text, k: f32) -> Text {
    Text {
        size: style.size * k,
        line: style.line * k,
        ..style.clone()
    }
}

/// The column the window shows with no Session: `title` and `text`, then what `add`
/// shows, [`WELCOME_WIDTH`] wide and centred a little above the middle.
pub fn welcome(ui: &mut Ui, title: &str, text: &str, add: impl FnOnce(&mut Ui)) {
    let t = ui.tokens();
    let area = ui.available_rect_before_wrap();
    let width = WELCOME_WIDTH
        .min(area.width() - 2.0 * t.space.dialog)
        .max(0.0);
    // Last frame's height centres this frame's column.
    let id = ui.scope_id().with("welcome-height");
    let height = ui.data(|d| d.get_temp::<f32>(id)).unwrap_or(0.0);
    let top = area.top() + ((area.height() - height) * 0.4).max(t.space.dialog);
    let column = Rect::from_min_size(
        pos2(area.center().x - width / 2.0, top),
        vec2(width, (area.bottom() - top).max(0.0)),
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(column)
            .layout(Layout::top_down(Align::Min)),
    );
    let ui = &mut child;
    ui.spacing_mut().item_spacing.y = 0.0;
    let heading = scaled(&t.ty.heading, TITLE_SCALE);
    ui.label(heading.rich(title).color(t.color.text.primary));
    ui.add_space(t.space.stack / 2.0);
    ui.label(t.ty.body.rich(text).color(t.color.text.muted));
    ui.add_space(t.space.dialog);
    add(ui);
    let used = ui.min_rect().height();
    ui.data_mut(|d| d.insert_temp(id, used));
}

/// The small muted title over a group of tiles.
pub fn welcome_title(ui: &mut Ui, text: &str) {
    let t = ui.tokens();
    ui.label(t.ty.small.rich(text).color(t.color.text.muted));
    ui.add_space(t.space.stack);
}

/// Stacks the tiles `add` shows, the `stack` gap apart.
pub fn tiles<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.spacing_mut().item_spacing.y = ui.tokens().space.stack;
    add(ui)
}

/// A saved Host on the welcome column: its `name` over its `detail`, on a card that
/// fills the column's width. Its accessible name is "Open {name}".
pub fn host_tile<'a>(name: &'a str, detail: &'a str) -> HostTile<'a> {
    HostTile { name, detail }
}

#[must_use = "add it with ui.add"]
pub struct HostTile<'a> {
    name: &'a str,
    detail: &'a str,
}

impl Widget for HostTile<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let t = ui.tokens();
        let height = t.ty.body.line + t.ty.small.line + 2.0 * TILE_PAD_Y;
        let size = vec2(ui.available_width(), height);
        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
        let enabled = ui.is_enabled();
        let label = format!("Open {}", self.name);
        response.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, &label));
        if !ui.is_rect_visible(rect) {
            return response;
        }
        let s = paint::seen(&response, State::Live);
        let p = ui.painter();
        p.add(t.elevation.card.as_shape(rect, t.radius.control));
        let fill = paint::state_fill(&t, t.color.surface.card, s);
        let rim = egui::Stroke::new(t.stroke.hairline, t.color.stroke.hairline);
        p.rect(rect, t.radius.control, fill, rim, egui::StrokeKind::Inside);
        let text = rect.shrink2(vec2(TILE_PAD_X, TILE_PAD_Y));
        let text = text.with_max_x(text.right() - glyph::BOX);
        let (top, bottom) = text.split_top_bottom_at_y(text.top() + t.ty.body.line);
        let ink = t.color.text.primary;
        paint::text_clipped(ui, top, &t.ty.body_strong, self.name, ink);
        paint::text_clipped(ui, bottom, &t.ty.small, self.detail, t.color.text.muted);
        let at = pos2(
            rect.right() - TILE_PAD_X - glyph::BOX / 2.0,
            rect.center().y,
        );
        Glyph::ChevronRight.paint(ui.painter(), at, t.color.text.muted);
        if s.focus {
            paint::focus_ring(ui, &t, rect, t.radius.control);
        }
        response
    }
}

/// What a panel with nothing to show says, centred in the room `ui` has left: `glyph`
/// on a disc, `title`, and `text` under it in muted small text.
pub fn empty_state(ui: &mut Ui, glyph: Glyph, title: &str, text: &str) {
    let t = ui.tokens();
    let area = ui.available_rect_before_wrap();
    if area.height() < DISC || area.width() < DISC {
        return;
    }
    let width = EMPTY_WIDTH.min(area.width() - 2.0 * t.space.panel).max(0.0);
    let wrapped = |style: &Text, text: &str, color| {
        let mut job = style.job(text, color);
        job.wrap.max_width = width;
        job.halign = Align::Center;
        ui.painter().layout_job(job)
    };
    let title = wrapped(&t.ty.body_strong, title, t.color.text.primary);
    let text = wrapped(&t.ty.small, text, t.color.text.muted);
    let gap = t.space.stack;
    let height = DISC + gap + title.size().y + gap / 2.0 + text.size().y;
    if height > area.height() {
        return;
    }
    let x = area.center().x;
    let mut y = (area.center().y - height / 2.0).round();
    let p = ui.painter();
    let disc = pos2(x, y + DISC / 2.0);
    p.circle_filled(disc, DISC / 2.0, t.color.fill.control);
    glyph.paint(p, disc, t.color.text.muted);
    y += DISC + gap;
    p.galley(pos2(x.round(), y), title.clone(), t.color.text.primary);
    y += title.size().y + gap / 2.0;
    p.galley(pos2(x.round(), y), text, t.color.text.muted);
}
