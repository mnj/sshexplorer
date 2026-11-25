//! Per-session directory history and explicit path navigation.
use super::*;

struct Destination {
    request: Request<(Listing, Option<String>)>,
    history_index: Option<usize>,
    path: String,
}

#[derive(Default)]
pub(super) struct Navigation {
    history: Vec<String>,
    index: usize,
    pending: Option<Destination>,
    editor: Option<String>,
    focus: bool,
    error: Option<String>,
}

impl Session {
    pub(super) fn remember_place(&mut self) {
        if self.navigation.pending.is_some() || (self.view == View::Tree && self.tree.revealing()) {
            return;
        }
        let path = self.place();
        let nav = &mut self.navigation;
        if nav.history.get(nav.index) == Some(&path) {
            return;
        }
        nav.history.truncate(nav.index + 1);
        nav.history.push(path);
        nav.index = nav.history.len() - 1;
    }

    pub(super) fn cancel_navigation(&mut self) {
        self.navigation.pending = None;
        self.navigation.error = None;
    }

    fn history_index(&self, step: isize) -> Option<usize> {
        let at = self
            .navigation
            .pending
            .as_ref()
            .and_then(|p| p.history_index)
            .unwrap_or(self.navigation.index);
        at.checked_add_signed(step)
            .filter(|&i| i < self.navigation.history.len())
    }

    pub fn can_move(&self, step: isize) -> bool {
        self.history_index(step).is_some() && !self.is_disconnected()
    }

    pub fn move_in_history(&mut self, step: isize) {
        if let Some(i) = self.history_index(step) {
            self.navigate(self.navigation.history[i].clone(), Some(i));
        }
    }

    pub fn edit_path(&mut self) {
        self.navigation.editor = Some(self.place());
        self.navigation.focus = true;
    }

    fn navigate(&mut self, path: String, history_index: Option<usize>) {
        if self.is_disconnected() {
            return;
        }
        self.remember_place();
        self.retire_folder_refresh();
        self.listing = None;
        self.navigation.error = None;
        self.navigation.pending = Some(Destination {
            request: self.connection.locate(path.clone()),
            history_index,
            path,
        });
    }

    pub(super) fn receive_navigation(&mut self) {
        let Some(result) = self
            .navigation
            .pending
            .as_ref()
            .and_then(|p| p.request.try_take())
        else {
            return;
        };
        let destination = self.navigation.pending.take().unwrap();
        match result {
            Ok((listing, focus)) => {
                let path = listing.path.clone();
                let hidden_directory =
                    self.view == View::Tree && path.split('/').any(|name| name.starts_with('.'));
                if !self.show_hidden
                    && (hidden_directory
                        || focus.as_ref().is_some_and(|name| name.starts_with('.')))
                {
                    self.navigation.error = Some(format!(
                        "Enable Show hidden entries to open {}.",
                        destination.path
                    ));
                    return;
                }
                self.clear_search();
                self.tree.listed(&path, Ok(listing.clone()));
                self.show_folder(listing, focus.clone());
                if self.view == View::Tree {
                    let fetch = self.tree.reveal_known(&path, focus.as_deref());
                    self.fetch(fetch);
                }
                if let Some(i) = destination.history_index {
                    self.navigation.index = i;
                }
                self.focus_listing = true;
            }
            Err(e) => {
                self.navigation.error = Some(format!("Cannot open {}: {e}", destination.path))
            }
        }
    }

    pub fn path_bar(&mut self, ui: &mut egui::Ui) {
        if let Some(text) = &mut self.navigation.editor {
            let mut go = false;
            let mut cancel = false;
            let id = self.id.with("path");
            ui.horizontal(|ui| {
                let enter = ui.memory(|m| m.has_focus(id))
                    && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
                let field = ui
                    .add(
                        widgets::text_field(text)
                            .id(id)
                            .width((ui.available_width() - 145.).max(60.)),
                    )
                    .accessible_name("Remote path");
                if std::mem::take(&mut self.navigation.focus) {
                    field.request_focus();
                    if let Some(mut state) = egui::text_edit::TextEditState::load(ui.ctx(), id) {
                        use egui::text::{CCursor, CCursorRange};
                        state.cursor.set_char_range(Some(CCursorRange::two(
                            CCursor::new(0),
                            CCursor::new(text.chars().count()),
                        )));
                        state.store(ui.ctx(), id);
                    }
                }
                go = ui.add(widgets::button("Go")).clicked() || enter;
                cancel = ui.add(widgets::button("Cancel")).clicked()
                    || ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            });
            if go && !text.is_empty() {
                let path = self.navigation.editor.take().unwrap();
                let path = if path.starts_with('/') {
                    path
                } else {
                    folder::join(&self.place(), &path)
                };
                self.navigate(path, None);
            } else if cancel {
                self.navigation.editor = None;
                self.focus_listing = true;
            }
        }
        if let Some(error) = &self.navigation.error {
            ui.add(widgets::invalid_message(error));
        }
    }
}
