//! An open Session: its connection, its tab label, and its Tree view and Folder view.
//!
//! Both views are a table with the columns Name, Size, Modified and Permissions, with
//! the Kind icons, sorted by clicking a header; Tree view indents within Name. What the
//! keys and the pointer do is [`Tree`]'s or [`Folder`]'s; this module turns them into
//! [`Command`]s, asks the host for the directories they need, and draws the result.
//!
//! A Session opens in Tree view. Every directory fetched is kept in the [`Tree`], and
//! Folder view lists a kept directory without fetching it again. Switching views keeps
//! the place: Tree view to Folder view lists the Current directory with the same entry
//! focused, and Folder view to Tree view expands to it and focuses the same entry.
//!
//! A Refresh (F5, the toolbar button, or Auto-refresh) fetches the Current directory
//! again, and in Tree view every expanded directory below it, and merges the answers in
//! ([`Tree::refreshed`], [`Folder::refreshed`]); the old rows stay until then. Folder
//! view on a directory that no longer exists lists the nearest ancestor that does, with
//! one line naming the one that went. Each Session refreshes only itself.
//!
//! When the connection drops the Session is Disconnected: a strip above the listing says
//! why and how reconnecting goes ([`Lost`]), the kept listings stay browsable but greyed,
//! Refresh does nothing and F5 means Reconnect. A reconnect that needs the operator (a
//! prompt, a changed key) hands over to the same dialogs as opening; one that succeeds
//! clears the strip and refreshes the Current directory.
//!
//! Preview, on the right, shows the Focused entry of either view; Enter on a file and
//! Tab move the keyboard there.
//!
//! A Search (the toolbar's search field, Ctrl+F then Enter) runs from the Current
//! directory, and its [`Matches`] replace the listing until it is cleared: they focus,
//! select, preview and download like the listing, and Enter on one ends the Search and
//! goes there. While it runs, Refresh does nothing; once it has ended, Refresh runs it
//! again. Auto-refresh waits while Matches show.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use egui::{Align, Event, EventFilter, Id, Layout, Sense};

use crate::download::{self, Queue};
use crate::download_ui;

use crate::connect::{Outcome, Pending};
use crate::folder::{
    self, Column, Command, Effect, Folder, Sort, modified_text, permissions_text, size_text,
};
use crate::keymap::{Action, KeyMap};
use crate::kind::Kind;
use crate::matches::{self, Matches};
use crate::preview::Focused;
use crate::preview_panel::{Leave, PanelFrame, PreviewPanel};
use crate::reconnect::Lost;
use crate::refresh::Timer;
use crate::search::{self, How, Query};
use crate::settings::{AppSettings, ReconnectAttempts};
use crate::ssh::{self, Entry, Listing, NO_SUCH_FILE, Request, Target};
use crate::theme::TokensExt as _;
use crate::tree::{self, Line, Tree};
use crate::widgets::{self, Glyph, RowLook, Selection, State, Tooltip as _};

/// How a Session shows the remote filesystem. The choice belongs to the Session.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    Tree,
    Folder,
}

/// What the Session needs from the app to draw a frame.
pub struct Frame<'a> {
    pub keymap: &'a KeyMap,
    pub settings: &'a AppSettings,
    /// Whether the listing may take keys: no dialog is up.
    pub keys: bool,
    /// Where Preview sends audio.
    pub audio: crate::media::AudioOut,
}

/// What a Session takes from the Host it was opened from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Details {
    /// The tab's label: a saved Host's display name, else [`label`].
    pub label: String,
    /// The tab's tooltip: for a saved Host, where it connects, port and all.
    pub tooltip: Option<String>,
    pub readonly: bool,
    pub local_folder: PathBuf,
    /// Why the OpenSSH config was not applied, naming the file and line.
    pub note: Option<String>,
    /// A note the operator can dismiss, such as the credential store not keeping the
    /// secret.
    pub dismissible: Option<String>,
}

pub struct Session {
    pub details: Details,
    view: View,
    /// Every directory fetched so far, as Tree view shows them.
    tree: Tree,
    /// Directories Tree view is fetching.
    fetches: Vec<(String, Request<Listing>)>,
    folder: Folder,
    sort: Sort,
    /// The "Show hidden entries" setting the views were last arranged with.
    show_hidden: bool,
    /// A directory being listed for Folder view, and the entry to focus in it.
    listing: Option<(Request<Listing>, Option<String>)>,
    /// Why the last directory asked for could not be listed.
    error: Option<String>,
    /// Directories a Tree view Refresh is fetching again.
    refreshes: Vec<(String, Request<Listing>)>,
    /// Folder view's Refresh, while it runs.
    folder_refresh: Option<FolderRefresh>,
    /// A Refresh finished since the last frame: the Auto-refresh interval restarts.
    refreshed: bool,
    /// "<path> no longer exists.", above the ancestor Folder view lists instead.
    notice: Option<String>,
    timer: Timer,
    /// The listing's own focus target: it holds the keyboard while it has egui's focus.
    id: Id,
    /// The listing takes the keyboard on the next frame: a new Session focuses it.
    focus_listing: bool,
    /// The Focused entry moved by key; scroll it into view.
    reveal: bool,
    /// The Preview panel, dropped before the connection its read uses.
    preview: PreviewPanel,
    /// Preview takes the keyboard on the next frame: Enter on a file.
    focus_preview: bool,
    /// The Session's Downloads; dropped, and so cancelled, before the connection.
    downloads: Queue,
    downloads_panel: download_ui::Panel,
    /// While Disconnected: why, and how reconnecting goes.
    lost: Option<Lost>,
    /// The reconnect attempt running.
    reconnecting: Option<ssh::Opening>,
    /// A reconnect that needs the operator: a prompt, or a refusal to read.
    handover: Option<Pending>,
    /// Directories whose fetch the drop cut short, fetched again on reconnect.
    refetch: Vec<String>,
    connection: ssh::Session,
    /// The operator's time zone, for the Modified column.
    zone: jiff::tz::TimeZone,
    /// What the search field holds.
    search_text: String,
    /// The Search's Matches, shown in place of the listing, and the Search while it runs.
    search: Option<Searching>,
    /// The search field takes the keyboard on the next frame.
    focus_search: bool,
}

/// A Search and its Matches. The Search is dropped, and so stopped, when it ends.
struct Searching {
    matches: Matches,
    running: Option<search::Search>,
}

impl Session {
    pub fn new(details: Details, opened: ssh::Opened) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let sort = Sort::default();
        let id = Id::NULL.with("sshexplorer-listing").with(n);
        let mut tree = Tree::new(sort, false);
        let fetches = tree
            .open_at(opened.home.clone())
            .into_iter()
            .map(|path| (path.clone(), opened.session.list(path)))
            .collect();
        Session {
            details,
            view: View::Tree,
            tree,
            fetches,
            folder: Folder::new(opened.home, sort, false),
            sort,
            show_hidden: false,
            listing: None,
            error: None,
            refreshes: Vec::new(),
            folder_refresh: None,
            refreshed: false,
            notice: None,
            timer: Timer::default(),
            id,
            preview: PreviewPanel::new(id.with("preview")),
            focus_preview: false,
            focus_listing: true,
            reveal: false,
            downloads: Queue::default(),
            downloads_panel: download_ui::Panel::default(),
            lost: None,
            reconnecting: None,
            handover: None,
            refetch: Vec::new(),
            connection: opened.session,
            zone: jiff::tz::TimeZone::system(),
            search_text: String::new(),
            search: None,
            focus_search: false,
        }
    }

    pub fn label(&self) -> &str {
        &self.details.label
    }

    /// Queues a Download of the Selection into the Local folder. Nothing when the
    /// Selection is empty.
    pub fn download(&mut self, options: download::Options) {
        if self.is_disconnected() {
            return;
        }
        let Some((dir, entries)) = self.selection() else {
            return;
        };
        let job = download::Job {
            dir,
            entries,
            local_folder: self.details.local_folder.clone(),
            options,
        };
        self.downloads.push(&self.connection, job);
    }

    /// Whether there is a Selection to download, and a connection to download it over.
    pub fn can_download(&self) -> bool {
        if self.is_disconnected() {
            return false;
        }
        if let Some(searching) = &self.search {
            return searching.matches.selection().is_some();
        }
        match self.view {
            View::Tree => !self.tree.selection().is_empty(),
            View::Folder => (0..self.folder.len()).any(|i| self.folder.is_selected(i)),
        }
    }

    /// The Selection in the Session's view: the directory its entries share, and the
    /// entries as listed. `None` when nothing is selected.
    fn selection(&self) -> Option<(String, Vec<ssh::Entry>)> {
        if let Some(searching) = &self.search {
            return searching.matches.selection();
        }
        let (dir, entries) = match self.view {
            View::Folder => {
                let entries = (0..self.folder.len())
                    .filter(|i| self.folder.is_selected(*i))
                    .filter_map(|i| self.folder.entry(i).cloned())
                    .collect();
                (self.folder.path().to_owned(), entries)
            }
            View::Tree => {
                let paths = self.tree.selection();
                let (dir, _) = folder::parent(paths.first()?)?;
                // A directory above the Initial remote folder may not be listed yet: it is
                // known only as a directory.
                let entries = (0..self.tree.len())
                    .filter(|i| self.tree.is_selected(*i))
                    .filter_map(|i| match self.tree.row(i).line {
                        tree::Line::Entry { name, entry, .. } => {
                            Some(entry.cloned().unwrap_or_else(|| ssh::Entry {
                                name: name.to_owned(),
                                is_dir: true,
                                ..ssh::Entry::default()
                            }))
                        }
                        _ => None,
                    })
                    .collect();
                (dir, entries)
            }
        };
        let entries: Vec<ssh::Entry> = entries;
        (!entries.is_empty()).then_some((dir, entries))
    }

    /// Whether a Download is running or queued.
    pub fn is_downloading(&self) -> bool {
        self.downloads.is_busy()
    }

    /// Whether a Download waits for the operator to answer a conflict.
    pub fn needs_attention(&self) -> bool {
        self.downloads.conflict.is_some()
    }

    /// The Session's tab is not shown this frame: Preview's playback pauses, as moving
    /// focus away does.
    pub fn hidden(&mut self) {
        self.preview.hidden();
    }

    /// Takes what the Downloads have said and starts the next queued one. Call it every
    /// frame, on screen or not; `now` is egui's time.
    pub fn poll(&mut self, now: f64) {
        if self.downloads.poll(&self.connection) {
            self.downloads_panel.ended(now);
        }
    }

    pub fn view(&self) -> View {
        self.view
    }

    /// The place the toolbar shows: the Current directory.
    pub fn place(&self) -> String {
        match self.view {
            View::Tree => self.tree.current_dir(),
            View::Folder => self.folder.path().to_owned(),
        }
    }

    /// Starts a Refresh, unless one or a Search is running: the Current directory, and in
    /// Tree view every expanded directory below it, fetched again and merged in. While
    /// the Matches of an ended Search show, the Search runs again instead. Nothing while
    /// Disconnected.
    pub fn refresh(&mut self) {
        if self.is_refreshing() || self.is_searching() || self.is_disconnected() {
            return;
        }
        if let Some(searching) = &self.search {
            let matches = searching.matches.rerun();
            let query = matches.query().clone();
            self.run_search(matches, query);
            return;
        }
        match self.view {
            View::Tree => {
                for path in self.tree.refresh_paths() {
                    let request = self.connection.list(path.clone());
                    self.refreshes.push((path, request));
                }
            }
            View::Folder => {
                let path = self.folder.path().to_owned();
                self.folder_refresh = Some(FolderRefresh {
                    request: self.connection.list(path.clone()),
                    path,
                    gone: None,
                });
            }
        }
    }

    /// Whether the connection has dropped and not come back yet.
    pub fn is_disconnected(&self) -> bool {
        self.lost.is_some()
    }

    /// Why the connection dropped, while Disconnected.
    pub fn lost_reason(&self) -> Option<&str> {
        self.lost.as_ref().map(Lost::reason)
    }

    /// Notices a drop, and runs the reconnect attempts as `attempts` allows. Call it
    /// every frame, on screen or not; `now` is egui's time. Gives the seconds until the
    /// strip's countdown next changes, for a repaint.
    pub fn keep_connected(&mut self, now: f64, attempts: ReconnectAttempts) -> Option<f64> {
        if self.lost.is_none() {
            let reason = self.connection.lost()?;
            self.lost = Some(Lost::new(reason, attempts, now));
            self.search_lost();
        }
        if let Some(opening) = self.reconnecting.take() {
            match opening.try_next() {
                None => self.reconnecting = Some(opening),
                Some(ssh::Event::Opened(opened)) => self.reconnected(opened),
                Some(ssh::Event::Failed(ssh::Failure::Connection(_) | ssh::Failure::Sftp(_))) => {
                    if let Some(lost) = &mut self.lost {
                        lost.failed(attempts, now);
                    }
                }
                Some(ssh::Event::Failed(ssh::Failure::Cancelled)) => self.stop_reconnecting(),
                // A prompt, a changed key or a refused login: the operator takes over.
                Some(event) => {
                    self.stop_reconnecting();
                    let id = self.id.with("reconnect");
                    let details = self.details.clone();
                    self.handover = Some(Pending::resume(id, details, opening, event));
                }
            }
        }
        let lost = self.lost.as_mut()?;
        if lost.due(now) {
            self.reconnecting = Some(self.connection.reconnect());
        }
        let wait = self.lost.as_ref()?.wait(now)?;
        // The countdown shows whole seconds.
        Some(wait.min(wait.fract().max(0.05)))
    }

    /// Reconnect now, Reconnect, or F5 while Disconnected: an attempt starts at once,
    /// unless one is running or the operator is being asked.
    pub fn reconnect_now(&mut self) {
        if self.reconnecting.is_some() || self.handover.is_some() {
            return;
        }
        if let Some(lost) = &mut self.lost {
            lost.now();
            self.reconnecting = Some(self.connection.reconnect());
        }
    }

    /// Stop: no more attempts until the operator asks.
    pub fn stop_reconnecting(&mut self) {
        // Dropping the attempt cancels whatever it would ask.
        self.reconnecting = None;
        if let Some(lost) = &mut self.lost {
            lost.stop();
        }
    }

    /// The connection is back: the strip clears, the Current directory is refreshed and
    /// the fetches the drop cut short are made again.
    fn reconnected(&mut self, opened: ssh::Opened) {
        self.connection = opened.session;
        self.lost = None;
        self.reconnecting = None;
        self.handover = None;
        // A Download the drop paused fetches its file again.
        self.downloads.resume(&self.connection);
        let refetch = std::mem::take(&mut self.refetch);
        self.fetch(refetch);
        self.refresh();
    }

    /// Draws the reconnect's prompt, when one needs the operator. Call it every frame.
    pub fn prompt(&mut self, ctx: &egui::Context) {
        let Some(pending) = &mut self.handover else {
            return;
        };
        match pending.show(ctx) {
            Some(Outcome::Opened(opened)) => self.reconnected(opened),
            Some(Outcome::Ended) => self.handover = None,
            None => {}
        }
    }

    /// Whether the reconnect's prompt is up, which takes the input from the window.
    pub fn is_asking(&self) -> bool {
        self.handover.as_ref().is_some_and(Pending::is_modal)
    }

    /// Whether a Refresh is running: the toolbar button shows a spinner.
    pub fn is_refreshing(&self) -> bool {
        !self.refreshes.is_empty() || self.folder_refresh.is_some()
    }

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

    /// Gives the Matches, or the listing, the keyboard: Down from the search field.
    pub fn focus_listing(&mut self) {
        self.focus_listing = true;
    }

    /// Gives Preview the keyboard: Tab from the search field.
    pub fn focus_preview(&mut self) {
        self.focus_preview = true;
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

    fn run_search(&mut self, matches: Matches, query: Query) {
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
    fn receive_search(&mut self) {
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
    fn search_lost(&mut self) {
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
    fn open_match(&mut self, path: String, is_dir: bool) {
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
                self.listing = Some((self.connection.list(dir), Some(name)));
            }
            View::Folder => self.list(dir, Some(name)),
        }
    }

    /// The Matches in place of the listing: a status line, then one row per Match with
    /// its path from the start and its kind.
    fn matches_view(&mut self, ui: &mut egui::Ui, focused: bool, reveal: bool, show_icons: bool) {
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
                        ui.label(t.ty.body.rich(counted).color(t.color.text.muted));
                    }
                    How::Stopped => {
                        ui.label(
                            t.ty.body
                                .rich(format!("Search stopped. {counted}"))
                                .color(t.color.text.muted),
                        );
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
                            let selection = match (matches.is_selected(i), live) {
                                (false, _) => Selection::None,
                                (true, true) => Selection::Live,
                                (true, false) => Selection::Unfocused,
                            };
                            let mut look = RowLook::selected(selection);
                            if focused && Some(i) == focus {
                                look = look.forced(State::Focus);
                            }
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

    /// Switches to `view`, keeping the place, and gives the listing the keyboard.
    pub fn set_view(&mut self, view: View) {
        if view == self.view {
            return;
        }
        self.view = view;
        self.focus_listing = true;
        self.reveal = true;
        match view {
            View::Folder => {
                let dir = self.tree.current_dir();
                // A directory is focused as `.` in its own listing; a file by its name.
                let focus = match self.tree.focused_path() {
                    Some(path) if path != dir => folder::parent(path).map(|(_, name)| name),
                    _ => Some(".".to_owned()),
                };
                self.list(dir, focus);
            }
            View::Tree => {
                let dir = self.folder.path().to_owned();
                let at = self.folder.focus();
                let fetch = match self.folder.line(at) {
                    folder::Line::Here => self.tree.reveal(&dir, None),
                    folder::Line::Up => match folder::parent(&dir) {
                        Some((up, _)) => self.tree.reveal(&up, None),
                        None => self.tree.reveal(&dir, None),
                    },
                    folder::Line::Entry(_) => {
                        let name = self.folder.name(at).to_owned();
                        self.tree.reveal(&dir, Some(&name))
                    }
                };
                self.fetch(fetch);
            }
        }
    }

    /// The Session: its listing in its view, the Download strip along the bottom, and
    /// the Download prompt, modal to this Session only.
    pub fn ui(&mut self, ui: &mut egui::Ui, frame: Frame<'_>) {
        let ctx = ui.ctx().clone();
        let now = ui.input(|i| i.time);
        let strip =
            download_ui::has_strip(&mut self.downloads, &mut self.downloads_panel, &ctx, now);
        let asking = self.needs_attention();
        let area = ui.available_rect_before_wrap();
        let keys = frame.keys;
        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            if strip {
                download_ui::strip(ui, &mut self.downloads, &mut self.downloads_panel);
            }
            ui.with_layout(Layout::top_down(Align::Min), |ui| {
                let attempts = frame.settings.connections.reconnect_attempts;
                self.lost_strip(ui, now, attempts);
                ui.add_enabled_ui(!asking, |ui| {
                    let frame = Frame {
                        keys: keys && !asking,
                        ..frame
                    };
                    self.listing_view(ui, frame);
                });
            });
        });
        download_ui::conflict(
            &ctx,
            self.id.with("conflict"),
            area,
            keys,
            &mut self.downloads,
            &mut self.downloads_panel,
        );
        download_ui::details(
            &ctx,
            self.id.with("details"),
            &self.downloads,
            &mut self.downloads_panel,
        );
    }

    /// The strip above the listing while Disconnected: "Connection lost: <reason>", how
    /// reconnecting goes, and Reconnect now and Stop, or Reconnect once stopped.
    fn lost_strip(&mut self, ui: &mut egui::Ui, now: f64, attempts: ReconnectAttempts) {
        let Some(lost) = &self.lost else {
            return;
        };
        let headline = lost.headline();
        let status = lost.status(now, attempts);
        let (mut now_clicked, mut stop) = (false, false);
        ui.horizontal(|ui| {
            ui.add(widgets::warning_message(&headline));
            // The buttons come before the status, whose width changes as it counts down,
            // so they stay where they are.
            match &status {
                Some(status) => {
                    now_clicked = ui.add(widgets::button("Reconnect now").flat()).clicked();
                    stop = ui.add(widgets::button("Stop").flat()).clicked();
                    let t = ui.tokens();
                    ui.label(t.ty.body.rich(status.as_str()).color(t.color.text.muted));
                }
                None => {
                    now_clicked = ui
                        .add(widgets::button("Reconnect").flat())
                        .tooltip("Connect again (F5)")
                        .clicked();
                }
            }
        });
        if now_clicked {
            self.reconnect_now();
        }
        if stop {
            self.stop_reconnecting();
        }
    }

    /// The listing in the Session's view.
    fn listing_view(&mut self, ui: &mut egui::Ui, frame: Frame<'_>) {
        self.receive();
        self.receive_search();
        self.auto_refresh(ui, frame.settings);
        let show_hidden = frame.settings.browsing.show_hidden;
        if show_hidden != self.show_hidden {
            self.show_hidden = show_hidden;
            self.folder.arrange(self.sort, show_hidden);
            self.tree.arrange(self.sort, show_hidden);
        }

        let t = ui.tokens();
        let focused_entry = self.focused();
        let leave = widgets::splitter(ui, t.space.gap, |ui| {
            egui::Panel::right(self.id.with("preview-panel"))
                .resizable(true)
                .default_size(PREVIEW_WIDTH)
                .frame(egui::Frame::NONE)
                .show(ui, |ui| {
                    self.preview.ui(
                        ui,
                        PanelFrame {
                            connection: &self.connection,
                            disconnected: self.lost.is_some(),
                            focused: focused_entry,
                            keymap: frame.keymap,
                            settings: frame.settings,
                            keys: frame.keys,
                            zone: &self.zone,
                            audio: frame.audio,
                        },
                    )
                })
                .inner
        });
        // The cycle runs listing, search field, Preview, and round again.
        match leave {
            Leave::Stay => {}
            Leave::Previous => self.focus_search = true,
            Leave::Back | Leave::Next => ui.memory_mut(|m| m.request_focus(self.id)),
        }

        // The listing's focus target sits under the rows, so they still take the pointer.
        let area = ui.available_rect_before_wrap();
        ui.interact(area, self.id, Sense::focusable_noninteractive());
        if std::mem::take(&mut self.focus_listing) {
            ui.memory_mut(|m| m.request_focus(self.id));
        }
        let focused = ui.memory(|m| m.has_focus(self.id));
        if focused {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    self.id,
                    EventFilter {
                        tab: true,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        ..EventFilter::default()
                    },
                )
            });
        }
        let page = ((area.height() / t.row_height()).floor() as usize)
            .saturating_sub(2)
            .max(1);
        let nothing_focused = ui.memory(|m| m.focused().is_none());
        let in_search = ui.memory(|m| m.has_focus(self.search_id()));
        if frame.keys && self.search.is_some() && (focused || nothing_focused || in_search) {
            // Escape stops a running Search, and clears one that has ended.
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                if self.is_searching() {
                    self.stop_search();
                } else {
                    self.clear_search();
                }
            }
        }
        if frame.keys && (focused || nothing_focused) {
            let (next, back) = ui.input_mut(|i| {
                let next = frame.keymap.take(i, Action::CycleFocus);
                (next, frame.keymap.take(i, Action::CycleFocusBack))
            });
            self.focus_search |= next;
            self.focus_preview |= back;
            self.keys(ui, frame.keymap, page);
        }
        if self.search.is_some() {
            let reveal = std::mem::take(&mut self.reveal);
            let show_icons = frame.settings.appearance.show_icons;
            self.matches_view(ui, focused, reveal, show_icons);
            if std::mem::take(&mut self.focus_preview) {
                ui.memory_mut(|m| m.request_focus(self.preview.id));
            }
            return;
        }

        if self.view == View::Folder {
            if let Some(notice) = &self.notice {
                ui.add(widgets::warning_message(notice));
            }
            if let Some(error) = &self.error {
                ui.add(widgets::invalid_message(error));
            }
        }
        let show_icons = frame.settings.appearance.show_icons;
        let mut command = None;
        let mut sort = None;
        let reveal = std::mem::take(&mut self.reveal) | self.tree.take_reveal();
        // The Selection is live while the listing holds the keyboard and the window is in
        // front.
        let live = focused && ui.input(|i| i.focused);
        if self.lost.is_some() {
            // Kept listings stay browsable while Disconnected, greyed.
            ui.multiply_opacity(t.opacity.disabled);
        }
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                widgets::table(self.id.with("table"))
                    .columns([
                        widgets::Column::fill(),
                        widgets::Column::auto().number(),
                        widgets::Column::auto(),
                        widgets::Column::auto(),
                    ])
                    .show(ui, |table| {
                        table.header(|h| {
                            for column in Column::ALL {
                                let sorted =
                                    (self.sort.column == column).then_some(widgets::Sorted {
                                        ascending: self.sort.ascending,
                                        rank: None,
                                    });
                                if h.sort(column.title(), sorted).clicked() {
                                    sort = Some(self.sort.by(column));
                                }
                            }
                        });
                        let (len, focus) = match self.view {
                            View::Tree => (self.tree.len(), self.tree.focus()),
                            View::Folder => (self.folder.len(), Some(self.folder.focus())),
                        };
                        let shown = reveal.then_some(focus).flatten();
                        table.rows(len, shown, |table, i| {
                            let selected = match self.view {
                                View::Tree => self.tree.is_selected(i),
                                View::Folder => self.folder.is_selected(i),
                            };
                            let selection = match (selected, live) {
                                (false, _) => Selection::None,
                                (true, true) => Selection::Live,
                                (true, false) => Selection::Unfocused,
                            };
                            let mut look = RowLook::selected(selection);
                            if focused && Some(i) == focus {
                                look = look.forced(State::Focus);
                            }
                            let shown = self.shown(i);
                            let response = table.row_with(look, |row| {
                                self.cells(row, &shown, show_icons);
                            });
                            let (label, expanded) = (shown.label, shown.expanded);
                            response.ctx.accesskit_node_builder(response.id, |node| {
                                node.set_label(label);
                                if let Some(expanded) = expanded {
                                    node.set_expanded(expanded);
                                }
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
        if let Some(sort) = sort {
            self.sort = sort;
            self.folder.arrange(sort, self.show_hidden);
            self.tree.arrange(sort, self.show_hidden);
        }
        if let Some(command) = command {
            ui.memory_mut(|m| m.request_focus(self.id));
            let now = ui.input(|i| i.time);
            self.apply(command, now);
        }
        if std::mem::take(&mut self.focus_preview) {
            ui.memory_mut(|m| m.request_focus(self.preview.id));
        }
    }

    /// The Focused entry, as Preview shows it.
    fn focused(&self) -> Option<Focused> {
        if let Some(searching) = &self.search {
            let matches = &searching.matches;
            let m = matches.get(matches.focus()?);
            let name = matches::name(&m.path).to_owned();
            return Some(Focused {
                path: matches.absolute(m),
                name: name.clone(),
                entry: Some(Entry {
                    name,
                    is_dir: m.is_dir,
                    ..Entry::default()
                }),
            });
        }
        if self.view == View::Tree {
            let row = self.tree.row(self.tree.focus()?);
            return match row.line {
                tree::Line::Entry {
                    name, path, entry, ..
                } => Some(Focused {
                    path: path.to_owned(),
                    name: name.to_owned(),
                    entry: entry.cloned(),
                }),
                _ => None,
            };
        }
        if self.folder.is_empty() {
            return None;
        }
        let i = self.folder.focus();
        let path = self.folder.path();
        Some(match self.folder.line(i) {
            folder::Line::Here => Focused {
                path: path.to_owned(),
                name: ".".into(),
                entry: None,
            },
            folder::Line::Up => Focused {
                path: folder::parent(path).map_or_else(|| "/".to_owned(), |(up, _)| up),
                name: "..".into(),
                entry: None,
            },
            folder::Line::Entry(_) => {
                let entry = self.folder.entry(i)?.clone();
                Focused {
                    path: folder::join(path, &entry.name),
                    name: entry.name.clone(),
                    entry: Some(entry),
                }
            }
        })
    }

    /// What row `i` of the view shows.
    fn shown(&self, i: usize) -> Shown<'_> {
        match self.view {
            View::Folder => Shown {
                label: self.folder.name(i).to_owned(),
                entry: self.folder.entry(i),
                depth: None,
                expanded: None,
                status: false,
            },
            View::Tree => {
                let row = self.tree.row(i);
                let (label, entry, expanded, status) = match row.line {
                    Line::Entry {
                        name,
                        entry,
                        expanded,
                        ..
                    } => (name.to_owned(), entry, expanded, false),
                    Line::Loading => ("Loading…".to_owned(), None, None, true),
                    Line::Failed(reason) => (format!("Cannot read: {reason}"), None, None, true),
                };
                Shown {
                    label,
                    entry,
                    depth: Some(row.depth),
                    expanded,
                    status,
                }
            }
        }
    }

    fn cells(&self, row: &mut widgets::TableRow<'_>, shown: &Shown<'_>, show_icons: bool) {
        let entry = shown.entry;
        let kind = entry.map_or(Kind::Directory, |e| Kind::of(&e.name, e.is_dir));
        let target = entry.and_then(|e| e.link.as_deref());
        row.ui(|ui| {
            if let Some(depth) = shown.depth {
                ui.add_space(widgets::TREE_INDENT * depth as f32);
                match shown.expanded {
                    Some(open) => {
                        let glyph = if open {
                            Glyph::ChevronDown
                        } else {
                            Glyph::ChevronRight
                        };
                        let ink = ui.tokens().color.text.muted;
                        ui.add(widgets::glyph(glyph).color(ink));
                    }
                    None => ui.add_space(widgets::TREE_INDENT),
                }
            }
            if shown.status {
                let t = ui.tokens();
                ui.label(t.ty.body.rich(&shown.label).color(t.color.text.muted));
            } else {
                ui.add(
                    widgets::entry_name(&shown.label, kind)
                        .icon(show_icons)
                        .link(target.is_some())
                        .target(target),
                );
            }
        });
        // A directory's size is blank, and so is anything the host did not say.
        let size = entry.and_then(|e| e.size.filter(|_| !e.is_dir));
        row.text(size.map(size_text).unwrap_or_default());
        let modified = entry.and_then(|e| e.modified);
        row.muted(
            modified
                .map(|at| modified_text(at, &self.zone))
                .unwrap_or_default(),
        );
        let mode = entry.and_then(|e| e.permissions);
        row.muted(mode.map(permissions_text).unwrap_or_default());
    }

    /// The listing's keys, and typed characters for the name prefix.
    fn keys(&mut self, ui: &mut egui::Ui, keymap: &KeyMap, page: usize) {
        const KEYS: [(Action, Command); 17] = [
            (Action::MoveUp, Command::Up),
            (Action::MoveDown, Command::Down),
            (Action::MoveFirst, Command::Home),
            (Action::MoveLast, Command::End),
            (Action::PageUp, Command::PageUp(0)),
            (Action::PageDown, Command::PageDown(0)),
            (Action::ExtendUp, Command::ExtendUp),
            (Action::ExtendDown, Command::ExtendDown),
            (Action::FocusUp, Command::FocusUp),
            (Action::FocusDown, Command::FocusDown),
            (Action::ToggleSelected, Command::Toggle),
            (Action::SelectAll, Command::SelectAll),
            (Action::Open, Command::Open),
            (Action::Parent, Command::Parent),
            (Action::ParentFolder, Command::Parent),
            (Action::Expand, Command::Expand),
            (Action::Collapse, Command::Collapse),
        ];
        let view = self.view;
        let (commands, sort, now) = ui.input_mut(|input| {
            let mut commands = Vec::new();
            for (action, command) in KEYS {
                // Alt+Up is Folder view's alone, and Right and Left are Tree view's.
                let skip = match view {
                    View::Tree => action == Action::ParentFolder,
                    View::Folder => matches!(action, Action::Expand | Action::Collapse),
                };
                if !skip && keymap.take(input, action) {
                    commands.push(match command {
                        Command::PageUp(_) => Command::PageUp(page),
                        Command::PageDown(_) => Command::PageDown(page),
                        other => other,
                    });
                }
            }
            let sort = keymap.take(input, Action::Sort);
            input.events.retain(|event| match event {
                Event::Text(text) if !text.chars().any(char::is_control) => {
                    commands.extend(text.chars().map(Command::Type));
                    false
                }
                _ => true,
            });
            (commands, sort, input.time)
        });
        if sort {
            self.sort = self.sort.next();
            self.folder.arrange(self.sort, self.show_hidden);
            self.tree.arrange(self.sort, self.show_hidden);
        }
        for command in commands {
            self.reveal = true;
            self.apply(command, now);
        }
    }

    fn apply(&mut self, command: Command, now: f64) {
        if let Some(searching) = &mut self.search {
            if let matches::Effect::Open(m) = searching.matches.apply(command) {
                let path = searching.matches.absolute(&m);
                self.open_match(path, m.is_dir);
            }
            return;
        }
        match self.view {
            View::Folder => {
                let effect = self.folder.apply(command, now);
                self.carry_out(effect);
            }
            View::Tree => match self.tree.apply(command, now) {
                tree::Effect::None => {}
                tree::Effect::Fetch(path) => self.fetch(vec![path]),
                tree::Effect::Preview => self.focus_preview = true,
            },
        }
    }

    fn carry_out(&mut self, effect: Effect) {
        match effect {
            Effect::None => {}
            Effect::List { path, focus } => {
                self.listing = Some((self.connection.list(path), focus));
            }
            Effect::Preview => self.focus_preview = true,
        }
    }

    /// Starts fetching `paths` for Tree view.
    fn fetch(&mut self, paths: Vec<String>) {
        for path in paths {
            let request = self.connection.list(path.clone());
            self.fetches.push((path, request));
        }
    }

    /// Lists `path` in Folder view, from the kept listing when there is one, and focuses
    /// `focus` in it.
    fn list(&mut self, path: String, focus: Option<String>) {
        match self.tree.listing(&path) {
            Some(listing) => {
                let listing = listing.clone();
                self.show_folder(listing, focus);
            }
            None => self.listing = Some((self.connection.list(path), focus)),
        }
    }

    fn show_folder(&mut self, listing: Listing, focus: Option<String>) {
        self.folder = Folder::new(listing, self.sort, self.show_hidden);
        if let Some(name) = focus {
            self.folder.focus_name(&name);
        }
        self.listing = None;
        self.error = None;
        self.notice = None;
        self.reveal = true;
    }

    /// Starts an Auto-refresh when one is due. This only runs while the Session is
    /// shown; a tick is held back while the window is minimized or a Refresh runs.
    // A tick is also held back while Matches show.
    fn auto_refresh(&mut self, ui: &egui::Ui, settings: &AppSettings) {
        let now = ui.input(|i| i.time);
        if std::mem::take(&mut self.refreshed) {
            self.timer.restart(now);
        }
        let minimized = ui.input(|i| i.viewport().minimized == Some(true));
        let ready = !minimized && !self.is_refreshing() && !self.shows_matches();
        let every = settings.browsing.auto_refresh;
        if self.timer.tick(now, every, ready) {
            self.refresh();
        } else if ready && let Some(wait) = self.timer.wait(now, every) {
            ui.ctx()
                .request_repaint_after(Duration::from_secs_f64(wait.max(0.01)));
        }
    }

    /// Takes the answers of a running Refresh and merges them in.
    fn receive_refresh(&mut self) {
        let running = self.is_refreshing();
        let mut arrived = Vec::new();
        self.refreshes
            .retain(|(path, request)| match request.try_take() {
                Some(answer) => {
                    arrived.push((path.clone(), answer));
                    false
                }
                None => true,
            });
        // Preview reloads a Focused file whose size or modified time changed on its own:
        // it compares the merged entry with what it shows on the next frame.
        for (path, answer) in arrived {
            // A failure the drop caused leaves the kept listing as it was.
            if !(answer.is_err() && self.dropped()) {
                self.tree.refreshed(&path, answer);
            }
        }

        if let Some(answer) = self
            .folder_refresh
            .as_ref()
            .and_then(|r| r.request.try_take())
            && let Some(FolderRefresh { path, gone, .. }) = self.folder_refresh.take()
            && !(answer.is_err() && self.dropped())
        {
            self.folder_refreshed(path, gone, answer);
        }
        if running && !self.is_refreshing() {
            self.refreshed = true;
        }
    }

    /// Folder view's Refresh answer for `path`. `gone` is the directory Folder view
    /// listed, when it no longer exists and `path` is an ancestor tried instead.
    fn folder_refreshed(
        &mut self,
        path: String,
        gone: Option<String>,
        answer: Result<Listing, String>,
    ) {
        let missing = matches!(&answer, Err(reason) if reason == NO_SUCH_FILE);
        self.tree.refreshed(&path, answer.clone());
        match answer {
            Ok(listing) => match gone {
                None => {
                    self.error = None;
                    self.folder.refreshed(listing);
                }
                Some(gone) => {
                    self.show_folder(listing, None);
                    self.notice = Some(format!("{gone} no longer exists."));
                }
            },
            Err(_) if missing && folder::parent(&path).is_some() => {
                if let Some((up, _)) = folder::parent(&path) {
                    self.folder_refresh = Some(FolderRefresh {
                        request: self.connection.list(up.clone()),
                        path: up,
                        gone: Some(gone.unwrap_or(path)),
                    });
                }
            }
            Err(reason) => {
                let empty = Listing {
                    path: path.clone(),
                    entries: Vec::new(),
                };
                self.show_folder(empty, None);
                self.error = Some(format!("Cannot read: {reason}"));
                if let Some(gone) = gone {
                    self.notice = Some(format!("{gone} no longer exists."));
                }
            }
        }
    }

    /// Takes the listings that have arrived. Every one is kept in the tree.
    fn receive(&mut self) {
        self.receive_refresh();
        let mut arrived = Vec::new();
        self.fetches
            .retain(|(path, request)| match request.try_take() {
                Some(answer) => {
                    arrived.push((path.clone(), answer));
                    false
                }
                None => true,
            });
        for (path, answer) in arrived {
            if answer.is_err() && self.dropped() {
                // Fetched again once the connection is back.
                self.refetch.push(path);
            } else {
                self.tree.listed(&path, answer);
            }
        }

        let Some((request, _)) = &self.listing else {
            return;
        };
        let Some(answer) = request.try_take() else {
            return;
        };
        let Some((_, focus)) = self.listing.take() else {
            return;
        };
        match answer {
            Ok(listing) => {
                self.tree.listed(&listing.path.clone(), Ok(listing.clone()));
                self.show_folder(listing, focus);
            }
            // The drop stopped it: Folder view stays where it was.
            Err(_) if self.dropped() => {}
            Err(reason) => {
                self.error = Some(format!("Cannot read: {reason}"));
            }
        }
    }

    /// Whether the connection has dropped, noticed yet or not: a request failing then
    /// failed because of the drop.
    fn dropped(&self) -> bool {
        self.lost.is_some() || self.connection.lost().is_some()
    }
}

/// Folder view's Refresh: the directory being listed again, and the one that went when
/// that is an ancestor tried instead.
struct FolderRefresh {
    request: Request<Listing>,
    path: String,
    gone: Option<String>,
}

/// What the Matches show when the host cannot run a Search.
pub const UNAVAILABLE: &str = "Search is not available on this host.";

/// What the Matches show when the connection dropped during the Search.
pub const SEARCH_LOST: &str = "Search stopped: connection lost";

/// The Preview panel's width when a Session opens.
const PREVIEW_WIDTH: f32 = 360.0;
/// What one row shows, in either view.
struct Shown<'a> {
    /// The entry's name, or a status row's text: the row's AccessKit label.
    label: String,
    entry: Option<&'a Entry>,
    /// Tree view's depth, for the indent.
    depth: Option<usize>,
    /// Whether a Tree view directory is expanded.
    expanded: Option<bool>,
    /// A "Loading…" or failure row.
    status: bool,
}

/// An unsaved Session's tab label: `account@address`, and `:port` off port 22.
pub fn label(target: &Target) -> String {
    let mut label = format!("{}@{}", target.account, target.address);
    if target.port != 22 {
        label.push_str(&format!(":{}", target.port));
    }
    label
}
