//! A listing's name cell: the entry's Kind icon, its name, and for a symlink an arrow on
//! the icon. The icon is drawn in the Kind's colour of the bundled set, and in the muted
//! text colour only when the cell is disabled.
//!
//! The name is painted, not a label, so the row that holds the cell is what names the
//! entry for AccessKit.

use egui::{
    Color32, CornerRadius, Label, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, Ui, Vec2,
    Widget, pos2, vec2,
};

use super::button::ICON_GAP;
use super::paint;
use crate::kind::Kind;
use crate::theme::{Tokens, TokensExt as _};

/// The name cell of an entry of `kind`, called `name`.
pub fn entry_name(name: &str, kind: Kind) -> EntryName<'_> {
    EntryName {
        name,
        kind,
        icon: true,
        link: false,
        target: None,
    }
}

#[must_use = "add it with ui.add"]
pub struct EntryName<'a> {
    name: &'a str,
    kind: Kind,
    icon: bool,
    link: bool,
    target: Option<&'a str>,
}

impl<'a> EntryName<'a> {
    /// Whether the Kind icon shows: the Show icons setting.
    pub fn icon(mut self, icon: bool) -> Self {
        self.icon = icon;
        self
    }

    /// A symlink: the icon is its target's, with an arrow.
    pub fn link(mut self, link: bool) -> Self {
        self.link = link;
        self
    }

    /// A symlink's target, shown muted after the name as "→ target".
    pub fn target(mut self, target: Option<&'a str>) -> Self {
        self.target = target;
        self
    }
}

/// The colour of `kind` in the bundled set.
pub fn kind_color(t: &Tokens, kind: Kind) -> Color32 {
    let k = &t.color.kind;
    match kind {
        Kind::Directory => k.directory,
        Kind::Text => k.text,
        Kind::Source => k.source,
        Kind::Markdown => k.markdown,
        Kind::Image => k.image,
        Kind::Pdf => k.pdf,
        Kind::Archive => k.archive,
        Kind::Audio => k.audio,
        Kind::Video => k.video,
        Kind::Other => k.other,
    }
}

impl Widget for EntryName<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let t = ui.tokens();
        let side = t.icon.small;
        let style = &t.ty.body;
        let text_width = paint::width(ui, style, self.name);
        let lead = if self.icon { side + ICON_GAP } else { 0.0 };
        let want = vec2(lead + text_width, t.row_height());
        let size = vec2(want.x.min(ui.available_width()), want.y);
        let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
        if !ui.is_rect_visible(rect) {
            return response;
        }
        let mut left = rect.left();
        if self.icon {
            let ink = if ui.is_enabled() {
                kind_color(&t, self.kind)
            } else {
                t.color.text.muted
            };
            let at = pos2(left + side / 2.0, rect.center().y);
            paint_kind(ui, &t, at, side, self.kind, ink);
            if self.link {
                paint_link_arrow(ui, &t, at, side);
            }
            left += lead;
        }
        let label = Rect::from_min_max(pos2(left, rect.top()), rect.max);
        paint::text_clipped(ui, label, style, self.name, t.color.text.primary);
        if let Some(target) = self.target {
            ui.add_space(t.space.gap);
            let job = style.job(&format!("→ {target}"), t.color.text.muted);
            ui.add(Label::new(job).selectable(false).truncate());
        }
        response
    }
}

/// A folder for a directory, a sheet with a folded corner for anything else.
pub(super) fn paint_kind(ui: &Ui, t: &Tokens, center: Pos2, side: f32, kind: Kind, ink: Color32) {
    let painter = ui.painter();
    let stroke = Stroke::new(t.stroke.accent_line.max(1.0), ink);
    if kind == Kind::Directory {
        let body = Rect::from_center_size(center + vec2(0.0, 1.0), vec2(side - 2.0, side - 5.0));
        let tab = Rect::from_min_size(body.left_top() + vec2(0.0, -2.0), vec2(side * 0.4, 3.0));
        painter.rect_filled(tab, CornerRadius::same(1), ink);
        painter.rect_filled(body, CornerRadius::same(2), ink);
        return;
    }
    let sheet = Rect::from_center_size(center, vec2(side * 0.7, side - 2.0));
    let fold = side * 0.25;
    let points = vec![
        sheet.left_top(),
        sheet.right_top() - vec2(fold, 0.0),
        sheet.right_top() + vec2(0.0, fold),
        sheet.right_bottom(),
        sheet.left_bottom(),
    ];
    painter.add(Shape::convex_polygon(points, Color32::TRANSPARENT, stroke));
    if side > t.icon.small {
        // The larger Properties icon has room for the fold and interior detail.
        let corner = sheet.right_top() + vec2(-fold, fold);
        painter.add(Shape::line(
            vec![corner - vec2(0.0, fold), corner, corner + vec2(fold, 0.0)],
            stroke,
        ));
        let left = sheet.left() + side * 0.12;
        for (y, fraction) in [(0.55, 0.72), (0.72, 0.54)] {
            painter.hline(
                left..=sheet.left() + sheet.width() * fraction,
                sheet.top() + sheet.height() * y,
                stroke,
            );
        }
        return;
    }
    let band = Rect::from_min_max(
        pos2(sheet.left(), sheet.bottom() - side * 0.3),
        sheet.right_bottom(),
    );
    painter.rect_filled(band, CornerRadius::ZERO, ink);
}

/// The symlink arrow: a small curved-in arrow on a disc of the row's ground, at the
/// icon's bottom-left corner.
pub(super) fn paint_link_arrow(ui: &Ui, t: &Tokens, center: Pos2, side: f32) {
    let painter = ui.painter();
    let at = center + Vec2::splat(side * 0.5) * vec2(-0.55, 0.55);
    let r = side * 0.28;
    painter.circle_filled(at, r, t.color.surface.card);
    painter.rect_stroke(
        Rect::from_center_size(at, Vec2::splat(2.0 * r)),
        CornerRadius::same(r as u8),
        Stroke::new(1.0, t.color.text.primary),
        StrokeKind::Inside,
    );
    let ink = Stroke::new(1.2, t.color.text.primary);
    let tip = at + vec2(r * 0.45, -r * 0.45);
    let tail = at + vec2(-r * 0.4, r * 0.4);
    painter.line_segment([tail, tip], ink);
    painter.line_segment([tip, tip + vec2(-r * 0.55, 0.0)], ink);
    painter.line_segment([tip, tip + vec2(0.0, r * 0.55)], ink);
}
