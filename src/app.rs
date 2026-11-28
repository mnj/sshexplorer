//! The main window: the toolbar along the top and, once one is open, the Sessions as tabs;
//! and the Settings dialog over it.
//!
//! Launch opens no Session: the window shows the welcome column, which lists the saved
//! Hosts (a click opens a Session of one) and Add a Host. Hosts in the status bar, or
//! Ctrl+O, opens the Host dialog; saved Hosts are kept in `hosts.toml` beside the
//! settings. The tabs run along the Tab bar edge, and Ctrl+Tab and Ctrl+Shift+Tab cycle
//! them. Under its tab, the active Session has its own bar: Back, Forward and Refresh
//! (also F5), the place as breadcrumbs, the Location menu, Search and the view switch.
//! The status bar shows the Session's connection, Downloads, the Readonly indicator and
//! the Local folder. The Settings gear, and Ctrl+, from anywhere, opens the Settings
//! dialog. Every change there applies on the next frame and is saved at once. The SSH
//! stack starts with the first Session opened, and every network step runs on it, off
//! this frame thread.

use std::path::PathBuf;
use std::sync::{Arc, mpsc};

use egui::{Id, LayerId, Order, ViewportCommand};

use crate::binding_editor::BindingEditor;
use crate::connect::{HostAction, HostDialog, OpenRequest, Outcome, Pending};
use crate::credentials;
use crate::hosts::{Host, HostStore};
use crate::keymap::{Action, KeyMap};
use crate::openssh_config::{Config, IdentityAgent};
use crate::session::{self, Session, View};
use crate::settings::{
    Animate, AppSettings, Appearance, AutoRefresh, Browsing, Connections, DEFAULT_UI_SCALE, Edge,
    PreviewSettings, ReconnectAttempts, ScratchChoice, Search, Store, ThemeChoice, UI_SCALES,
};
use crate::ssh::{self, Failure, Opening, Options, Ssh};
use crate::theme::{self, Accent, Density, TokensExt as _, fonts};
use crate::widgets::{self, Glyph, TabKind, Tooltip as _};

/// The Settings dialog's pages, in order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Appearance,
    Browsing,
    Search,
    Preview,
    Download,
    Connections,
    Keys,
}

impl Page {
    pub const ALL: [Page; 7] = [
        Page::Appearance,
        Page::Browsing,
        Page::Search,
        Page::Preview,
        Page::Download,
        Page::Connections,
        Page::Keys,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Page::Appearance => "Appearance",
            Page::Browsing => "Browsing",
            Page::Search => "Search",
            Page::Preview => "Preview",
            Page::Download => "Download",
            Page::Connections => "Connections",
            Page::Keys => "Keys",
        }
    }

    /// The footer's reset button on this page.
    fn reset_label(self) -> String {
        match self {
            Page::Keys => "Reset all keys".to_owned(),
            page => format!("Reset {} to defaults", page.name()),
        }
    }
}

const PAGE_NAMES: [&str; 7] = [
    "Appearance",
    "Browsing",
    "Search",
    "Preview",
    "Download",
    "Connections",
    "Keys",
];

/// The Settings dialog while it is open.
struct SettingsDialog {
    page: usize,
    /// The installed families the font fields list, read when the dialog opened.
    ui_fonts: Vec<String>,
    mono_fonts: Vec<String>,
    keys: BindingEditor,
}

impl SettingsDialog {
    fn open(ctx: &egui::Context) -> SettingsDialog {
        let families = fonts::source(ctx).families();
        let names = |keep: fn(&fonts::Family) -> bool| {
            families
                .iter()
                .filter(|f| keep(f))
                .map(|f| f.name.clone())
                .collect()
        };
        SettingsDialog {
            page: 0,
            ui_fonts: names(fonts::Family::has_latin_regular),
            mono_fonts: names(fonts::Family::is_fixed_pitch),
            keys: BindingEditor::default(),
        }
    }
}

/// The app's state across frames.
pub struct App {
    settings: AppSettings,
    /// Where settings are saved; `None` keeps them for this run only.
    store: Option<Store>,
    dialog: Option<SettingsDialog>,
    /// The Appearance last put on the context, so a change is applied once.
    applied: Option<Appearance>,
    /// The `known_hosts` file host keys are checked against, and Trust appends to.
    /// Empty (none) unless set: only [`App::operator_ssh`] names the operator's own.
    known_hosts: PathBuf,
    /// The OpenSSH config, read at each Session open while it is in use; none unless
    /// set, so that only the app itself reads the operator's `~/.ssh/config`.
    ssh_config: Option<PathBuf>,
    /// The SSH agent when the config does not name one; none unless set. The app uses
    /// `SSH_AUTH_SOCK`, or the OpenSSH agent's pipe on Windows.
    agent: Option<PathBuf>,
    /// The saved Hosts, in the Host dialog's order.
    hosts: Vec<Host>,
    /// Where Hosts are saved; `None` keeps them for this run only.
    host_store: Option<HostStore>,
    host_dialog: Option<HostDialog>,
    host_baseline: Vec<Host>,
    host_conflict: bool,
    host_save_error: Option<String>,
    settings_save_error: Option<String>,
    /// Where remembered secrets are kept: the OS credential store in the app, a store in
    /// memory unless [`App::credentials_at`] says otherwise.
    credentials: Arc<dyn credentials::Store>,
    /// Where Preview sends audio: the system's output in the app, nowhere unless
    /// [`App::audio_out`] says otherwise, so tests need no sound device.
    audio: crate::media::AudioOut,
    /// Removals from the credential store still running, each answering once.
    forgetting: Vec<mpsc::Receiver<Result<(), credentials::Refused>>>,
    pending: Vec<Pending>,
    /// Dropped before `ssh`, so that each closes its connection on a live runtime.
    sessions: Vec<Session>,
    active: usize,
    ssh: Option<Ssh>,
    next_id: u64,
    /// "A Download is running. Close anyway?", while it asks.
    confirm: Option<Closing>,
    /// The operator confirmed quitting during a Download.
    quitting: bool,
}

/// What waits on "A Download is running. Close anyway?".
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Closing {
    Session(usize),
    Quit,
}

/// An app that touches nothing of the operator's: no `known_hosts`, OpenSSH config or
/// SSH agent (see [`App::operator_ssh`]), secrets in memory and no sound.
impl Default for App {
    fn default() -> Self {
        App::with_known_hosts(PathBuf::new())
    }
}

/// The operator's home directory, or none.
fn home() -> PathBuf {
    std::env::home_dir().unwrap_or_default()
}

/// `~/.ssh/known_hosts` (`%USERPROFILE%\.ssh\known_hosts` on Windows).
fn default_known_hosts() -> PathBuf {
    home().join(".ssh").join("known_hosts")
}

/// `~/.ssh/config` (`%USERPROFILE%\.ssh\config` on Windows).
fn default_ssh_config() -> PathBuf {
    home().join(".ssh").join("config")
}

/// Readies `ctx` for the app, before its first frame: the bundled fonts, the tokens and
/// both themes' styles, following the system's light or dark. egui's own zoom keys are
/// turned off: Ctrl+= and Ctrl+- belong to Preview, and UI scale has its own keys.
pub fn setup(ctx: &egui::Context) {
    theme::install(ctx, egui::ThemePreference::System);
    ctx.options_mut(|o| o.zoom_with_keyboard = false);
}

impl App {
    /// An app that checks host keys against the `known_hosts` file at `path`.
    pub fn with_known_hosts(path: PathBuf) -> Self {
        App {
            settings: AppSettings::default(),
            store: None,
            dialog: None,
            applied: None,
            known_hosts: path,
            ssh_config: None,
            agent: None,
            hosts: Vec::new(),
            host_store: None,
            host_dialog: None,
            host_baseline: Vec::new(),
            host_conflict: false,
            host_save_error: None,
            settings_save_error: None,
            credentials: Arc::new(credentials::MemoryStore::default()),
            audio: crate::media::AudioOut::Silent,
            forgetting: Vec::new(),
            pending: Vec::new(),
            sessions: Vec::new(),
            active: 0,
            ssh: None,
            next_id: 0,
            confirm: None,
            quitting: false,
        }
    }

    /// The app with the settings `store` holds, and the Hosts saved beside them, saving
    /// every change back.
    pub fn with_store(store: Store) -> App {
        let host_store = HostStore::beside(store.path());
        let loaded = host_store.load_identified();
        let host_save_error = loaded
            .as_ref()
            .err()
            .map(|e| format!("Cannot load Hosts: {e}"));
        let hosts = loaded.unwrap_or_else(|_| host_store.load());
        App {
            settings: store.load(),
            host_baseline: hosts.clone(),
            hosts,
            host_save_error,
            host_store: Some(host_store),
            store: Some(store),
            ..App::default()
        }
    }

    /// The same app, keeping remembered secrets in `store`: the OS credential store in
    /// the app, a [`credentials::MemoryStore`] in tests.
    pub fn credentials_at(mut self, store: Arc<dyn credentials::Store>) -> App {
        self.credentials = store;
        self
    }

    /// The same app, sending Preview's audio to `out`: the system's output in the app,
    /// [`crate::media::AudioOut::Silent`] in tests.
    pub fn audio_out(mut self, out: crate::media::AudioOut) -> App {
        self.audio = out;
        self
    }

    /// The same app, checking host keys against the `known_hosts` file at `path`.
    pub fn known_hosts_at(mut self, path: PathBuf) -> App {
        self.known_hosts = path;
        self
    }

    /// The same app, reaching the SSH agent at `socket` unless the config names another;
    /// `None` for no agent. Tests use it to keep away from the operator's agent.
    pub fn agent_at(mut self, socket: Option<PathBuf>) -> App {
        self.agent = socket;
        self
    }

    /// The same app, reading the OpenSSH config at `path`.
    pub fn ssh_config_at(mut self, path: PathBuf) -> App {
        self.ssh_config = Some(path);
        self
    }

    /// The same app, using the operator's own `~/.ssh/known_hosts`, `~/.ssh/config` and
    /// SSH agent. Only main.rs calls it, so that no test touches them.
    pub fn operator_ssh(self) -> App {
        self.known_hosts_at(default_known_hosts())
            .ssh_config_at(default_ssh_config())
            .agent_at(ssh::default_agent())
    }

    /// The OpenSSH config as the file holds it now; empty when it is not in use.
    fn read_ssh_config(&self) -> Config {
        match &self.ssh_config {
            Some(path) if self.settings.connections.use_ssh_config => Config::read(path, &home()),
            _ => Config::default(),
        }
    }

    /// The app with `settings`, kept for this run only.
    pub fn with_settings(settings: AppSettings) -> App {
        App {
            settings,
            ..App::default()
        }
    }

    pub fn settings(&self) -> &AppSettings {
        &self.settings
    }

    pub fn keymap(&self) -> &KeyMap {
        &self.settings.keys
    }

    /// Puts the settings on `ctx` if they are not there yet. The first frame does this
    /// itself; call it before then so the first frame is already drawn with them.
    pub fn apply(&mut self, ctx: &egui::Context) {
        let a = &self.settings.appearance;
        if self.applied.as_ref() == Some(a) {
            return;
        }
        theme::set(ctx, &a.theme_settings());
        ctx.set_theme(a.theme_preference());
        ctx.set_zoom_factor(a.zoom_factor());
        self.applied = Some(a.clone());
    }

    fn persist_hosts(&mut self, ctx: &egui::Context, overwrite: bool) {
        let replaced = match &self.host_store {
            Some(store) => match store.merge(&self.host_baseline, &self.hosts, overwrite) {
                Ok(replaced) => replaced,
                Err(e) => {
                    self.host_conflict = e.kind() == std::io::ErrorKind::AlreadyExists;
                    self.host_save_error = Some(format!(
                        "Cannot save Hosts to {}: {e}",
                        store.path().display()
                    ));
                    return;
                }
            },
            None => self.host_baseline.clone(),
        };
        self.host_baseline.clone_from(&self.hosts);
        self.host_save_error = None;
        self.host_conflict = false;
        let mut keys = Vec::new();
        for old in replaced {
            let new = self.hosts.iter().find(|h| h.id == old.id);
            keys.extend(
                credentials::forgotten(&old, new)
                    .into_iter()
                    .map(|kind| credentials::Key::new(old.id.clone(), kind)),
            );
        }
        if !keys.is_empty() {
            self.forget(ctx, keys);
        }
    }

    fn save(&mut self) {
        self.settings_save_error = self.store.as_ref().and_then(|store| {
            store
                .save(&self.settings)
                .err()
                .map(|e| format!("Cannot save settings to {}: {e}", store.path().display()))
        });
    }

    /// Draws one frame in `ui`, which covers the window.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.apply(&ctx);
        let before = self.settings.clone();
        // Every Session's Downloads go on, on screen or not.
        let now = ui.input(|i| i.time);
        let attempts = self.settings.connections.reconnect_attempts;
        for (i, session) in self.sessions.iter_mut().enumerate() {
            if i != self.active {
                session.hidden();
            }
            session.poll(now);
            // Reconnecting goes on whether the Session is shown or not.
            if let Some(wait) = session.keep_connected(now, attempts) {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait));
            }
        }
        self.quit_requested(&ctx);
        let modal = self.host_dialog.is_some()
            || self.pending.iter().any(Pending::is_modal)
            || self.sessions.iter().any(Session::is_asking)
            || self.confirm.is_some();
        // A binding being recorded takes the keys before any action sees them.
        if let Some(dialog) = &mut self.dialog {
            dialog.keys.record(&ctx, &mut self.settings.keys);
        }
        self.keys(ui, modal);
        let mut open_settings = false;
        ui.add_enabled_ui(!modal, |ui| {
            open_settings = self.window(ui);
        });
        if open_settings && self.dialog.is_none() {
            self.dialog = Some(SettingsDialog::open(&ctx));
        }
        self.dialogs(&ctx);
        self.confirm_close(&ctx);
        self.settings_dialog(&ctx);
        if self.settings != before {
            self.save();
            self.apply(&ctx);
        }
    }

    /// The status bar, tab strip and active Session; whether Settings was clicked.
    fn window(&mut self, ui: &mut egui::Ui) -> bool {
        if self.host_dialog.is_none() && widgets::retry_save(ui, self.host_save_error.as_deref()) {
            self.persist_hosts(ui.ctx(), false);
        }
        if self.dialog.is_none() && widgets::retry_save(ui, self.settings_save_error.as_deref()) {
            self.save();
        }
        let mut hosts = false;
        let mut open_settings = false;
        let t = ui.tokens();
        let side = t.status_bar_height();
        egui::Panel::bottom("app-status")
            .frame(egui::Frame::NONE)
            .exact_size(side)
            .resizable(false)
            .show(ui, |ui| {
                widgets::status_bar(ui, |ui| {
                    // Reserve the controls first so a long endpoint cannot push them off screen.
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        open_settings = ui
                            .add(
                                widgets::glyph_button(Glyph::Gear)
                                    .side(side)
                                    .name("Settings"),
                            )
                            .tooltip("Settings")
                            .clicked();
                        ui.add_space(t.space.gap);
                        let button = ui.add(
                            widgets::glyph_button(Glyph::HostPlus)
                                .side(side)
                                .name("Hosts"),
                        );
                        let tip = "Open or manage hosts";
                        hosts = match self.settings.keys.binding(Action::HostDialog) {
                            Some(key) => {
                                button.tooltip_key(tip, key.label(cfg!(target_os = "macos")))
                            }
                            None => button.tooltip(tip),
                        }
                        .clicked();
                        if let Some(session) = self.sessions.get_mut(self.active) {
                            ui.add_space(t.space.gap);
                            let details = &session.details;
                            let folder = details.local_folder.display().to_string();
                            ui.scope(|ui| {
                                ui.set_max_width(ui.available_width() / 3.0);
                                ui.add(egui::Label::new(folder).truncate())
                                    .tooltip("Local folder: where Downloads are written");
                            });
                            if details.readonly {
                                ui.add_space(t.space.gap);
                                ui.add(widgets::badge("Readonly"))
                                    .tooltip("This Session cannot change the remote filesystem");
                            }
                            ui.add_space(t.space.gap);
                            ui.with_layout(
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.shrink_clip_rect(ui.max_rect());
                                    session.host_status(ui);
                                },
                            );
                        }
                    });
                });
            });
        if hosts {
            self.open_host_dialog();
        }
        self.pending.retain_mut(|opening| !opening.progress(ui));
        if self.sessions.is_empty() {
            if self.pending.is_empty() {
                self.welcome(ui);
            }
            return open_settings;
        }
        let edge = self.settings.appearance.tab_bar_edge;
        match edge {
            Edge::Top => {
                self.tabs(ui, false);
                self.active_session(ui);
            }
            Edge::Bottom => {
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    self.tabs(ui, false);
                    ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                        self.active_session(ui);
                    });
                });
            }
            Edge::Left | Edge::Right => {
                let layout = if edge == Edge::Left {
                    egui::Layout::left_to_right(egui::Align::Min)
                } else {
                    egui::Layout::right_to_left(egui::Align::Min)
                };
                ui.with_layout(layout, |ui| {
                    self.tabs(ui, true);
                    ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                        self.active_session(ui);
                    });
                });
            }
        }
        open_settings
    }

    /// The Sessions' tabs: in a row along the top or bottom, or stacked down a side.
    fn tabs(&mut self, ui: &mut egui::Ui, stacked: bool) {
        let t = ui.tokens();
        let mut close = None;
        let strip = |ui: &mut egui::Ui| {
            widgets::tab_strip(ui, |ui| {
                for (i, session) in self.sessions.iter().enumerate() {
                    let kind = if i == self.active {
                        TabKind::Active
                    } else {
                        TabKind::Back
                    };
                    // A Session off screen whose Download waits for an answer is marked.
                    let attention = i != self.active && session.needs_attention();
                    let mut tab = widgets::tab(session.label())
                        .kind(kind)
                        .out_of_sync(attention)
                        .disconnected(session.is_disconnected())
                        .show(ui);
                    let mut lines: Vec<String> = session.details.tooltip.iter().cloned().collect();
                    if attention {
                        lines.push(WAITING.to_owned());
                    }
                    if let Some(reason) = session.lost_reason() {
                        lines.push(format!("Disconnected: {reason}"));
                    }
                    let tooltip = (!lines.is_empty()).then(|| lines.join("\n"));
                    if let Some(tooltip) = tooltip {
                        tab.response = tab.response.tooltip(&tooltip);
                    }
                    if tab.close.is_some_and(|c| c.clicked()) {
                        close = Some(i);
                    } else if tab.response.clicked() {
                        self.active = i;
                    }
                }
            });
        };
        if stacked {
            // The strip stands its first tab in from the top, and stacks them with no gap.
            ui.vertical(|ui| widgets::rows(ui, strip));
        } else {
            ui.allocate_ui(egui::vec2(ui.available_width(), t.tab_height()), |ui| {
                ui.horizontal(strip);
            });
        }
        if let Some(i) = close {
            self.close(i);
        }
    }

    /// The active Session's own bar, under its tab: the view switch, Refresh,
    /// the place, Search, and the Session's notes.
    fn session_bar(&mut self, ui: &mut egui::Ui) {
        let mut dismiss = false;
        let Some(session) = self.sessions.get(self.active) else {
            return;
        };
        let details = session.details.clone();
        let host = self
            .hosts
            .iter()
            .find(|h| !details.host_id.is_empty() && h.id == details.host_id);
        let mut toggle_pin = false;
        let busy = session.is_searching() || session.is_disconnected();
        let (view, place, refreshing) = (session.view(), session.place(), session.is_refreshing());
        let online = !session.is_disconnected();
        let moves = [session.can_move(-1), session.can_move(1)];
        let (sessions, index, settings) = (&mut self.sessions, self.active, &self.settings);
        let mut picked = None;
        let mut refresh = false;
        let compact = ui.available_width() < 720.;
        let mut go = None;
        let mut step = None;
        // Where the operator is, and the ways to move: at the leading end.
        let leading = |ui: &mut egui::Ui| {
            for (glyph, name, by, enabled) in [
                (Glyph::ChevronLeft, "Back", -1, moves[0]),
                (Glyph::ChevronRight, "Forward", 1, moves[1]),
            ] {
                let button = widgets::glyph_button(glyph).name(name);
                if ui.add_enabled(enabled, button).tooltip(name).clicked() {
                    step = Some(by);
                }
            }
            let button = widgets::glyph_button(Glyph::Refresh).name("Refresh");
            refresh = ui
                .add_enabled(!refreshing && !busy, button)
                .tooltip("Fetch the listing again")
                .clicked();
            if refreshing {
                ui.add(widgets::spinner());
            }
        };
        // The view, Search and the Location menu at the trailing end; the place between.
        let trailing = |ui: &mut egui::Ui| {
            let mut choice = view;
            let views = [(View::Tree, "Tree"), (View::Folder, "Folder")];
            if ui.add(widgets::segmented(&mut choice, &views)).changed() {
                picked = Some(choice);
            }
            if let Some(session) = sessions.get_mut(index) {
                if !compact {
                    session.search_field(ui, &settings.keys, settings);
                }
                widgets::menu_button("Location").show(ui, |ui| {
                    if ui.add(widgets::menu_item("Copy path")).clicked() {
                        ui.ctx().copy_text(place.clone());
                    }
                    if ui
                        .add_enabled(online, widgets::menu_item("Go to path"))
                        .clicked()
                    {
                        session.edit_path();
                    }
                    if let Some(host) = host {
                        let pinned = host.bookmarks.contains(&place);
                        let action = if pinned {
                            "Unpin current folder"
                        } else {
                            "Pin current folder"
                        };
                        toggle_pin = ui.add(widgets::menu_item(action)).clicked();
                        if !host.bookmarks.is_empty() {
                            widgets::menu_separator(ui);
                            ui.add(widgets::muted("Pinned folders"));
                            egui::ScrollArea::vertical()
                                .max_height(ui.available_height())
                                .show(ui, |ui| {
                                    for path in &host.bookmarks {
                                        if ui
                                            .add_enabled(online, widgets::menu_item(path))
                                            .clicked()
                                        {
                                            session.open_bookmark(path);
                                        }
                                    }
                                });
                        }
                    }
                });
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.shrink_clip_rect(ui.max_rect());
                go = widgets::crumbs(ui, &place, online);
            });
        };
        widgets::toolbar(ui, "session-toolbar", leading, trailing);
        if let Some(session) = sessions.get_mut(index) {
            if let Some(by) = step {
                session.move_in_history(by);
            }
            if let Some(path) = go.filter(|path| *path != place) {
                session.open_bookmark(&path);
            }
            if compact {
                let t = ui.tokens();
                ui.horizontal(|ui| {
                    ui.add_space(t.space.panel);
                    session.search_field(ui, &settings.keys, settings);
                });
                ui.add_space(t.space.bar_y);
            }
            session.path_bar(ui);
        }
        if details.note.is_some() || details.dismissible.is_some() {
            ui.horizontal_wrapped(|ui| {
                if let Some(note) = &details.note {
                    ui.add(widgets::warning_message(note));
                }
                if let Some(note) = &details.dismissible {
                    // The dismiss control first, so a long note cannot push it out.
                    dismiss = ui
                        .add(widgets::glyph_button(Glyph::Close).name("Dismiss"))
                        .tooltip("Dismiss")
                        .clicked();
                    ui.add(widgets::warning_message(note));
                }
            });
        }
        if toggle_pin && let Some(host) = self.hosts.iter_mut().find(|h| h.id == details.host_id) {
            if host.bookmarks.contains(&place) {
                host.bookmarks.retain(|path| path != &place);
            } else {
                host.bookmarks.push(place);
            }
            self.persist_hosts(ui.ctx(), false);
        }
        if dismiss && let Some(session) = self.sessions.get_mut(self.active) {
            session.details.dismissible = None;
        }
        if let (Some(view), Some(session)) = (picked, self.sessions.get_mut(self.active)) {
            session.set_view(view);
        }
        if let (true, Some(session)) = (refresh, self.sessions.get_mut(self.active)) {
            session.refresh();
        }
    }

    fn active_session(&mut self, ui: &mut egui::Ui) {
        self.session_bar(ui);
        // The listing takes keys only while no dialog is up.
        let keys = ui.is_enabled() && self.dialog.is_none();
        if let Some(session) = self.sessions.get_mut(self.active) {
            let frame = session::Frame {
                keymap: &self.settings.keys,
                settings: &self.settings,
                keys,
                audio: self.audio,
            };
            session.ui(ui, frame);
        }
    }

    fn open_host_dialog(&mut self) {
        if self.host_dialog.is_none() {
            if self.host_save_error.is_none()
                && let Some(store) = &self.host_store
            {
                match store.load_identified() {
                    Ok(hosts) => {
                        self.host_baseline = hosts.clone();
                        self.hosts = hosts;
                    }
                    Err(e) => self.host_save_error = Some(format!("Cannot load Hosts: {e}")),
                }
            }
            self.host_dialog = Some(HostDialog::new(&self.hosts, self.read_ssh_config()));
        }
    }

    /// Starts opening the Session `request` asks for.
    fn open_session(&mut self, ctx: &egui::Context, request: OpenRequest) {
        let plan = request.plan(&self.read_ssh_config());
        let opening = match plan.resolved {
            Ok(resolved) => {
                let options = Options {
                    known_hosts: self.known_hosts.clone(),
                    also_known_hosts: resolved.known_hosts,
                    connect_timeout: resolved.connect_timeout,
                    keepalive: resolved.server_alive_interval,
                    auth: plan.auth,
                    key_files: resolved.key_files,
                    identities_only: resolved.identities_only,
                    agent: match resolved.identity_agent {
                        IdentityAgent::Default => self.agent.clone(),
                        IdentityAgent::Off => None,
                        IdentityAgent::Socket(path) => Some(path),
                    },
                    remember: plan.remember.map(|host| ssh::Remember {
                        store: self.credentials.clone(),
                        host,
                    }),
                };
                let ssh = self.ssh.get_or_insert_with(|| {
                    let ctx = ctx.clone();
                    Ssh::new(move || ctx.request_repaint())
                });
                ssh.connect(resolved.target, options, &plan.folder)
            }
            Err(proxy) => Opening::failed(Failure::Unsupported(proxy.to_string())),
        };
        self.next_id += 1;
        let id = Id::NULL.with("opening").with(self.next_id);
        self.pending.push(Pending::new(id, plan.details, opening));
    }

    /// The window with no Session: the saved Hosts to open, and the way to add one.
    fn welcome(&mut self, ui: &mut egui::Ui) {
        let mut open = None;
        let mut add = false;
        let key = self.settings.keys.binding(Action::HostDialog);
        let text = "Browse remote filesystems over SSH";
        widgets::welcome(ui, "sshexplorer", text, |ui| {
            let t = ui.tokens();
            if !self.hosts.is_empty() {
                widgets::welcome_title(ui, "Saved Hosts");
                let room = (ui.available_height() - 4.0 * t.control_height()).max(0.0);
                egui::ScrollArea::vertical()
                    .max_height(room)
                    .show(ui, |ui| {
                        widgets::tiles(ui, |ui| {
                            for (i, host) in self.hosts.iter().enumerate() {
                                let name = if host.display_name.is_empty() {
                                    host.address.as_str()
                                } else {
                                    host.display_name.as_str()
                                };
                                let detail = host_detail(host);
                                if ui.add(widgets::host_tile(name, &detail)).clicked() {
                                    open = Some(i);
                                }
                            }
                        });
                    });
                ui.add_space(t.space.section);
            }
            ui.horizontal(|ui| {
                let button = widgets::button("Add a Host");
                let button = if self.hosts.is_empty() {
                    button.primary()
                } else {
                    button
                };
                add = ui.add(button).clicked();
                if let Some(key) = key {
                    ui.add(widgets::key_chip(key.label(cfg!(target_os = "macos"))));
                }
            });
        });
        if add {
            self.open_host_dialog();
        }
        if let Some(host) = open.and_then(|i| self.hosts.get(i)) {
            let request = OpenRequest::saved(host);
            self.open_session(ui.ctx(), request);
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if let Some(dialog) = &mut self.host_dialog {
            match dialog.show_with_save_error(
                ctx,
                &mut self.hosts,
                self.host_save_error.as_deref(),
                self.host_conflict,
            ) {
                Some(HostAction::Open(request)) => {
                    self.host_dialog = None;
                    self.open_session(ctx, *request);
                }
                Some(HostAction::Reload) => {
                    match self.host_store.as_ref().unwrap().load_identified() {
                        Ok(hosts) => {
                            self.hosts = hosts.clone();
                            self.host_baseline = hosts;
                            self.host_save_error = None;
                            self.host_conflict = false;
                            self.host_dialog =
                                Some(HostDialog::new(&self.hosts, self.read_ssh_config()));
                        }
                        Err(e) => self.host_save_error = Some(e.to_string()),
                    }
                }
                Some(action @ (HostAction::Changed | HostAction::Overwrite)) => {
                    dialog.take_forgotten();
                    self.persist_hosts(ctx, matches!(action, HostAction::Overwrite));
                }
                Some(HostAction::Dismiss) => self.host_dialog = None,
                None => {}
            }
        }
        let mut refused = false;
        self.forgetting.retain(|answer| match answer.try_recv() {
            Ok(result) => {
                refused |= result.is_err();
                false
            }
            Err(mpsc::TryRecvError::Empty) => true,
            Err(mpsc::TryRecvError::Disconnected) => false,
        });
        if refused && let Some(dialog) = &mut self.host_dialog {
            dialog.set_notice(credentials::NOT_REMOVED);
        }
        let mut i = 0;
        while i < self.pending.len() {
            match self.pending[i].show(ctx) {
                Some(outcome) => {
                    let mut pending = self.pending.remove(i);
                    if let Outcome::Opened(mut opened) = outcome {
                        pending.details.dismissible = opened.note.take();
                        self.sessions.push(Session::new(pending.details, opened));
                        self.active = self.sessions.len() - 1;
                    }
                }
                None => i += 1,
            }
        }
        // A reconnect that needs the operator asks whether its Session is shown or not.
        for session in &mut self.sessions {
            session.prompt(ctx);
        }
    }

    /// Removes `keys` from the credential store on a thread of its own, since the store
    /// may block. The answer wakes the frame.
    fn forget(&mut self, ctx: &egui::Context, keys: Vec<credentials::Key>) {
        let (sender, answer) = mpsc::channel();
        let store = self.credentials.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let mut result = Ok(());
            for key in &keys {
                if store.delete(key).is_err() {
                    result = Err(credentials::Refused);
                }
            }
            let _ = sender.send(result);
            ctx.request_repaint();
        });
        self.forgetting.push(answer);
    }

    /// Queues a Download of the active Session's Selection.
    fn download(&mut self) {
        if let Some(session) = self.sessions.get_mut(self.active) {
            let options = session.download_options(&self.settings);
            session.download(options);
        }
    }

    /// Closes Session `i`, first asking when a Download is running in it.
    fn close(&mut self, i: usize) {
        if self.sessions.get(i).is_some_and(Session::is_downloading) {
            self.confirm = Some(Closing::Session(i));
        } else {
            self.close_now(i);
        }
    }

    /// When the window is asked to close during a Download, holds it open to ask first.
    fn quit_requested(&mut self, ctx: &egui::Context) {
        let asked = ctx.input(|i| i.viewport().close_requested());
        if asked && !self.quitting && self.sessions.iter().any(Session::is_downloading) {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.confirm = Some(Closing::Quit);
        }
    }

    /// "A Download is running. Close anyway?"
    fn confirm_close(&mut self, ctx: &egui::Context) {
        let Some(closing) = self.confirm else {
            return;
        };
        let id = Id::NULL.with("sshexplorer-close-anyway");
        let mut answer = None;
        widgets::scrim(ctx, LayerId::new(Order::Middle, id));
        let shown = widgets::window(id, "Close anyway?")
            .closable()
            .centred()
            .show(ctx, |ui| {
                let t = ui.tokens();
                ui.label(
                    t.ty.body
                        .rich("A Download is running. Close anyway?")
                        .color(t.color.text.primary),
                );
                widgets::button_row(ui, |ui| {
                    if ui.add(widgets::button("Close anyway").danger()).clicked() {
                        answer = Some(true);
                    }
                    if ui.add(widgets::button("Cancel")).clicked() {
                        answer = Some(false);
                    }
                });
            });
        let escape = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        if shown.is_some_and(|s| s.close_clicked) || escape {
            answer = Some(false);
        }
        match answer {
            Some(true) => {
                self.confirm = None;
                match closing {
                    Closing::Session(i) => self.close_now(i),
                    Closing::Quit => {
                        // Dropping the Sessions cancels their Downloads.
                        self.sessions.clear();
                        self.quitting = true;
                        ctx.send_viewport_cmd(ViewportCommand::Close);
                    }
                }
            }
            Some(false) => self.confirm = None,
            None => {}
        }
    }

    fn close_now(&mut self, i: usize) {
        if i < self.sessions.len() {
            self.sessions.remove(i);
            if i < self.active {
                self.active -= 1;
            }
            self.active = self.active.min(self.sessions.len().saturating_sub(1));
        }
    }

    /// The app-wide keys: Settings and the UI scale; and the Sessions' keys and Show
    /// hidden entries, which wait while a dialog is up.
    fn keys(&mut self, ui: &mut egui::Ui, modal: bool) {
        let map = &self.settings.keys;
        let free = !modal && self.dialog.is_none();
        let popup_open = egui::Popup::is_any_open(ui.ctx());
        let sessions = [
            Action::CloseSession,
            Action::NextSession,
            Action::PreviousSession,
            Action::HostDialog,
            Action::ShowHidden,
            Action::Download,
            Action::TreeView,
            Action::FolderView,
            Action::Refresh,
            Action::Search,
        ];
        let [
            settings,
            up,
            down,
            reset,
            close,
            next,
            previous,
            host_dialog,
            hidden,
            download,
            tree_view,
            folder_view,
            refresh,
            search,
        ] = ui.input_mut(|i| {
            [
                Action::Settings,
                Action::UiScaleUp,
                Action::UiScaleDown,
                Action::UiScaleReset,
                Action::CloseSession,
                Action::NextSession,
                Action::PreviousSession,
                Action::HostDialog,
                Action::ShowHidden,
                Action::Download,
                Action::TreeView,
                Action::FolderView,
                Action::Refresh,
                Action::Search,
            ]
            .map(|action| {
                (free || !sessions.contains(&action))
                    && !(popup_open && action == Action::Download)
                    && map.take(i, action)
            })
        });
        if free && let Some(session) = self.sessions.get_mut(self.active) {
            ui.input_mut(|i| {
                if map.take(i, Action::GoToPath) {
                    session.edit_path();
                }
                if map.take(i, Action::Back) {
                    session.move_in_history(-1);
                }
                if map.take(i, Action::Forward) {
                    session.move_in_history(1);
                }
            });
        }
        if hidden {
            let b = &mut self.settings.browsing;
            b.show_hidden = !b.show_hidden;
        }
        if let Some(session) = self.sessions.get_mut(self.active) {
            if tree_view {
                session.set_view(View::Tree);
            }
            if folder_view {
                session.set_view(View::Folder);
            }
            if refresh {
                // F5 means Reconnect while Disconnected.
                if session.is_disconnected() {
                    session.reconnect_now();
                } else {
                    session.refresh();
                }
            }
            if search {
                session.focus_search();
            }
        }
        if close {
            self.close(self.active);
        }
        if download {
            self.download();
        }
        let count = self.sessions.len();
        if count > 0 {
            if next {
                self.active = (self.active + 1) % count;
            }
            if previous {
                self.active = (self.active + count - 1) % count;
            }
        }
        if host_dialog {
            self.open_host_dialog();
        }
        if settings {
            self.dialog = match self.dialog.take() {
                Some(_) => None,
                None => Some(SettingsDialog::open(ui.ctx())),
            };
        }
        let a = &mut self.settings.appearance;
        if up {
            a.scale_up();
        }
        if down {
            a.scale_down();
        }
        if reset {
            a.ui_scale = DEFAULT_UI_SCALE;
        }
    }

    fn settings_dialog(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.dialog else {
            return;
        };
        let page = Page::ALL[dialog.page.min(Page::ALL.len() - 1)];
        let id = egui::Id::NULL.with("sshexplorer-settings");
        let report = fonts::report(ctx);
        let settings = &mut self.settings;
        let SettingsDialog {
            page: current,
            ui_fonts,
            mono_fonts,
            keys: editor,
        } = dialog;
        if page != Page::Keys {
            editor.stop();
        }
        let mut retry = false;
        let shown = widgets::pages_dialog(id, "Settings", &PAGE_NAMES)
            .reset_label(page.reset_label())
            .show(ctx, current, |ui, page| {
                retry = widgets::retry_save(ui, self.settings_save_error.as_deref());
                match Page::ALL[page] {
                    Page::Appearance => {
                        appearance(ui, &mut settings.appearance, report, ui_fonts, mono_fonts)
                    }
                    Page::Browsing => browsing(ui, &mut settings.browsing),
                    Page::Preview => preview(ui, &mut settings.preview),
                    Page::Download => download_page(ui, settings),
                    Page::Keys => editor.ui(ui, &mut settings.keys),
                    Page::Connections => connections(ui, &mut settings.connections),
                    Page::Search => search_page(ui, &mut settings.search),
                }
            });
        if shown.reset {
            match page {
                Page::Appearance => settings.appearance = Appearance::default(),
                Page::Browsing => settings.browsing = Browsing::default(),
                Page::Preview => settings.preview = PreviewSettings::default(),
                Page::Download => {
                    settings.download = crate::settings::Download::default();
                    settings.search.follow_links = Search::default().follow_links;
                }
                Page::Connections => settings.connections = Connections::default(),
                Page::Search => settings.search = Search::default(),
                Page::Keys => {
                    settings.keys.reset_all();
                    editor.stop();
                }
            }
        }
        if retry {
            self.save();
        }
        if shown.close {
            self.dialog = None;
        }
    }
}

/// Where a saved Host connects, as its welcome tile says it: `account@address:port`,
/// leaving out what the Host leaves to the OpenSSH config.
fn host_detail(host: &Host) -> String {
    let mut text = String::new();
    if !host.account.is_empty() {
        text.push_str(&host.account);
        text.push('@');
    }
    text.push_str(&host.address);
    if let Some(port) = host.port {
        text.push_str(&format!(":{port}"));
    }
    text
}

/// The tooltip line of a Session tab whose Download waits for an answer.
const WAITING: &str = "A Download is waiting for an answer";

const THEMES: [(ThemeChoice, &str); 3] = [
    (ThemeChoice::System, "Follow system"),
    (ThemeChoice::Light, "Light"),
    (ThemeChoice::Dark, "Dark"),
];

const DENSITIES: [(Density, &str); 2] = [
    (Density::Comfortable, "Comfortable"),
    (Density::Compact, "Compact"),
];

const ANIMATES: [(Animate, &str); 3] = [
    (Animate::System, "Follow system"),
    (Animate::On, "On"),
    (Animate::Off, "Off"),
];

const EDGES: [(Edge, &str); 4] = [
    (Edge::Top, "Top"),
    (Edge::Left, "Left"),
    (Edge::Right, "Right"),
    (Edge::Bottom, "Bottom"),
];

fn percent(scale: u16) -> String {
    format!("{scale} %")
}

/// The Appearance page.
fn appearance(
    ui: &mut egui::Ui,
    a: &mut Appearance,
    report: Option<fonts::Report>,
    ui_fonts: &[String],
    mono_fonts: &[String],
) {
    widgets::form("appearance").show(ui, |ui| {
        widgets::form_row(ui, "Theme", |ui| {
            ui.add(widgets::segmented(&mut a.theme, &THEMES));
        });
        widgets::form_row(ui, "Accent", |ui| {
            widgets::dropdown("accent", a.accent.name())
                .name("Accent")
                .show(ui, |ui| {
                    for accent in Accent::ALL {
                        widgets::choice_value(ui, &mut a.accent, accent, accent.name());
                    }
                });
        });
        widgets::form_row(ui, "UI scale", |ui| {
            widgets::dropdown("ui-scale", percent(a.ui_scale))
                .name("UI scale")
                .show(ui, |ui| {
                    for scale in UI_SCALES {
                        widgets::choice_value(ui, &mut a.ui_scale, scale, percent(scale));
                    }
                });
        });
        widgets::form_row(ui, "Density", |ui| {
            ui.add(widgets::segmented(&mut a.density, &DENSITIES));
        });
        let ui_outcome = report.map(|r| r.ui);
        widgets::form_row(ui, "UI font", |ui| {
            font_field(
                ui,
                "ui-font",
                "UI font",
                &mut a.ui_font,
                ui_fonts,
                fonts::BUNDLED_UI,
                ui_outcome,
            );
        });
        let mono_outcome = report.map(|r| r.mono);
        widgets::form_row(ui, "Monospace font", |ui| {
            font_field(
                ui,
                "mono-font",
                "Monospace font",
                &mut a.mono_font,
                mono_fonts,
                fonts::BUNDLED_MONO,
                mono_outcome,
            );
        });
        widgets::form_row(ui, "Show icons", |ui| {
            ui.add(widgets::switch(&mut a.show_icons, "").name("Show icons"));
        });
        widgets::form_row(ui, "Animate", |ui| {
            ui.add(widgets::segmented(&mut a.animate, &ANIMATES));
        });
        widgets::form_row(ui, "Tab bar edge", |ui| {
            let edge = EDGES.iter().find(|(e, _)| *e == a.tab_bar_edge);
            let text = edge.map_or("Top", |(_, name)| name);
            widgets::dropdown("tab-bar-edge", text)
                .name("Tab bar edge")
                .show(ui, |ui| {
                    for (edge, name) in EDGES {
                        widgets::choice_value(ui, &mut a.tab_bar_edge, edge, name);
                    }
                });
        });
    });
}

/// The Browsing page.
fn browsing(ui: &mut egui::Ui, b: &mut Browsing) {
    widgets::form("browsing").show(ui, |ui| {
        widgets::form_row(ui, "Show hidden entries", |ui| {
            ui.add(widgets::switch(&mut b.show_hidden, "").name("Show hidden entries"));
        });
        widgets::form_row(ui, "Auto-refresh", |ui| {
            widgets::dropdown("auto-refresh", b.auto_refresh.name())
                .name("Auto-refresh")
                .show(ui, |ui| {
                    for every in AutoRefresh::ALL {
                        widgets::choice_value(ui, &mut b.auto_refresh, every, every.name());
                    }
                });
        });
    });
}

const SCRATCH: [(ScratchChoice, &str); 3] = [
    (ScratchChoice::Memory, "In memory"),
    (ScratchChoice::Temp, "System temp"),
    (ScratchChoice::Chosen, "Chosen"),
];

/// The Preview page.
fn preview(ui: &mut egui::Ui, p: &mut PreviewSettings) {
    widgets::form("preview").show(ui, |ui| {
        widgets::form_row(ui, "Size cap", |ui| {
            let mut cap = i64::from(p.size_cap);
            ui.add(widgets::stepper_field(&mut cap, 1..=4096).suffix("MiB"))
                .accessible_name("Size cap");
            p.size_cap = u32::try_from(cap.max(1)).unwrap_or(p.size_cap);
        });
        widgets::form_row(ui, "Autoplay audio and video", |ui| {
            ui.add(widgets::switch(&mut p.autoplay, "").name("Autoplay audio and video"));
        });
        widgets::form_row(ui, "Scratch folder", |ui| {
            ui.vertical(|ui| {
                ui.add(widgets::segmented(&mut p.scratch, &SCRATCH));
                if p.scratch == ScratchChoice::Chosen {
                    ui.horizontal(|ui| {
                        let mut path = p
                            .scratch_dir
                            .as_ref()
                            .map(|d| d.display().to_string())
                            .unwrap_or_default();
                        let field = ui
                            .add(widgets::text_field(&mut path).hint("Folder"))
                            .accessible_name("Scratch folder path");
                        if field.changed() {
                            p.scratch_dir = (!path.is_empty()).then(|| PathBuf::from(path));
                        }
                        if ui.add(widgets::button("Browse…")).clicked()
                            && let Some(dir) = rfd::FileDialog::new().pick_folder()
                        {
                            p.scratch_dir = Some(dir);
                        }
                    });
                }
            });
        });
    });
}

/// The Search page, with the Follow setting it shares with Download.
fn search_page(ui: &mut egui::Ui, s: &mut Search) {
    widgets::form("search").show(ui, |ui| {
        widgets::form_row(ui, "Follow symlinked directories", |ui| {
            ui.vertical(|ui| {
                ui.add(
                    widgets::switch(&mut s.follow_links, "").name("Follow symlinked directories"),
                );
                ui.add(widgets::form_hint("Shared with Download"));
            });
        });
        widgets::form_row(ui, "Match cap", |ui| {
            let mut cap = i64::from(s.match_cap);
            ui.add(
                widgets::stepper_field(&mut cap, 1..=1_000_000)
                    .step(100)
                    .suffix("matches"),
            )
            .accessible_name("Match cap");
            s.match_cap = u32::try_from(cap.max(1)).unwrap_or(s.match_cap);
        });
    });
}

/// The Download page, with the Follow setting it shares with Search.
fn download_page(ui: &mut egui::Ui, s: &mut AppSettings) {
    let switch = |ui: &mut egui::Ui, on: &mut bool, name: &str, hint: Option<&str>| {
        widgets::form_row(ui, name, |ui| {
            ui.vertical(|ui| {
                ui.add(widgets::switch(on, "").name(name));
                if let Some(hint) = hint {
                    ui.add(widgets::form_hint(hint));
                }
            });
        });
    };
    widgets::form("download").show(ui, |ui| {
        let d = &mut s.download;
        switch(ui, &mut d.keep_modified, "Keep modification time", None);
        switch(
            ui,
            &mut d.keep_executable,
            "Keep executable bit",
            Some("No effect on Windows"),
        );
        switch(
            ui,
            &mut s.search.follow_links,
            "Follow symlinked directories",
            Some("Shared with Search"),
        );
    });
}

/// The Connections page.
fn connections(ui: &mut egui::Ui, c: &mut Connections) {
    widgets::form("connections").show(ui, |ui| {
        widgets::form_row(ui, "Use ~/.ssh/config", |ui| {
            ui.add(widgets::switch(&mut c.use_ssh_config, "").name("Use ~/.ssh/config"));
        });
        widgets::form_row(ui, "Reconnect attempts", |ui| {
            widgets::dropdown("reconnect-attempts", c.reconnect_attempts.name())
                .name("Reconnect attempts")
                .show(ui, |ui| {
                    for attempts in ReconnectAttempts::ALL {
                        widgets::choice_value(
                            ui,
                            &mut c.reconnect_attempts,
                            attempts,
                            attempts.name(),
                        );
                    }
                });
        });
    });
}

/// A font setting: the bundled face or an installed family, and under it what became of
/// a family that cannot be used.
fn font_field(
    ui: &mut egui::Ui,
    id: &str,
    name: &str,
    chosen: &mut Option<String>,
    families: &[String],
    bundled: &str,
    outcome: Option<fonts::Outcome>,
) {
    let default = format!("Default ({bundled})");
    let mut choices = vec![widgets::Choice::new(&default).rule_below()];
    choices.extend(families.iter().map(|f| widgets::Choice::new(f)));
    let selected = match chosen.as_deref() {
        None => Some(0),
        Some(name) => families.iter().position(|f| f == name).map(|i| i + 1),
    };
    let text = chosen.clone().unwrap_or_else(|| default.clone());
    ui.vertical(|ui| {
        let picked = widgets::filter_dropdown(id, text)
            .name(name)
            .hint("Filter")
            .none_text("No installed font matches")
            .show(ui, &choices, selected);
        match picked.index {
            Some(0) => *chosen = None,
            Some(i) => *chosen = families.get(i - 1).cloned(),
            None => {}
        }
        if let (Some(family), Some(outcome)) = (chosen.as_deref(), outcome) {
            let why = match outcome {
                fonts::Outcome::NotInstalled => Some("is not installed"),
                fonts::Outcome::Rejected(_) => Some("cannot be used"),
                _ => None,
            };
            if let Some(why) = why {
                let message = format!("“{family}” {why}. Using the bundled {bundled}.");
                ui.add(widgets::warning_message(message));
            }
        }
    });
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        App::ui(self, ui);
    }
}
