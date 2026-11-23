//! Preview's text body: numbered lines of text or highlighted source in the monospace
//! style, without wrapping, scrolling both ways; and the spinner beside "Loading
//! preview…".
//!
//! Only the rows in view are laid out, so a 10 MiB file scrolls as quickly as a short
//! one. Each line is a selectable label: a drag selects across the lines in view, and
//! the selection copies with the usual keys. The line numbers are painted, so they are
//! never part of a selection.

use egui::text::LayoutJob;
use egui::{Color32, Label, Response, Sense, TextWrapMode, Ui, Widget, vec2};

use crate::source::Span;
use crate::theme::TokensExt as _;

/// One line as [`code_view`] draws it: its text and, once highlighted, its colours.
pub struct CodeLine<'a> {
    pub text: &'a str,
    /// Coloured runs covering the line; empty draws it plain.
    pub spans: &'a [Span],
}

/// A view of `count` lines, each given by `line`, numbered from 1.
pub fn code_view<'a>(
    id_salt: impl egui::AsIdSalt,
    count: usize,
    line: &'a dyn Fn(usize) -> CodeLine<'a>,
) -> CodeView<'a> {
    CodeView {
        id_salt: egui::IdSalt::new(id_salt),
        count,
        line,
        scroll_rows: 0.0,
    }
}

#[must_use = "call show"]
pub struct CodeView<'a> {
    id_salt: egui::IdSalt,
    count: usize,
    line: &'a dyn Fn(usize) -> CodeLine<'a>,
    scroll_rows: f32,
}

impl CodeView<'_> {
    /// Scrolls by `rows` lines this frame: positive moves down the text.
    pub fn scroll_rows(mut self, rows: f32) -> Self {
        self.scroll_rows = rows;
        self
    }

    /// Draws the view in the rest of `ui`; returns the number of rows that fit in it,
    /// for paging.
    pub fn show(self, ui: &mut Ui) -> usize {
        let t = ui.tokens();
        let style = &t.ty.mono;
        let row = style.line;
        let digits = self.count.max(1).to_string().len();
        let gutter = paint_width(ui, style, &"0".repeat(digits));
        let plain = t.color.text.primary;
        let muted = t.color.text.muted;
        let gap = t.space.gap;
        let page = ((ui.available_height() / row).floor() as usize).max(1);
        let scroll = self.scroll_rows * row;
        egui::ScrollArea::both()
            .id_salt(self.id_salt)
            .auto_shrink(false)
            .show_rows(ui, row, self.count, |ui, rows| {
                if scroll != 0.0 {
                    ui.scroll_with_delta(vec2(0.0, -scroll));
                }
                for i in rows {
                    let line = (self.line)(i);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        let number = (i + 1).to_string();
                        let (rect, _) = ui.allocate_exact_size(vec2(gutter, row), Sense::hover());
                        let galley = ui.painter().layout_job(style.job(&number, muted));
                        let at = rect.right_top() - vec2(galley.size().x, 0.0);
                        ui.painter().galley(at, galley, muted);
                        ui.add(
                            Label::new(job(style, line, plain))
                                .selectable(true)
                                .wrap_mode(TextWrapMode::Extend),
                        );
                    });
                }
            });
        page
    }
}

/// The line's text as a layout job: each span in its ink, the rest plain.
fn job(style: &crate::theme::Text, line: CodeLine<'_>, plain: Color32) -> LayoutJob {
    let mut job = LayoutJob::default();
    let mut at = 0;
    for span in line.spans {
        let (start, end) = (
            span.start.min(line.text.len()),
            span.end.min(line.text.len()),
        );
        if start > at {
            job.append(&line.text[at..start], 0.0, style.format(plain));
        }
        if end > start
            && let Some(piece) = line.text.get(start..end)
        {
            let [r, g, b] = span.ink;
            job.append(piece, 0.0, style.format(Color32::from_rgb(r, g, b)));
            at = end;
        }
    }
    if at < line.text.len() {
        job.append(
            line.text.get(at..).unwrap_or_default(),
            0.0,
            style.format(plain),
        );
    }
    if job.sections.is_empty() {
        job.append("", 0.0, style.format(plain));
    }
    job
}

fn paint_width(ui: &Ui, style: &crate::theme::Text, text: &str) -> f32 {
    ui.painter()
        .layout_job(style.job(text, Color32::TRANSPARENT))
        .size()
        .x
}

/// The value of a details row in a [`super::form`] whose name is `name`: read-only text
/// that wraps. Assistive tech reads it as `name` with the text as its value, so a file's
/// name shown here is not taken for the listing row of the same name.
pub fn detail_value(name: impl Into<String>, text: impl Into<String>) -> DetailValue {
    DetailValue {
        name: name.into(),
        text: text.into(),
    }
}

#[must_use = "add it with ui.add"]
pub struct DetailValue {
    name: String,
    text: String,
}

impl Widget for DetailValue {
    fn ui(self, ui: &mut Ui) -> Response {
        let t = ui.tokens();
        let ink = t.color.text.primary;
        let job = LayoutJob {
            wrap: egui::text::TextWrapping::wrap_at_width(ui.available_width().max(row_min(&t))),
            ..t.ty.body.job(&self.text, ink)
        };
        let galley = ui.painter().layout_job(job);
        let size = vec2(galley.size().x, galley.size().y.max(t.control_height()));
        let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
        response.widget_info(|| {
            let mut info = egui::WidgetInfo::new(egui::Role::Definition);
            info.current_text_value = Some(self.text.clone());
            info
        });
        // The name as the label, the text as the value: a description list's definition.
        let name = self.name;
        response
            .ctx
            .accesskit_node_builder(response.id, |node| node.set_label(name));
        if ui.is_rect_visible(rect) {
            let at = egui::Align2::LEFT_CENTER
                .align_size_within_rect(galley.size(), rect)
                .min
                .round();
            ui.painter().galley(at, galley, ink);
        }
        response
    }
}

/// The narrowest a details value wraps to.
fn row_min(t: &crate::theme::Tokens) -> f32 {
    t.ty.body.line * 4.0
}

/// Body text in the muted colour: a status, a placeholder or a path beside a control.
pub fn muted(text: impl Into<String>) -> Muted {
    Muted { text: text.into() }
}

#[must_use = "add it with ui.add"]
pub struct Muted {
    text: String,
}

impl Widget for Muted {
    fn ui(self, ui: &mut Ui) -> Response {
        let t = ui.tokens();
        ui.label(t.ty.body.rich(self.text).color(t.color.text.muted))
    }
}

/// The spinner and a muted line beside it, as "Loading preview…" shows while a load
/// runs.
pub fn loading_line(text: impl Into<String>) -> LoadingLine {
    LoadingLine { text: text.into() }
}

#[must_use = "add it with ui.add"]
pub struct LoadingLine {
    text: String,
}

impl Widget for LoadingLine {
    fn ui(self, ui: &mut Ui) -> Response {
        ui.horizontal(|ui| {
            ui.add(spinner());
            ui.add(muted(self.text));
        })
        .response
    }
}

/// The busy spinner, in the muted text colour at the body line's height.
pub fn spinner() -> Spinner {
    Spinner
}

#[must_use = "add it with ui.add"]
pub struct Spinner;

impl Widget for Spinner {
    fn ui(self, ui: &mut Ui) -> Response {
        let t = ui.tokens();
        ui.add(
            egui::Spinner::new()
                .size(t.ty.body.line)
                .color(t.color.text.muted),
        )
    }
}
