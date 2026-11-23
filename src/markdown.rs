//! Markdown Preview's rendered view: what the viewer is given, and which links may leave
//! the app.
//!
//! The view is egui_commonmark's, drawn with stock egui widgets, so it takes its look
//! from the `Style` the theme generates from the tokens (ADR-0001); its code blocks use
//! Source's syntect theme pair.
//!
//! Preview fetches nothing a Markdown file names. Every image, relative or not, is
//! replaced by its alt text before the viewer sees the text, so no image loader is ever
//! asked for one; and a clicked link reaches the system browser only when it is
//! `http(s)`. A relative link names a file on the host, not on this computer, so it does
//! nothing.

use egui_commonmark::{CommonMarkCache, CommonMarkScrollOptions, CommonMarkViewer};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

use crate::source;

/// The options the rendered view parses with, so that images are found the same way.
pub fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_DEFINITION_LIST
}

/// `source` with each image replaced by its alt text, as written (emphasis kept).
pub fn without_images(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut copied = 0;
    // The image being replaced: where it starts in `source`, and where its alt text
    // starts and ends.
    let mut image: Option<(usize, Option<(usize, usize)>)> = None;
    let mut depth = 0;
    for (event, range) in Parser::new_ext(source, options()).into_offset_iter() {
        match event {
            Event::Start(Tag::Image { .. }) => {
                if depth == 0 {
                    image = Some((range.start, None));
                }
                depth += 1;
            }
            Event::End(TagEnd::Image) => {
                depth -= 1;
                if depth == 0
                    && let Some((start, alt)) = image.take()
                {
                    out.push_str(&source[copied..start]);
                    if let Some((a, b)) = alt {
                        out.push_str(&source[a..b]);
                    }
                    copied = range.end;
                }
            }
            _ if depth > 0 => {
                if let Some((_, alt)) = &mut image {
                    *alt = Some(match *alt {
                        None => (range.start, range.end),
                        Some((a, b)) => (a.min(range.start), b.max(range.end)),
                    });
                }
            }
            _ => {}
        }
    }
    out.push_str(&source[copied..]);
    out
}

/// Whether a clicked link goes to the system browser: only `http://` and `https://`.
pub fn opens_in_browser(url: &str) -> bool {
    let lower = |n: usize| url.get(..n).map(str::to_ascii_lowercase);
    lower(7).as_deref() == Some("http://") || lower(8).as_deref() == Some("https://")
}

/// Draws `rendered` (from [`without_images`]) scrolled `scroll` pages further, and drops
/// any link it asked to open that does not [open in the browser](opens_in_browser).
pub fn show(
    ui: &mut egui::Ui,
    id: egui::Id,
    cache: &mut CommonMarkCache,
    rendered: &str,
    scroll: f32,
) {
    if scroll != 0.0 {
        // Taken by the viewer's scroll area, the next to end.
        let page = ui.available_height();
        ui.scroll_with_delta(egui::vec2(0.0, -scroll * page));
    }
    let mut scrolling = CommonMarkScrollOptions::default();
    scrolling.use_viewport_cache = true;
    CommonMarkViewer::new()
        .syntax_theme_light(source::LIGHT_THEME)
        .syntax_theme_dark(source::DARK_THEME)
        .show_scrollable(id, ui, cache, &scrolling, rendered);
    ui.ctx().output_mut(|o| {
        o.commands.retain(|c| match c {
            egui::OutputCommand::OpenUrl(open) => opens_in_browser(&open.url),
            _ => true,
        })
    });
}
