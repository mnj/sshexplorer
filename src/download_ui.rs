//! A Session's Download chrome: the strip along its bottom, the conflict prompt that is
//! modal to that Session only, and the Details window.
//!
//! The strip shows the running Download: the current file, files and bytes done of the
//! total ("counting…" until known), the rate, Cancel, and "+N queued". A clean finish
//! says "Downloaded N files" and clears after [`CLEAR_AFTER`]; a skip, failure, cancel
//! or stop keeps the strip, with Details, until dismissed.

use egui::{Context, Id, Key, Rect};

use crate::download::{Choice, End, Queue, Summary};
use crate::folder::size_text;
use crate::theme::TokensExt as _;
use crate::widgets::{self, Tooltip as _};

/// How long "Downloaded N files" stays, in seconds.
pub const CLEAR_AFTER: f64 = 4.0;

/// The Session's Download chrome state across frames.
#[derive(Default)]
pub struct Panel {
    /// The conflict prompt's New name, and the automatic name it started from.
    rename: String,
    suggestion: Option<String>,
    apply_all: bool,
    details: bool,
    /// When the shown summary arrived, in egui seconds.
    finished_at: Option<f64>,
}

impl Panel {
    /// A Download ended at `now`.
    pub fn ended(&mut self, now: f64) {
        self.finished_at = Some(now);
        self.details = false;
    }
}

fn files(n: usize) -> String {
    if n == 1 {
        "1 file".to_owned()
    } else {
        format!("{n} files")
    }
}

/// What the strip says about a finished Download.
fn summary_text(s: &Summary) -> String {
    let mut text = match &s.end {
        End::Finished => format!("Downloaded {}", files(s.copied)),
        End::Cancelled => format!("Download cancelled after {}", files(s.copied)),
        End::Stopped(why) => format!("Download stopped after {}: {why}", files(s.copied)),
    };
    if !s.skipped.is_empty() {
        text.push_str(&format!(", {} skipped", s.skipped.len()));
    }
    if !s.failed.is_empty() {
        text.push_str(&format!(", {} failed", s.failed.len()));
    }
    text
}

/// Whether the strip is up: a Download is running or waiting, or the last one's summary
/// is still shown. Drops a clean summary once [`CLEAR_AFTER`] has passed.
pub fn has_strip(queue: &mut Queue, panel: &mut Panel, ctx: &Context, now: f64) -> bool {
    if queue.is_busy() {
        return true;
    }
    let Some(summary) = &queue.finished else {
        return false;
    };
    if summary.is_clean() {
        let at = *panel.finished_at.get_or_insert(now);
        let left = at + CLEAR_AFTER - now;
        if left <= 0.0 {
            queue.finished = None;
            panel.finished_at = None;
            return false;
        }
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(left));
    }
    true
}

/// The strip, in a row as wide as `ui`.
pub fn strip(ui: &mut egui::Ui, queue: &mut Queue, panel: &mut Panel) {
    let t = ui.tokens();
    let mut cancel = false;
    let mut dismiss = false;
    widgets::status_bar(ui, |ui| {
        // The bar packs its items edge to edge; the strip's stand a gap apart.
        let gap = |ui: &mut egui::Ui| ui.add_space(t.space.gap);
        let muted = |ui: &mut egui::Ui, text: String| {
            gap(ui);
            ui.label(t.ty.small.rich(text).color(t.color.text.muted));
        };
        gap(ui);
        if let Some(download) = queue.running() {
            let p = download.progress();
            let current = p.current.clone().unwrap_or_else(|| "Download".to_owned());
            ui.label(t.ty.small.rich(current).color(t.color.text.primary));
            match (p.files_total, p.bytes_total) {
                (Some(files), Some(bytes)) => {
                    muted(ui, format!("{} of {files} files", p.files_done));
                    muted(
                        ui,
                        format!("{} of {}", size_text(p.bytes_done), size_text(bytes)),
                    );
                }
                _ => muted(ui, "counting…".to_owned()),
            }
            muted(ui, format!("{}/s", size_text(p.rate() as u64)));
            gap(ui);
            cancel = ui.add(widgets::button("Cancel").flat()).clicked();
            if queue.queued() > 0 {
                muted(ui, format!("+{} queued", queue.queued()));
            }
            // Progress arrives off the frame thread, which wakes the frame for it.
        } else if let Some(summary) = &queue.finished {
            ui.label(
                t.ty.small
                    .rich(summary_text(summary))
                    .color(t.color.text.primary),
            );
            if !summary.is_clean() {
                gap(ui);
                if ui.add(widgets::button("Details").flat()).clicked() {
                    panel.details = true;
                }
                dismiss = ui
                    .add(widgets::button("Dismiss").flat())
                    .tooltip("Clear the strip")
                    .clicked();
            }
        }
    });
    if cancel {
        queue.cancel();
    }
    if dismiss {
        queue.finished = None;
        panel.finished_at = None;
        panel.details = false;
    }
}

/// The Details window for the shown summary, when it is open.
pub fn details(ctx: &Context, id: Id, queue: &Queue, panel: &mut Panel) {
    let Some(summary) = queue.finished.as_ref().filter(|_| panel.details) else {
        panel.details = false;
        return;
    };
    let mut open = true;
    widgets::window(id, "Download details")
        .open(&mut open)
        .centred()
        .show(ctx, |ui| {
            let t = ui.tokens();
            let line = |ui: &mut egui::Ui, text: String| {
                ui.label(t.ty.body.rich(text).color(t.color.text.primary));
            };
            line(ui, format!("Copied: {}", summary.copied));
            line(ui, format!("Skipped: {}", summary.skipped.len()));
            for note in &summary.skipped {
                line(ui, format!("  {} — {}", note.path, note.reason));
            }
            line(ui, format!("Failed: {}", summary.failed.len()));
            for note in &summary.failed {
                line(ui, format!("  {} — {}", note.path, note.reason));
            }
            if let End::Stopped(why) = &summary.end {
                line(ui, format!("Stopped: {why}"));
            }
        });
    let escape = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape));
    if !open || escape {
        panel.details = false;
    }
}

/// The conflict prompt, centred on `area`, the Session's part of the window. Escape
/// answers Cancel when `keys`.
pub fn conflict(
    ctx: &Context,
    id: Id,
    area: Rect,
    keys: bool,
    queue: &mut Queue,
    panel: &mut Panel,
) {
    let Some(conflict) = &queue.conflict else {
        panel.suggestion = None;
        return;
    };
    if panel.suggestion.as_deref() != Some(&conflict.suggestion) {
        panel.rename = conflict.suggestion.clone();
        panel.suggestion = Some(conflict.suggestion.clone());
    }
    let edited = panel.rename != conflict.suggestion;
    if edited {
        panel.apply_all = false;
    }
    let where_ = conflict
        .path
        .parent()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let message = format!("{} already exists in {where_}.", conflict.name);
    let mut choice = None;
    let shown = widgets::window(id, "File already exists")
        .closable()
        .centred()
        .default_pos(area.center())
        .show(ctx, |ui| {
            let t = ui.tokens();
            ui.label(t.ty.body.rich(&message).color(t.color.text.primary));
            widgets::form(id.with("form")).show(ui, |ui| {
                widgets::form_row(ui, "New name", |ui| {
                    ui.add(widgets::text_field(&mut panel.rename))
                        .accessible_name("New name");
                });
            });
            ui.add_enabled_ui(!edited, |ui| {
                ui.add(widgets::checkbox(
                    &mut panel.apply_all,
                    "Apply to all remaining",
                ))
                .disabled_tooltip("Not with a name edited by hand");
            });
            widgets::button_row(ui, |ui| {
                let name = panel.rename.trim();
                let valid =
                    !name.is_empty() && !name.contains(['/', '\\']) && name != "." && name != "..";
                if ui
                    .add_enabled(valid, widgets::button("Rename").primary())
                    .clicked()
                {
                    choice = Some(Choice::Rename(name.to_owned()));
                }
                if ui.add(widgets::button("Overwrite")).clicked() {
                    choice = Some(Choice::Overwrite);
                }
                if ui.add(widgets::button("Skip")).clicked() {
                    choice = Some(Choice::Skip);
                }
                if ui.add(widgets::button("Cancel")).clicked() {
                    choice = Some(Choice::Cancel);
                }
            });
        });
    let escape = keys && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape));
    if shown.is_some_and(|s| s.close_clicked) || escape {
        choice = Some(Choice::Cancel);
    }
    if let Some(choice) = choice
        && let Some(conflict) = queue.conflict.take()
    {
        let all = panel.apply_all && choice != Choice::Cancel;
        conflict.answer(choice, all);
        panel.apply_all = false;
        panel.suggestion = None;
    }
}
