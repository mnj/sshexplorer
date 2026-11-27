//! The path as breadcrumbs: one flat button per directory from the root down, chevrons
//! between them, the last one in the primary text colour.
//!
//! A path too long for the row drops its leading directories behind an ellipsis, so the
//! directory the operator is in always shows. Each crumb's accessible name is its whole
//! path.
//!
//! ```ignore
//! if let Some(path) = widgets::crumbs(ui, "/home/tester") {
//!     session.open_bookmark(&path);
//! }
//! ```

use egui::{Align2, Color32, Role, Sense, Ui, WidgetInfo, pos2, vec2};

use super::glyph::{self, Glyph};
use super::paint::{self, State};
use crate::theme::TokensExt as _;

/// Beside a crumb's text.
const CRUMB_PAD: f32 = 6.0;
/// The room a chevron between two crumbs takes.
const CHEVRON: f32 = 12.0;
/// How far a crumb's hover fill stands in from the row's top and bottom.
const FILL_INSET: f32 = 2.0;
const ELLIPSIS: &str = "…";

/// The crumbs of the absolute `path`: the root, then each directory with its own path.
fn parts(path: &str) -> Vec<(String, String)> {
    let mut parts = vec![("/".to_owned(), "/".to_owned())];
    let mut whole = String::new();
    for name in path.split('/').filter(|name| !name.is_empty()) {
        whole.push('/');
        whole.push_str(name);
        parts.push((name.to_owned(), whole.clone()));
    }
    parts
}

/// Shows `path` as breadcrumbs in the width left in `ui`, one control high. Gives the
/// path of the crumb clicked. With `enabled` off the crumbs show and cannot be clicked.
pub fn crumbs(ui: &mut Ui, path: &str, enabled: bool) -> Option<String> {
    let t = ui.tokens();
    let parts = parts(path);
    let widths: Vec<f32> = parts
        .iter()
        .map(|(name, _)| paint::width(ui, &t.ty.body_strong, name) + 2.0 * CRUMB_PAD)
        .collect();
    let room = ui.available_width();
    let ellipsis = paint::width(ui, &t.ty.body, ELLIPSIS) + CHEVRON;
    // Drop leading crumbs until what is left fits; the last one always stays.
    let mut first = 0;
    let total = |first: usize| {
        let shown = &widths[first..];
        let lead = if first > 0 { ellipsis } else { 0.0 };
        lead + shown.iter().sum::<f32>() + CHEVRON * (shown.len() - 1) as f32
    };
    while first + 1 < parts.len() && total(first) > room {
        first += 1;
    }
    let height = t.control_height();
    let mut clicked = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        if first > 0 {
            let (rect, _) = ui.allocate_exact_size(vec2(ellipsis, height), Sense::hover());
            let text = rect.with_max_x(rect.right() - CHEVRON);
            let ink = t.color.text.muted;
            paint::text_in(ui, text, Align2::CENTER_CENTER, &t.ty.body, ELLIPSIS, ink);
            let at = pos2(rect.right() - CHEVRON / 2.0, rect.center().y);
            Glyph::ChevronRight.paint(ui.painter(), at, t.color.stroke.strong);
        }
        let last = parts.len() - 1;
        for (i, (name, whole)) in parts.iter().enumerate().skip(first) {
            let width = widths[i].min(ui.available_width().max(glyph::BOX));
            let sense = if enabled {
                Sense::click()
            } else {
                Sense::hover()
            };
            let (rect, response) = ui.allocate_exact_size(vec2(width, height), sense);
            response.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, whole));
            if ui.is_rect_visible(rect) {
                let s = paint::seen(&response, State::Live);
                let ground = rect.shrink2(vec2(0.0, FILL_INSET));
                let fill = paint::state_fill(&t, Color32::TRANSPARENT, s);
                ui.painter().rect_filled(ground, t.radius.row, fill);
                let (style, ink) = if i == last {
                    (&t.ty.body_strong, t.color.text.primary)
                } else {
                    (&t.ty.body, t.color.text.muted)
                };
                paint::text_clipped(ui, rect.shrink2(vec2(CRUMB_PAD, 0.0)), style, name, ink);
                if s.focus {
                    paint::focus_ring(
                        ui,
                        &t,
                        ground.shrink(paint::RING_GAP + t.stroke.focus_ring),
                        t.radius.row,
                    );
                }
            }
            if response.clicked() {
                clicked = Some(whole.clone());
            }
            if i < last {
                let (gap, _) = ui.allocate_exact_size(vec2(CHEVRON, height), Sense::hover());
                Glyph::ChevronRight.paint(ui.painter(), gap.center(), t.color.stroke.strong);
            }
        }
    });
    clicked
}
