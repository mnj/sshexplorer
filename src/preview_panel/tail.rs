//! Tail mode belongs to one focused path and keeps only the last window in memory.
use crate::ssh::{
    Request, Session,
    tail::{Chunk, Cursor, WINDOW},
};
use crate::widgets::{self, CodeLine};
use std::time::Duration;

#[derive(Default)]
pub(super) struct Tail {
    cursor: Option<Cursor>,
    reading: Option<Request<Chunk>>,
    bytes: Vec<u8>,
    lines: Vec<String>,
    pub follow: bool,
    due: f64,
    last_snapshot: f64,
    error: Option<String>,
}

impl Tail {
    pub fn latest(&mut self) {
        self.due = 0.0;
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        id: egui::Id,
        path: &str,
        session: &Session,
        offline: bool,
        scroll: f32,
    ) -> usize {
        let now = ui.input(|i| i.time);
        let mut bottom = false;
        if offline {
            self.reading = None;
            self.error = Some("Connection lost".into());
            self.latest();
        } else {
            if let Some(result) = self.reading.as_ref().and_then(Request::try_take) {
                self.reading = None;
                self.due = now + 1.0;
                match result {
                    Ok(chunk) => {
                        self.error = None;
                        if chunk.replace || !chunk.bytes.is_empty() {
                            if chunk.replace {
                                self.bytes.clear();
                                self.last_snapshot = now;
                            }
                            self.bytes.extend(chunk.bytes);
                            self.bytes.drain(..self.bytes.len().saturating_sub(WINDOW));
                            self.lines = String::from_utf8_lossy(&self.bytes)
                                .lines()
                                .map(str::to_owned)
                                .collect();
                            bottom = true;
                        }
                        self.cursor = Some(chunk.cursor);
                    }
                    Err(e) => self.error = Some(e),
                }
            }
            if self.reading.is_none() {
                if self.due == 0.0 || (self.follow && now >= self.due) {
                    // SFTP v3 has no portable inode. Periodically reconcile the full
                    // window even if a replacement has the same size, timestamp and suffix.
                    let cursor = if self.due == 0.0 || now - self.last_snapshot >= 30.0 {
                        None
                    } else {
                        self.cursor.clone()
                    };
                    self.reading = Some(session.tail(path.to_owned(), cursor));
                } else if self.follow {
                    ui.ctx()
                        .request_repaint_after(Duration::from_secs_f64((self.due - now).max(0.0)));
                }
            }
        }
        if let Some(e) = &self.error {
            ui.add(widgets::invalid_message(e.as_str()));
        } else if self.cursor.is_none() {
            ui.add(widgets::loading_line("Reading latest lines…"));
        }
        let line = |i: usize| CodeLine {
            text: &self.lines[i],
            spans: &[],
        };
        widgets::code_view(id.with("tail"), self.lines.len(), &line)
            .scroll_rows(if bottom {
                self.lines.len() as f32
            } else {
                scroll
            })
            .show(ui)
    }
}
