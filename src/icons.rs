//! Icons: a bundled image per icon key, drawn by the widgets that take a key
//! (`widgets::icon_button`, a menu item's icon, a flyout's title).
//!
//! No icon is bundled yet, so every key draws nothing; the Kind icons add the first set.
//! A Glyph (`widgets::glyph`) is not an icon: it is drawn as strokes in the text colour.

use egui::{Image, Theme};

/// The icon `key` at `size` px for `theme`, or its greyscale copy when not `enabled`.
/// `None` for a key that has no bundled image.
pub fn image(key: &str, size: f32, theme: Theme, enabled: bool) -> Option<Image<'static>> {
    let _ = (key, size, theme, enabled);
    None
}
