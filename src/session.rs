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
//! Preview, on the right, shows the Focused entry of either view; Enter on a file and
//! Tab move the keyboard there.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use egui::{Event, EventFilter, Id, Sense};

use crate::folder::{
    self, Column, Command, Effect, Folder, Sort, modified_text, permissions_text, size_text,
};
use crate::keymap::{Action, KeyMap};
use crate::kind::Kind;
use crate::preview::Focused;
use crate::preview_panel::{Leave, PanelFrame, PreviewPanel};
use crate::settings::AppSettings;
use crate::ssh::{self, Entry, Listing, Request, Target};
use crate::theme::TokensExt as _;
use crate::tree::{self, Line, Tree};
use crate::widgets::{self, Glyph, RowLook, Selection, State};

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
    connection: ssh::Session,
    /// The operator's time zone, for the Modified column.
    zone: jiff::tz::TimeZone,
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
            id,
            preview: PreviewPanel::new(id.with("preview")),
            focus_preview: false,
            focus_listing: true,
            reveal: false,
            connection: opened.session,
            zone: jiff::tz::TimeZone::system(),
        }
    }

    pub fn label(&self) -> &str {
        &self.details.label
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

    /// The Session's listing in its view.
    pub fn ui(&mut self, ui: &mut egui::Ui, frame: Frame<'_>) {
        self.receive();
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
                            focused: focused_entry,
                            keymap: frame.keymap,
                            settings: frame.settings,
                            keys: frame.keys,
                            zone: &self.zone,
                        },
                    )
                })
                .inner
        });
        // TODO(#31): Tab and Shift+Tab pass through the search field too.
        if leave != Leave::Stay {
            ui.memory_mut(|m| m.request_focus(self.id));
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
        if frame.keys && (focused || nothing_focused) {
            // TODO(#31): the search field comes between the listing and Preview.
            let cycle = ui.input_mut(|i| {
                let next = frame.keymap.take(i, Action::CycleFocus);
                next | frame.keymap.take(i, Action::CycleFocusBack)
            });
            self.focus_preview |= cycle;
            self.keys(ui, frame.keymap, page);
        }

        if let (View::Folder, Some(error)) = (self.view, &self.error) {
            ui.add(widgets::invalid_message(error));
        }
        let show_icons = frame.settings.appearance.show_icons;
        let mut command = None;
        let mut sort = None;
        let reveal = std::mem::take(&mut self.reveal) | self.tree.take_reveal();
        // The Selection is live while the listing holds the keyboard and the window is in
        // front.
        let live = focused && ui.input(|i| i.focused);
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
                        for i in 0..len {
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
                        }
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
        self.reveal = true;
    }

    /// Takes the listings that have arrived. Every one is kept in the tree.
    fn receive(&mut self) {
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
            self.tree.listed(&path, answer);
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
            Err(reason) => {
                self.error = Some(format!("Cannot read: {reason}"));
            }
        }
    }
}

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
