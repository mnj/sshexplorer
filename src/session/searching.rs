//! A Session's Search: the toolbar's search field, starting, stopping and clearing a
//! Search, taking its Matches as they arrive, and the Matches shown in place of the
//! listing. It is `Session`'s own code, kept apart from browsing, Download and
//! reconnecting.

use super::*;

impl Session {
    /// The search field's text.
    pub fn search_text(&mut self) -> &mut String {
        &mut self.search_text
    }

    /// The search field's id, to give it the keyboard.
    pub fn search_id(&self) -> Id {
        self.id.with("search")
    }

    /// Gives the search field the keyboard on the next frame: Ctrl+F. Nothing while
    /// Disconnected.
    pub fn focus_search(&mut self) {
        self.focus_search = !self.is_disconnected();
    }

    /// Whether the search field takes the keyboard this frame, and forgets it.
    pub fn take_focus_search(&mut self) -> bool {
        std::mem::take(&mut self.focus_search)
    }

    /// The toolbar's search field, and Stop while a Search runs. Enter starts a Search;
    /// Down moves into the Matches (or the listing), and the cycle keys move on round
    /// the Session.
    pub fn search_field(&mut self, ui: &mut egui::Ui, keymap: &KeyMap, settings: &AppSettings) {
        let id = self.search_id();
        if self.take_focus_search() {
            ui.memory_mut(|m| m.request_focus(id));
        }
        if ui.memory(|m| m.has_focus(id)) {
            let (down, next, back) = ui.input_mut(|i| {
                (
                    keymap.take(i, Action::MoveDown),
                    keymap.take(i, Action::CycleFocus),
                    keymap.take(i, Action::CycleFocusBack),
                )
            });
            if down || back {
                self.focus_listing = true;
                ui.memory_mut(|m| m.surrender_focus(id));
            } else if next {
                self.focus_preview = true;
                ui.memory_mut(|m| m.surrender_focus(id));
            }
        }
        let field = ui
            .add_enabled(
                !self.is_disconnected(),
                widgets::text_field(&mut self.search_text)
                    .id(id)
                    .search()
                    .hint("Search names"),
            )
            .accessible_name("Search");
        if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            self.search(settings);
            field.request_focus();
        }
        if self.is_searching()
            && ui
                .add(widgets::button("Stop").flat())
                .tooltip("Stop the Search")
                .clicked()
        {
            self.stop_search();
        }
    }

    /// Starts a Search for the search field's text from the Current directory, stopping
    /// one that runs. Nothing when the field is empty, or while Disconnected.
    pub fn search(&mut self, settings: &AppSettings) {
        let text = self.search_text.trim();
        if text.is_empty() || self.is_disconnected() {
            return;
        }
        let query = Query {
            start: self.place(),
            text: text.to_owned(),
            follow: settings.search.follow_links,
            cap: settings.search.match_cap.max(1) as usize,
        };
        self.run_search(Matches::new(query.clone()), query);
    }

    pub(super) fn run_search(&mut self, matches: Matches, query: Query) {
        // A Search still running stops as its handle drops.
        self.search = Some(Searching {
            matches,
            running: Some(search::start(&self.connection, query)),
        });
    }

    /// Stops the running Search: the toolbar's Stop, or Escape. Its Matches stay.
    pub fn stop_search(&mut self) {
        if let Some(running) = self.search.as_ref().and_then(|s| s.running.as_ref()) {
            running.stop();
        }
    }

    /// Whether a Search is running.
    pub fn is_searching(&self) -> bool {
        self.search.as_ref().is_some_and(|s| s.running.is_some())
    }

    /// Whether Matches show in place of the listing, running or ended.
    pub fn shows_matches(&self) -> bool {
        self.search.is_some()
    }

    /// Clears the Search and shows the listing again.
    pub fn clear_search(&mut self) {
        self.search = None;
        self.focus_listing = true;
    }

    /// Takes what the running Search has said.
    pub(super) fn receive_search(&mut self) {
        let Some(searching) = &mut self.search else {
            return;
        };
        let Some(running) = &searching.running else {
            return;
        };
        while let Some(event) = running.try_next() {
            match event {
                search::Event::Found(batch) => searching.matches.found(batch),
                search::Event::Ended(ended) => {
                    searching.matches.end(ended);
                    searching.running = None;
                    break;
                }
            }
        }
    }

    /// The connection dropped: a Search still running ends as lost, keeping its Matches.
    /// The engine usually says so itself; this covers a drop noticed here first.
    pub(super) fn search_lost(&mut self) {
        self.receive_search();
        let Some(searching) = &mut self.search else {
            return;
        };
        if searching.running.take().is_some() {
            searching.matches.end(search::Ended {
                how: How::Lost,
                unreadable: 0,
            });
        }
    }

    /// Opens the Match at the absolute `path`, which ends the Search. A directory
    /// becomes the Current directory and a file is focused in its parent, which is
    /// fetched first when its kept listing does not list it.
    pub(super) fn open_match(&mut self, path: String, is_dir: bool) {
        self.search = None;
        self.focus_listing = true;
        self.reveal = true;
        let Some((dir, name)) = folder::parent(&path) else {
            return;
        };
        let stale = self
            .tree
            .listing(&dir)
            .is_some_and(|l| !l.entries.iter().any(|e| e.name == name));
        match self.view {
            View::Tree => {
                let mut fetch = self.tree.reveal(&dir, Some(&name));
                if stale && !fetch.contains(&dir) {
                    fetch.push(dir);
                }
                self.fetch(fetch);
            }
            View::Folder if is_dir => self.list(path, None),
            View::Folder if stale => {
                self.listing = Some(FolderListing::start(&self.connection, dir, Some(name)));
            }
            View::Folder => self.list(dir, Some(name)),
        }
    }

    /// The Matches in place of the listing: a status line, then one row per Match with
    /// its path from the start and its kind.
    pub(super) fn matches_view(
        &mut self,
        ui: &mut egui::Ui,
        focused: bool,
        reveal: bool,
        show_icons: bool,
    ) {
        let Some(searching) = &self.search else {
            return;
        };
        let matches = &searching.matches;
        let count = matches.len();
        let counted = if count == 1 {
            "1 match".to_owned()
        } else {
            format!("{count} matches")
        };
        let t = ui.tokens();
        match matches.ended() {
            None => {
                ui.horizontal(|ui| {
                    ui.add(widgets::spinner());
                    ui.label(t.ty.body.rich(format!("Searching… {counted}")));
                });
            }
            Some(ended) => {
                match ended.how {
                    How::Finished => {
                        ui.add(widgets::muted(counted));
                    }
                    How::Stopped => {
                        ui.add(widgets::muted(format!("Search stopped. {counted}")));
                    }
                    How::Capped => {
                        ui.add(widgets::warning_message(format!(
                            "Stopped at {count} matches. Narrow the search."
                        )));
                    }
                    How::Unavailable => {
                        ui.add(widgets::warning_message(UNAVAILABLE));
                    }
                    How::Lost => {
                        ui.add(widgets::warning_message(SEARCH_LOST));
                    }
                }
                match ended.unreadable {
                    0 => {}
                    1 => {
                        ui.add(widgets::warning_message("1 directory could not be read"));
                    }
                    n => {
                        ui.add(widgets::warning_message(format!(
                            "{n} directories could not be read"
                        )));
                    }
                }
            }
        }
        let live = focused && ui.input(|i| i.focused);
        let mut command = None;
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                widgets::table(self.id.with("matches"))
                    .columns([widgets::Column::fill(), widgets::Column::auto()])
                    .show(ui, |table| {
                        table.header(|h| {
                            h.text("Path");
                            h.text("Kind");
                        });
                        let focus = matches.focus();
                        let shown = reveal.then_some(focus).flatten();
                        table.rows(count, shown, |table, i| {
                            let m = matches.get(i);
                            let ringed = focused && Some(i) == focus;
                            let look = RowLook::of(matches.is_selected(i), live, ringed);
                            let kind = Kind::of(matches::name(&m.path), m.is_dir);
                            let response = table.row_with(look, |row| {
                                row.ui(|ui| {
                                    ui.add(widgets::entry_name(&m.path, kind).icon(show_icons));
                                });
                                row.muted(if m.is_dir { "Directory" } else { "File" });
                            });
                            let label = m.path.clone();
                            response.ctx.accesskit_node_builder(response.id, |node| {
                                node.set_label(label);
                            });
                            if reveal && Some(i) == focus {
                                response.scroll_to_me(None);
                            }
                            if response.double_clicked() {
                                command = Some(Command::DoubleClick(i));
                            } else if response.clicked() {
                                let m = response.ctx.input(|i| i.modifiers);
                                command = Some(if m.command {
                                    Command::CtrlClick(i)
                                } else if m.shift {
                                    Command::ShiftClick(i)
                                } else {
                                    Command::Click(i)
                                });
                            }
                        });
                    });
            });
        if let Some(command) = command {
            ui.memory_mut(|m| m.request_focus(self.id));
            let now = ui.input(|i| i.time);
            self.apply(command, now);
        }
    }
}
