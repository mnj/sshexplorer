//! An open Session: its connection, its tab label, and its Folder view.
//!
//! Folder view is a table of one directory: Name, Size, Modified and Permissions, with
//! the Kind icons, sorted by clicking a header. What the keys and the pointer do to it is
//! [`Folder`]'s; this module turns them into [`Command`]s, asks the host for another
//! directory when a command leaves this one, and draws the result.
// TODO(#28): Tree view and switching views.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use egui::{Event, EventFilter, Id, Sense};

use crate::folder::{
    Column, Command, Effect, Folder, Sort, modified_text, permissions_text, size_text,
};
use crate::keymap::{Action, KeyMap};
use crate::kind::Kind;
use crate::settings::AppSettings;
use crate::ssh::{self, Listing, Request, Target};
use crate::theme::TokensExt as _;
use crate::widgets::{self, RowLook, Selection, State};

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
}

pub struct Session {
    pub details: Details,
    folder: Folder,
    sort: Sort,
    /// The "Show hidden entries" setting the folder was last arranged with.
    show_hidden: bool,
    /// A directory being listed, and the entry to focus in it.
    listing: Option<(Request<Listing>, Option<String>)>,
    /// Why the last directory asked for could not be listed.
    error: Option<String>,
    /// The listing's own focus target: it holds the keyboard while it has egui's focus.
    id: Id,
    /// The listing takes the keyboard on the next frame: a new Session focuses it.
    focus_listing: bool,
    /// The Focused entry moved by key; scroll it into view.
    reveal: bool,
    connection: ssh::Session,
    /// The operator's time zone, for the Modified column.
    zone: jiff::tz::TimeZone,
}

impl Session {
    pub fn new(details: Details, opened: ssh::Opened) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let sort = Sort::default();
        Session {
            details,
            folder: Folder::new(opened.home, sort, false),
            sort,
            show_hidden: false,
            listing: None,
            error: None,
            id: Id::NULL.with("sshexplorer-listing").with(n),
            focus_listing: true,
            reveal: false,
            connection: opened.session,
            zone: jiff::tz::TimeZone::system(),
        }
    }

    pub fn label(&self) -> &str {
        &self.details.label
    }

    /// The Folder view of the listed directory.
    pub fn ui(&mut self, ui: &mut egui::Ui, frame: Frame<'_>) {
        self.receive();
        let show_hidden = frame.settings.browsing.show_hidden;
        if show_hidden != self.show_hidden {
            self.show_hidden = show_hidden;
            self.folder.arrange(self.sort, show_hidden);
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
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        ..EventFilter::default()
                    },
                )
            });
        }
        let t = ui.tokens();
        let page = ((area.height() / t.row_height()).floor() as usize)
            .saturating_sub(2)
            .max(1);
        let nothing_focused = ui.memory(|m| m.focused().is_none());
        if frame.keys && (focused || nothing_focused) {
            self.keys(ui, frame.keymap, page);
        }

        if let Some(error) = &self.error {
            ui.add(widgets::invalid_message(error));
        }
        let show_icons = frame.settings.appearance.show_icons;
        let mut command = None;
        let mut sort = None;
        let reveal = std::mem::take(&mut self.reveal);
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
                        for i in 0..self.folder.len() {
                            let selection = match (self.folder.is_selected(i), live) {
                                (false, _) => Selection::None,
                                (true, true) => Selection::Live,
                                (true, false) => Selection::Unfocused,
                            };
                            let mut look = RowLook::selected(selection);
                            if focused && i == self.folder.focus() {
                                look = look.forced(State::Focus);
                            }
                            let response = table.row_with(look, |row| {
                                self.cells(row, i, show_icons);
                            });
                            let name = self.folder.name(i).to_owned();
                            response
                                .ctx
                                .accesskit_node_builder(response.id, |node| node.set_label(name));
                            if reveal && i == self.folder.focus() {
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
        }
        if let Some(command) = command {
            ui.memory_mut(|m| m.request_focus(self.id));
            let now = ui.input(|i| i.time);
            let effect = self.folder.apply(command, now);
            self.carry_out(effect);
        }
    }

    fn cells(&self, row: &mut widgets::TableRow<'_>, i: usize, show_icons: bool) {
        let entry = self.folder.entry(i);
        let name = self.folder.name(i);
        let kind = entry.map_or(Kind::Directory, |e| Kind::of(&e.name, e.is_dir));
        let target = entry.and_then(|e| e.link.as_deref());
        row.ui(|ui| {
            ui.add(
                widgets::entry_name(name, kind)
                    .icon(show_icons)
                    .link(target.is_some())
                    .target(target),
            );
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
        const KEYS: [(Action, Command); 15] = [
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
        ];
        let (commands, sort, now) = ui.input_mut(|input| {
            let mut commands = Vec::new();
            for (action, command) in KEYS {
                if keymap.take(input, action) {
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
        }
        for command in commands {
            self.reveal = true;
            let effect = self.folder.apply(command, now);
            self.carry_out(effect);
        }
    }

    fn carry_out(&mut self, effect: Effect) {
        match effect {
            Effect::None => {}
            Effect::List { path, focus } => {
                self.listing = Some((self.connection.list(path), focus));
            }
            // TODO(#33): Preview takes the keyboard.
            Effect::Preview => {}
        }
    }

    /// Takes a listing that has arrived.
    fn receive(&mut self) {
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
                self.folder = Folder::new(listing, self.sort, self.show_hidden);
                if let Some(name) = focus {
                    self.folder.focus_name(&name);
                }
                self.error = None;
                self.reveal = true;
            }
            Err(reason) => {
                self.error = Some(format!("Cannot read: {reason}"));
            }
        }
    }
}

/// An unsaved Session's tab label: `account@address`, and `:port` off port 22.
pub fn label(target: &Target) -> String {
    let mut label = format!("{}@{}", target.account, target.address);
    if target.port != 22 {
        label.push_str(&format!(":{}", target.port));
    }
    label
}
