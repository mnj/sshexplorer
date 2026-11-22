//! The Preview panel on the right of a Session: the Focused entry's details header and,
//! below it, what [`Preview`] has for the file.
//!
//! The panel fetches over the Session's connection when the model says a fetch is due,
//! and drops the read when the preview changes, which stops it. The bytes go to the
//! Scratch folder the Preview settings choose. Source is highlighted on a thread of its
//! own for the Theme in force, and again when Theme changes.
//!
//! The panel is a focus target of its own. While it holds the keyboard, the move keys
//! scroll it, Escape gives the keyboard back to the listing, and the cycle keys move on.

use egui::{EventFilter, Id, Key, Modifiers, Sense};

use crate::archive::{Listing, Stamp};
use crate::folder::{modified_text, permissions_text, size_text};
use crate::image_view::ImageView;
use crate::keymap::{Action, KeyMap};
use crate::media_view::{MediaFrame, MediaView};
use crate::pdf_view::PdfView;
use crate::preview::{self, Body, Focused, MarkdownView, Preview, Text, UNSUPPORTED};
use crate::scratch::Place;
use crate::settings::AppSettings;
use crate::source::Highlighting;
use crate::ssh::{self, Request};
use crate::theme::TokensExt as _;
use crate::widgets::{self, CodeLine};

/// What the panel needs from its Session to draw a frame.
pub struct PanelFrame<'a> {
    pub connection: &'a ssh::Session,
    pub focused: Option<Focused>,
    pub keymap: &'a KeyMap,
    pub settings: &'a AppSettings,
    /// Whether the panel may take keys: no dialog is up.
    pub keys: bool,
    pub zone: &'a jiff::tz::TimeZone,
    /// Where audio and video send their sound.
    pub audio: crate::media::AudioOut,
}

/// What the panel asks of its Session after a frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Leave {
    Stay,
    /// Escape: back to the listing.
    Back,
    /// The cycle keys, forward or back.
    Next,
    Previous,
}

pub struct PreviewPanel {
    pub id: Id,
    preview: Preview,
    /// The read in flight, and the fetch it answers.
    reading: Option<(u64, Request<Vec<u8>>)>,
    /// Rows the text view showed last frame, for paging.
    page: usize,
    /// The shown image's decoding, textures and zoom; new for each preview.
    image: ImageView,
    /// The shown PDF's renderer, textures, zoom and scroll; new for each preview.
    pdf: PdfView,
    /// The shown audio or video's playback; new for each preview.
    media: MediaView,
    /// The rendered Markdown view's state.
    markdown: egui_commonmark::CommonMarkCache,
    /// The shown archive listing's rows, made on first draw.
    archive_rows: Option<Vec<String>>,
}

/// Under "Too large to preview (…)".
pub const TOO_LARGE_HINT: &str = "Download it to open it locally.";
/// An archive with no members.
pub const EMPTY_ARCHIVE: &str = "The archive is empty.";

impl PreviewPanel {
    pub fn new(id: Id) -> Self {
        PreviewPanel {
            id,
            preview: Preview::default(),
            reading: None,
            page: 1,
            image: ImageView::default(),
            pdf: PdfView::default(),
            media: MediaView::default(),
            markdown: egui_commonmark::CommonMarkCache::default(),
            archive_rows: None,
        }
    }

    pub fn has_focus(&self, ui: &egui::Ui) -> bool {
        ui.memory(|m| m.has_focus(self.id))
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, frame: PanelFrame<'_>) -> Leave {
        let now = ui.input(|i| i.time);
        if self.preview.focus(frame.focused, now) {
            self.reading = None;
            self.image = ImageView::default();
            self.pdf = PdfView::default();
            self.media = MediaView::default();
            self.archive_rows = None;
        }
        self.receive(&Place::of(&frame.settings.preview));
        let cap = frame.settings.preview.size_cap_bytes();
        if let Some(fetch) = self.preview.due(now, cap) {
            let request = frame.connection.read(fetch.path, 0, fetch.len as usize);
            self.reading = Some((fetch.id, request));
        }
        if let Some(wait) = self.preview.wait(now) {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(wait));
        }
        self.highlight(ui);

        let area = ui.available_rect_before_wrap();
        // Under the contents, so a click on the panel's empty space gives it the keyboard.
        let response = ui.interact(area, self.id, Sense::click());
        response
            .ctx
            .accesskit_node_builder(response.id, |node| node.set_label("Preview"));
        let focused = self.has_focus(ui);
        let mut leave = Leave::Stay;
        let mut scroll = 0.0;
        if focused {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    self.id,
                    EventFilter {
                        tab: true,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        escape: true,
                    },
                )
            });
            if frame.keys {
                (leave, scroll) = self.keys(ui, frame.keymap);
            }
        }
        if response.clicked() {
            ui.memory_mut(|m| m.request_focus(self.id));
        }

        // Whether the keyboard is in Preview (on the panel or one of its controls), or
        // `None` when nothing has it, as after a click on a control.
        let inside = ui.memory(|m| m.focused()).map(|id| {
            id == self.id
                || ui
                    .ctx()
                    .read_response(id)
                    .is_some_and(|r| area.contains(r.rect.center()))
        });
        let toggle =
            focused && frame.keys && ui.input_mut(|i| frame.keymap.take(i, Action::PlayPause));

        let Some(shown) = self.preview.shown_mut() else {
            return leave;
        };
        details(ui, &shown.focused, shown.kind, frame.zone);
        let t = ui.tokens();
        ui.add_space(t.space.stack);
        match &mut shown.body {
            Body::Header => {}
            Body::Loading => {
                ui.horizontal(|ui| {
                    ui.add(widgets::spinner());
                    ui.label(t.ty.body.rich("Loading preview…").color(t.color.text.muted));
                });
            }
            Body::Unsupported => {
                ui.label(t.ty.body.rich(UNSUPPORTED).color(t.color.text.muted));
            }
            Body::Failed(line) => {
                ui.add(widgets::invalid_message(line.as_str()));
            }
            Body::Image(image) => {
                let keys = frame.keys.then_some(frame.keymap);
                if let Some(line) = self.image.ui(ui, &shown.focused.name, image, keys) {
                    shown.body = Body::Failed(line);
                }
            }
            Body::Pdf(pdf) => {
                let keys = frame.keys.then_some(frame.keymap);
                if let Some(line) = self.pdf.ui(ui, pdf, keys, scroll, self.id) {
                    shown.body = Body::Failed(line);
                }
                self.page = self.pdf.rows();
            }
            Body::Media(stream) => {
                let media = MediaFrame {
                    connection: frame.connection,
                    keymap: frame.keys.then_some(frame.keymap),
                    autoplay: frame.settings.preview.autoplay,
                    audio: frame.audio,
                    inside,
                    toggle,
                };
                if let Some(line) = self.media.ui(ui, stream, media) {
                    shown.body = Body::Failed(line);
                }
            }
            Body::TooLarge(line) => {
                ui.add(widgets::warning_message(line.as_str()));
                ui.label(t.ty.body.rich(TOO_LARGE_HINT).color(t.color.text.muted));
            }
            Body::Archive(listing) => {
                if listing.members.is_empty() {
                    ui.label(t.ty.body.rich(EMPTY_ARCHIVE).color(t.color.text.muted));
                }
                let rows = self
                    .archive_rows
                    .get_or_insert_with(|| archive_rows(listing, frame.zone));
                let line = |i: usize| CodeLine {
                    text: &rows[i],
                    spans: &[],
                };
                self.page = widgets::code_view(self.id.with("archive"), rows.len(), &line)
                    .scroll_rows(scroll * self.page as f32)
                    .show(ui);
            }
            Body::Text(text) => {
                self.page = text_view(ui, self.id, text, scroll * self.page as f32);
            }
            Body::Markdown(markdown) => {
                let views = [
                    (MarkdownView::Rendered, "Rendered"),
                    (MarkdownView::Source, "Source"),
                ];
                ui.add(widgets::segmented(&mut markdown.view, &views));
                ui.add_space(t.space.stack);
                match markdown.view {
                    MarkdownView::Source => {
                        let rows = scroll * self.page as f32;
                        self.page = text_view(ui, self.id, &mut markdown.text, rows);
                    }
                    MarkdownView::Rendered => {
                        if let Some(banner) = markdown.text.fetched.banner() {
                            ui.add(widgets::warning_message(banner));
                        }
                        let id = self.id.with("markdown");
                        let rendered = &markdown.rendered;
                        crate::markdown::show(ui, id, &mut self.markdown, rendered, scroll);
                    }
                }
            }
        }
        leave
    }

    /// The panel's keys: Escape, the cycle keys, and the move keys, which scroll by a
    /// line, a page, or to either end. Scrolls are returned in pages.
    fn keys(&mut self, ui: &mut egui::Ui, keymap: &KeyMap) -> (Leave, f32) {
        let page = self.page.max(1) as f32;
        ui.input_mut(|i| {
            let mut leave = Leave::Stay;
            if i.consume_key(Modifiers::NONE, Key::Escape) {
                leave = Leave::Back;
            } else if keymap.take(i, Action::CycleFocus) {
                leave = Leave::Next;
            } else if keymap.take(i, Action::CycleFocusBack) {
                leave = Leave::Previous;
            }
            let mut rows = 0.0;
            let steps = [
                (Action::MoveUp, -1.0),
                (Action::MoveDown, 1.0),
                (Action::PageUp, -page),
                (Action::PageDown, page),
                (Action::MoveFirst, -1e9),
                (Action::MoveLast, 1e9),
            ];
            for (action, by) in steps {
                if keymap.take(i, action) {
                    rows += by;
                }
            }
            (leave, rows / page)
        })
    }

    /// Takes a read that has arrived.
    fn receive(&mut self, place: &Place) {
        let Some((id, request)) = &self.reading else {
            return;
        };
        let Some(answer) = request.try_take() else {
            return;
        };
        let id = *id;
        self.reading = None;
        self.archive_rows = None;
        self.preview.arrive(id, answer, place);
    }

    /// Starts highlighting shown source for the Theme in force, if it is not yet.
    fn highlight(&mut self, ui: &egui::Ui) {
        let dark = ui.ctx().theme() == egui::Theme::Dark;
        let Some(shown) = self.preview.shown_mut() else {
            return;
        };
        let Some(text) = shown.body.text_mut() else {
            return;
        };
        let Some(syntax) = text.syntax else {
            return;
        };
        if text.highlighting.as_ref().is_some_and(|h| h.dark == dark) {
            return;
        }
        let ctx = ui.ctx().clone();
        text.highlighting = Some(Highlighting::start(
            text.text.clone(),
            text.lines.clone(),
            syntax,
            dark,
            move || ctx.request_repaint(),
        ));
    }
}

/// A text's lines under the size cap's banner, scrolled `rows` further. Returns the rows
/// in view.
fn text_view(ui: &mut egui::Ui, id: Id, text: &mut Text, rows: f32) -> usize {
    if let Some(banner) = text.fetched.banner() {
        ui.add(widgets::warning_message(banner));
    }
    if let Some(h) = &mut text.highlighting {
        h.poll();
    }
    let spans = text.highlighting.as_ref().and_then(|h| h.spans());
    let line = |i: usize| CodeLine {
        text: text.line(i),
        spans: spans
            .and_then(|s| s.get(i))
            .map_or(&[][..], |s| s.as_slice()),
    };
    widgets::code_view(id.with("text"), text.line_count(), &line)
        .scroll_rows(rows)
        .show(ui)
}

/// One row per archive member: size, modified time (in `zone` when the format records
/// an instant) and path, in columns.
fn archive_rows(listing: &Listing, zone: &jiff::tz::TimeZone) -> Vec<String> {
    listing
        .members
        .iter()
        .map(|m| {
            let size = if m.is_dir {
                String::new()
            } else {
                size_text(m.size)
            };
            let modified = match m.modified {
                Some(Stamp::Unix(s)) => u32::try_from(s)
                    .map(|s| modified_text(s, zone))
                    .unwrap_or_default(),
                Some(Stamp::Civil {
                    year,
                    month,
                    day,
                    hour,
                    minute,
                }) => format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}"),
                None => String::new(),
            };
            format!("{size:>10}  {modified:<16}  {}", m.path)
        })
        .collect()
}

/// The details header: name, kind, size, modified, permissions, owner and group, and a
/// symlink's target.
fn details(
    ui: &mut egui::Ui,
    focused: &Focused,
    kind: crate::kind::Kind,
    zone: &jiff::tz::TimeZone,
) {
    let mut rows = vec![
        ("Name", focused.name.clone()),
        ("Kind", preview::kind_name(kind).to_owned()),
    ];
    if let Some(e) = &focused.entry {
        if let Some(size) = e.size.filter(|_| !e.is_dir) {
            rows.push(("Size", size_text(size)));
        }
        if let Some(at) = e.modified {
            rows.push(("Modified", modified_text(at, zone)));
        }
        if let Some(mode) = e.permissions {
            rows.push(("Permissions", permissions_text(mode)));
        }
        if let Some(uid) = e.uid {
            rows.push(("Owner", uid.to_string()));
        }
        if let Some(gid) = e.gid {
            rows.push(("Group", gid.to_string()));
        }
        if let Some(target) = &e.link {
            rows.push(("Link to", target.clone()));
        }
    }
    widgets::form(ui.scope_id().with("details")).show(ui, |ui| {
        for (name, value) in rows {
            widgets::form_row(ui, name, |ui| ui.add(widgets::detail_value(name, value)));
        }
    });
}
