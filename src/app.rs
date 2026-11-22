//! The main window: the toolbar along the top and, once one is open, the Sessions as tabs.
//!
//! Launch opens no Session and lists no Hosts. Hosts opens the Host dialog; the Settings
//! gear is drawn here and does nothing yet. The SSH stack starts with the first Session
//! opened, and every network step runs on it, off this frame thread.

use std::path::PathBuf;

use egui::{Id, Key, KeyboardShortcut, Modifiers};

use crate::connect::{HostAction, HostDialog, Outcome, Pending};
use crate::session::Session;
use crate::ssh::Ssh;
use crate::theme::{self, TokensExt as _};
use crate::widgets::{self, Glyph, TabKind, Tooltip as _};

/// Closes the active Session.
// TODO(key map): register Ctrl+W (Cmd+W on macOS) in the key map once the registry exists.
const CLOSE_SESSION: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::W);

/// The app's state across frames.
pub struct App {
    /// The `known_hosts` file host keys are checked against, and Trust appends to.
    known_hosts: PathBuf,
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
/// both themes' styles, following the system's light or dark.
pub fn setup(ctx: &egui::Context) {
    theme::install(ctx, egui::ThemePreference::System);
}

impl App {
    /// An app that checks host keys against the `known_hosts` file at `path`.
    pub fn with_known_hosts(path: PathBuf) -> Self {
        App {
            known_hosts: path,
            host_dialog: None,
            pending: Vec::new(),
            sessions: Vec::new(),
            active: 0,
            ssh: None,
            next_id: 0,
        }
    }

    /// Draws one frame in `ui`, which covers the window.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let modal = self.host_dialog.is_some() || self.pending.iter().any(Pending::is_modal);
        ui.add_enabled_ui(!modal, |ui| {
            self.window(ui);
        });
        if !modal && ctx.input_mut(|i| i.consume_shortcut(&CLOSE_SESSION)) {
            self.close(self.active);
        }
        self.dialogs(&ctx);
    }

    fn window(&mut self, ui: &mut egui::Ui) {
        let mut hosts = false;
        widgets::toolbar(
            ui,
            |ui| {
                hosts = ui.add(widgets::button("Hosts").flat()).clicked();
            },
            |ui| {
                ui.add(widgets::glyph_button(Glyph::Gear).name("Settings"))
                    .tooltip("Settings");
            },
        );
        if hosts {
            self.host_dialog = Some(HostDialog::default());
        }
        if self.sessions.is_empty() {
            return;
        }
        let t = ui.tokens();
        let mut close = None;
        ui.allocate_ui(egui::vec2(ui.available_width(), t.tab_height()), |ui| {
            ui.horizontal(|ui| {
                widgets::tab_strip(ui, |ui| {
                    for (i, session) in self.sessions.iter().enumerate() {
                        let kind = if i == self.active {
                            TabKind::Active
                        } else {
                            TabKind::Back
                        };
                        let tab = widgets::tab(&session.label).kind(kind).show(ui);
                        if tab.close.is_some_and(|c| c.clicked()) {
                            close = Some(i);
                        } else if tab.response.clicked() {
                            self.active = i;
                        }
                    }
                });
            });
        });
        if let Some(i) = close {
            self.close(i);
        }
        if let Some(session) = self.sessions.get(self.active) {
            session.ui(ui);
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if let Some(dialog) = &mut self.host_dialog {
            match dialog.show(ctx) {
                Some(HostAction::Open(target)) => {
                    self.host_dialog = None;
                    let ssh = self.ssh.get_or_insert_with(|| {
                        let ctx = ctx.clone();
                        Ssh::new(move || ctx.request_repaint())
                    });
                    let opening = ssh.open(target.clone(), self.known_hosts.clone());
                    self.next_id += 1;
                    let id = Id::NULL.with("opening").with(self.next_id);
                    self.pending.push(Pending::new(id, target, opening));
                }
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
                        self.sessions.push(Session::new(&pending.target, opened));
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
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        App::ui(self, ui);
    }
}
