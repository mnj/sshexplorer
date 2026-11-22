//! The main window: the toolbar along the top and, once one is open, the Sessions as tabs;
//! and the Settings dialog over it.
//!
//! Launch opens no Session and lists no Hosts. Hosts, or Ctrl+O, opens the Host dialog;
//! saved Hosts are kept in `hosts.toml` beside the settings. The tabs run along the
//! Tab bar edge, and Ctrl+Tab and Ctrl+Shift+Tab cycle them. The toolbar shows the active
//! Session's Readonly indicator and Local folder. The Settings
//! gear, and Ctrl+, from anywhere, opens the Settings dialog. Every change there applies
//! on the next frame and is saved at once. The SSH stack starts with the first Session
//! opened, and every network step runs on it, off this frame thread.

use std::path::PathBuf;

use egui::Id;

use crate::binding_editor::BindingEditor;
use crate::connect::{HostAction, HostDialog, Outcome, Pending};
use crate::hosts::{Host, HostStore};
use crate::keymap::{Action, KeyMap};
use crate::session::{self, Session, View};
use crate::settings::{
    Animate, AppSettings, Appearance, Browsing, DEFAULT_UI_SCALE, Edge, Store, ThemeChoice,
    UI_SCALES,
};
use crate::ssh::Ssh;
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
    known_hosts: PathBuf,
    /// The saved Hosts, in the Host dialog's order.
    hosts: Vec<Host>,
    /// Where Hosts are saved; `None` keeps them for this run only.
    host_store: Option<HostStore>,
    host_dialog: Option<HostDialog>,
    pending: Vec<Pending>,
    /// Dropped before `ssh`, so that each closes its connection on a live runtime.
    sessions: Vec<Session>,
    active: usize,
    ssh: Option<Ssh>,
    next_id: u64,
}

impl Default for App {
    fn default() -> Self {
        App::with_known_hosts(default_known_hosts())
    }
}

/// `~/.ssh/known_hosts` (`%USERPROFILE%\.ssh\known_hosts` on Windows).
// TODO(#24): `UserKnownHostsFile` from the OpenSSH config.
fn default_known_hosts() -> PathBuf {
    std::env::home_dir()
        .unwrap_or_default()
        .join(".ssh")
        .join("known_hosts")
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
            hosts: Vec::new(),
            host_store: None,
            host_dialog: None,
            pending: Vec::new(),
            sessions: Vec::new(),
            active: 0,
            ssh: None,
            next_id: 0,
        }
    }

    /// The app with the settings `store` holds, and the Hosts saved beside them, saving
    /// every change back.
    pub fn with_store(store: Store) -> App {
        let host_store = HostStore::beside(store.path());
        App {
            settings: store.load(),
            hosts: host_store.load(),
            host_store: Some(host_store),
            store: Some(store),
            ..App::default()
        }
    }

    /// The same app, checking host keys against the `known_hosts` file at `path`.
    pub fn known_hosts_at(mut self, path: PathBuf) -> App {
        self.known_hosts = path;
        self
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

    fn save_hosts(&self) {
        if let Some(store) = &self.host_store
            && let Err(err) = store.save(&self.hosts)
        {
            eprintln!(
                "sshexplorer: cannot save Hosts to {}: {err}",
                store.path().display()
            );
        }
    }

    fn save(&self) {
        if let Some(store) = &self.store
            && let Err(err) = store.save(&self.settings)
        {
            eprintln!(
                "sshexplorer: cannot save settings to {}: {err}",
                store.path().display()
            );
        }
    }

    /// Draws one frame in `ui`, which covers the window.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.apply(&ctx);
        let before = self.settings.clone();
        let modal = self.host_dialog.is_some() || self.pending.iter().any(Pending::is_modal);
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
        self.settings_dialog(&ctx);
        if self.settings != before {
            self.save();
            self.apply(&ctx);
        }
    }

    /// The toolbar, the tab strip and the active Session; whether the gear was clicked.
    fn window(&mut self, ui: &mut egui::Ui) -> bool {
        let mut hosts = false;
        let mut open_settings = false;
        let active = self.sessions.get(self.active).map(|s| &s.details);
        // The active Session's view and place; the view the switch picks.
        let place = self
            .sessions
            .get(self.active)
            .map(|s| (s.view(), s.place()));
        let mut picked = None;
        widgets::toolbar(
            ui,
            |ui| {
                hosts = ui.add(widgets::button("Hosts").flat()).clicked();
                if let Some((view, place)) = &place {
                    let mut choice = *view;
                    let views = [(View::Tree, "Tree"), (View::Folder, "Folder")];
                    if ui.add(widgets::segmented(&mut choice, &views)).changed() {
                        picked = Some(choice);
                    }
                    ui.label(ui.tokens().ty.body.rich(place.as_str()))
                        .tooltip("Current directory");
                }
                if let Some(details) = active {
                    if details.readonly {
                        ui.add(widgets::badge("Readonly"))
                            .tooltip("This Session cannot change the remote filesystem");
                    }
                    let t = ui.tokens();
                    let folder = details.local_folder.display().to_string();
                    ui.label(t.ty.body.rich(folder).color(t.color.text.muted))
                        .tooltip("Local folder");
                }
            },
            |ui| {
                open_settings = ui
                    .add(widgets::glyph_button(Glyph::Gear).name("Settings"))
                    .tooltip("Settings")
                    .clicked();
            },
        );
        if hosts {
            self.open_host_dialog();
        }
        if let (Some(view), Some(session)) = (picked, self.sessions.get_mut(self.active)) {
            session.set_view(view);
        }
        if self.sessions.is_empty() {
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
                    let mut tab = widgets::tab(session.label()).kind(kind).show(ui);
                    if let Some(tooltip) = &session.details.tooltip {
                        tab.response = tab.response.tooltip(tooltip);
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

    fn active_session(&mut self, ui: &mut egui::Ui) {
        // The listing takes keys only while no dialog is up.
        let keys = ui.is_enabled() && self.dialog.is_none();
        if let Some(session) = self.sessions.get_mut(self.active) {
            let frame = session::Frame {
                keymap: &self.settings.keys,
                settings: &self.settings,
                keys,
            };
            session.ui(ui, frame);
        }
    }

    fn open_host_dialog(&mut self) {
        if self.host_dialog.is_none() {
            self.host_dialog = Some(HostDialog::new(&self.hosts));
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if let Some(dialog) = &mut self.host_dialog {
            match dialog.show(ctx, &mut self.hosts) {
                Some(HostAction::Open(request)) => {
                    self.host_dialog = None;
                    let ssh = self.ssh.get_or_insert_with(|| {
                        let ctx = ctx.clone();
                        Ssh::new(move || ctx.request_repaint())
                    });
                    let opening =
                        ssh.open_in(request.target, self.known_hosts.clone(), &request.folder);
                    self.next_id += 1;
                    let id = Id::NULL.with("opening").with(self.next_id);
                    self.pending
                        .push(Pending::new(id, request.details, opening));
                }
                Some(HostAction::Changed) => self.save_hosts(),
                Some(HostAction::Dismiss) => self.host_dialog = None,
                None => {}
            }
        }
        let mut i = 0;
        while i < self.pending.len() {
            match self.pending[i].show(ctx) {
                Some(outcome) => {
                    let pending = self.pending.remove(i);
                    if let Outcome::Opened(opened) = outcome {
                        self.sessions.push(Session::new(pending.details, opened));
                        self.active = self.sessions.len() - 1;
                    }
                }
                None => i += 1,
            }
        }
    }

    fn close(&mut self, i: usize) {
        if i < self.sessions.len() {
            self.sessions.remove(i);
            self.active = self.active.min(self.sessions.len().saturating_sub(1));
        }
    }

    /// The app-wide keys: Settings and the UI scale; and the Sessions' keys and Show
    /// hidden entries, which wait while a dialog is up.
    fn keys(&mut self, ui: &mut egui::Ui, modal: bool) {
        let map = &self.settings.keys;
        let free = !modal && self.dialog.is_none();
        let sessions = [
            Action::CloseSession,
            Action::NextSession,
            Action::PreviousSession,
            Action::HostDialog,
            Action::ShowHidden,
            Action::TreeView,
            Action::FolderView,
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
            tree_view,
            folder_view,
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
                Action::TreeView,
                Action::FolderView,
            ]
            .map(|action| (free || !sessions.contains(&action)) && map.take(i, action))
        });
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
        }
        if close {
            self.close(self.active);
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
        let shown = widgets::pages_dialog(id, "Changes apply at once", &PAGE_NAMES)
            .reset_label(page.reset_label())
            .show(ctx, current, |ui, page| match Page::ALL[page] {
                Page::Appearance => {
                    appearance(ui, &mut settings.appearance, report, ui_fonts, mono_fonts)
                }
                Page::Browsing => browsing(ui, &mut settings.browsing),
                Page::Keys => editor.ui(ui, &mut settings.keys),
                _ => {
                    ui.add(widgets::form_name("Nothing to set here yet."));
                }
            });
        if shown.reset {
            match page {
                Page::Appearance => settings.appearance = Appearance::default(),
                Page::Browsing => settings.browsing = Browsing::default(),
                Page::Keys => {
                    settings.keys.reset_all();
                    editor.stop();
                }
                _ => {}
            }
        }
        if shown.close {
            self.dialog = None;
        }
    }
}

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
