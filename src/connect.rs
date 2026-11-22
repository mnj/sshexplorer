//! Opening a Session from the window: the Host dialog, and the prompts and refusals that
//! come up while the connection opens.
//!
//! The Host dialog lists the saved Hosts, New Host and Unsaved Session on the left, and
//! the picked entry's fields on the right. Up and Down pick an entry, Enter opens a
//! Session, and Escape, the close control or a backdrop click dismisses it. Save and
//! Delete change the saved Hosts and keep the dialog open. Remember is kept on the Host
//! but nothing is remembered yet, and OpenSSH config placeholders come later.
//! Each Session being opened is a [`Pending`], which polls its [`ssh::Opening`] once a
//! frame and never waits on it.

use std::path::PathBuf;

use egui::{Context, Id, Key, LayerId, Order};

use crate::hosts::{self, AuthMethod, Host, PasteError};
use crate::session::{self, Details};
use crate::ssh::{self, Event, Failure, Opened, PasswordPrompt, Target, TrustPrompt};
use crate::theme::TokensExt as _;
use crate::widgets;

/// Shows a modal dialog: the scrim over the window and the dialog centred on it.
fn modal<R>(
    ctx: &Context,
    id: Id,
    title: &str,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> Option<widgets::Shown<R>> {
    widgets::scrim(ctx, LayerId::new(Order::Middle, id));
    widgets::window(id, title)
        .closable()
        .centred()
        .show(ctx, add)
}

/// Whether a pointer click this frame landed outside `rect`: a click on the backdrop.
fn clicked_outside(ctx: &Context, rect: egui::Rect) -> bool {
    ctx.input(|i| {
        i.pointer.primary_clicked()
            && i.pointer
                .interact_pos()
                .is_some_and(|pos| !rect.contains(pos))
    })
}

/// Text in the body style, wrapping at the dialog's width.
fn paragraph(ui: &mut egui::Ui, text: &str) {
    let t = ui.tokens();
    ui.label(t.ty.body.rich(text).color(t.color.text.primary));
}

/// The local OS user name: the account when the Host leaves it empty.
fn local_user() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default()
}

/// The left column's width.
const ENTRY_WIDTH: f32 = 180.0;

/// The Authentication method choices, as the dropdown names them.
const AUTH_METHODS: [(AuthMethod, &str); 4] = [
    (AuthMethod::Automatic, "Automatic"),
    (AuthMethod::Password, "Password"),
    (AuthMethod::KeyFile, "Key file"),
    (AuthMethod::Agent, "Agent"),
];

/// An entry of the Host dialog's left column.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Entry {
    /// The saved Host at this index.
    Saved(usize),
    NewHost,
    UnsavedSession,
}

/// The fields as typed, for the entry picked.
struct Fields {
    display_name: String,
    address: String,
    port: String,
    account: String,
    auth: AuthMethod,
    key_file: String,
    local_folder: String,
    initial_folder: String,
    readonly: bool,
    remember: bool,
}

impl Fields {
    fn of(host: &Host) -> Fields {
        Fields {
            display_name: host.display_name.clone(),
            address: host.address.clone(),
            port: host.port.map(|p| p.to_string()).unwrap_or_default(),
            account: host.account.clone(),
            auth: host.auth,
            key_file: host.key_file.clone(),
            local_folder: host.local_folder.display().to_string(),
            initial_folder: host.initial_folder.clone(),
            readonly: host.readonly,
            remember: host.remember,
        }
    }

    /// The Host the fields describe, with a valid port or none.
    fn host(&self) -> Host {
        Host {
            display_name: self.display_name.trim().to_owned(),
            address: self.address.trim().to_owned(),
            port: hosts::port_value(&self.port).ok().flatten(),
            account: self.account.trim().to_owned(),
            auth: self.auth,
            key_file: self.key_file.trim().to_owned(),
            local_folder: PathBuf::from(self.local_folder.trim()),
            initial_folder: self.initial_folder.trim().to_owned(),
            readonly: self.readonly,
            remember: self.remember,
        }
    }
}

/// The Host dialog while it is open: the entry picked in the left column, and its fields.
pub struct HostDialog {
    entry: Entry,
    fields: Fields,
    /// Why pasted text stayed in Address, since focus last left it.
    pasted: Vec<PasteError>,
    shown_before: bool,
}

/// What the operator did in the Host dialog.
pub enum HostAction {
    Open(OpenRequest),
    /// A Host was saved or deleted: the list is to be written out.
    Changed,
    Dismiss,
}

/// A Session to open: where, the folder to open in, and what its tab and toolbar show.
pub struct OpenRequest {
    pub target: Target,
    /// The Initial remote folder; empty for the home directory.
    pub folder: String,
    pub details: Details,
}

/// A saved Host's name in the left column: its display name, else its address.
fn entry_name(host: &Host) -> &str {
    if host.display_name.trim().is_empty() {
        &host.address
    } else {
        &host.display_name
    }
}

impl HostDialog {
    /// The dialog with the first saved Host picked, or New Host when there is none.
    pub fn new(hosts: &[Host]) -> HostDialog {
        let mut dialog = HostDialog {
            entry: Entry::NewHost,
            fields: Fields::of(&Host::default()),
            pasted: Vec::new(),
            shown_before: false,
        };
        if !hosts.is_empty() {
            dialog.pick(Entry::Saved(0), hosts);
        }
        dialog
    }

    /// Picks `entry` and shows its fields; unsaved changes to the last one are dropped.
    fn pick(&mut self, entry: Entry, hosts: &[Host]) {
        self.entry = entry;
        self.pasted.clear();
        self.fields = match entry {
            Entry::Saved(i) => Fields::of(&hosts[i]),
            Entry::NewHost => Fields::of(&Host::default()),
            Entry::UnsavedSession => Fields {
                remember: false,
                ..Fields::of(&Host::default())
            },
        };
    }

    /// Every entry, in the left column's order.
    fn entries(hosts: &[Host]) -> Vec<Entry> {
        (0..hosts.len())
            .map(Entry::Saved)
            .chain([Entry::NewHost, Entry::UnsavedSession])
            .collect()
    }

    /// Picks the entry `step` away from the one picked, if there is one.
    fn step(&mut self, step: isize, hosts: &[Host]) {
        let entries = Self::entries(hosts);
        let at = entries.iter().position(|e| *e == self.entry).unwrap_or(0);
        if let Some(next) = at.checked_add_signed(step).and_then(|i| entries.get(i)) {
            self.pick(*next, hosts);
        }
    }

    /// Draws the dialog for one frame. Save and Delete change `hosts` and say so with
    /// [`HostAction::Changed`].
    pub fn show(&mut self, ctx: &Context, hosts: &mut Vec<Host>) -> Option<HostAction> {
        let id = Id::NULL.with("host-dialog");
        if !egui::Popup::is_any_open(ctx) {
            let (up, down) = ctx.input_mut(|i| {
                (
                    i.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
                    i.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
                )
            });
            if up {
                self.step(-1, hosts);
            }
            if down {
                self.step(1, hosts);
            }
        }
        let port = hosts::port_value(&self.fields.port);
        let filled = port.is_ok() && !self.fields.address.trim().is_empty();
        let mut clicked = None;
        let mut picked = None;
        let shown = modal(ctx, id, "Hosts", |ui| {
            let t = ui.tokens();
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(ENTRY_WIDTH);
                    widgets::rows(ui, |ui| {
                        for entry in Self::entries(hosts) {
                            let name = match entry {
                                Entry::Saved(i) => entry_name(&hosts[i]),
                                Entry::NewHost => "New Host",
                                Entry::UnsavedSession => "Unsaved Session",
                            };
                            let selection = if entry == self.entry {
                                widgets::Selection::Live
                            } else {
                                widgets::Selection::None
                            };
                            let row = widgets::list_row(name)
                                .selection(selection)
                                .width(ENTRY_WIDTH)
                                .show(ui);
                            if row.response.clicked() {
                                picked = Some(entry);
                            }
                        }
                    });
                });
                ui.add_space(t.space.section);
                ui.vertical(|ui| {
                    self.fields_ui(ui, id);
                    widgets::button_row(ui, |ui| {
                        let open = widgets::button("Open Session").primary();
                        if ui.add_enabled(filled, open).clicked() {
                            clicked = Some(Clicked::Open);
                        }
                        if self.entry != Entry::UnsavedSession
                            && ui.add_enabled(filled, widgets::button("Save")).clicked()
                        {
                            clicked = Some(Clicked::Save);
                        }
                        if matches!(self.entry, Entry::Saved(_))
                            && ui.add(widgets::button("Delete")).clicked()
                        {
                            clicked = Some(Clicked::Delete);
                        }
                    });
                });
            });
        });
        // The click that opened the dialog is not a click on its backdrop.
        let first = !std::mem::replace(&mut self.shown_before, true);
        let shown = shown?;
        if let Some(entry) = picked {
            self.pick(entry, hosts);
            return None;
        }
        let enter = ctx.input(|i| i.key_pressed(Key::Enter));
        match clicked {
            Some(Clicked::Open) => return Some(HostAction::Open(self.request())),
            Some(Clicked::Save) => {
                let host = self.fields.host();
                match self.entry {
                    Entry::Saved(i) => hosts[i] = host,
                    _ => {
                        hosts.push(host);
                        self.entry = Entry::Saved(hosts.len() - 1);
                    }
                }
                return Some(HostAction::Changed);
            }
            Some(Clicked::Delete) => {
                if let Entry::Saved(i) = self.entry {
                    hosts.remove(i);
                    let next = match hosts.len() {
                        0 => Entry::NewHost,
                        n => Entry::Saved(i.min(n - 1)),
                    };
                    self.pick(next, hosts);
                }
                return Some(HostAction::Changed);
            }
            None if enter && filled => return Some(HostAction::Open(self.request())),
            None => {}
        }
        let escape = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape));
        let backdrop = !first && clicked_outside(ctx, shown.response.rect);
        if shown.close_clicked || escape || backdrop {
            return Some(HostAction::Dismiss);
        }
        None
    }

    /// The form: every field of the entry picked, and the messages under them.
    fn fields_ui(&mut self, ui: &mut egui::Ui, id: Id) {
        let t = ui.tokens();
        let f = &mut self.fields;
        let pasted = &mut self.pasted;
        let first = !self.shown_before;
        let entry = self.entry;
        let message = |ui: &mut egui::Ui, text: &str| {
            ui.label("");
            ui.add(widgets::invalid_message(text));
            ui.end_row();
        };
        widgets::form(id.with("form")).show(ui, |ui| {
            widgets::form_row(ui, "Display name", |ui| {
                ui.add(widgets::text_field(&mut f.display_name))
                    .accessible_name("Display name");
            });
            widgets::form_row(ui, "Address", |ui| {
                ui.horizontal(|ui| {
                    let address = ui
                        .add(widgets::text_field(&mut f.address).id(id.with("address")))
                        .accessible_name("Address");
                    if first {
                        address.request_focus();
                    }
                    if address.changed() {
                        pasted.clear();
                    }
                    if address.lost_focus() {
                        let split = hosts::split_pasted(&f.address, &f.port, &f.account);
                        f.address = split.address;
                        f.port = split.port;
                        f.account = split.account;
                        *pasted = split.errors;
                    }
                    let bad_port = hosts::port_value(&f.port).is_err();
                    ui.add(
                        widgets::number_field(&mut f.port)
                            .hint("22")
                            .width(t.size.field / 4.0)
                            .invalid(bad_port),
                    )
                    .accessible_name("Port");
                });
            });
            for error in pasted.iter() {
                message(ui, &error.to_string());
            }
            if hosts::port_value(&f.port).is_err() {
                message(ui, "Port must be 1 to 65535");
            }
            widgets::form_row(ui, "Account", |ui| {
                ui.add(widgets::text_field(&mut f.account).hint(local_user()))
                    .accessible_name("Account");
            });
            widgets::form_row(ui, "Authentication method", |ui| {
                let name = AUTH_METHODS
                    .iter()
                    .find(|(m, _)| *m == f.auth)
                    .map_or("Automatic", |(_, name)| name);
                widgets::dropdown(id.with("auth"), name)
                    .name("Authentication method")
                    .show(ui, |ui| {
                        for (method, name) in AUTH_METHODS {
                            widgets::choice_value(ui, &mut f.auth, method, name);
                        }
                    });
            });
            widgets::form_row(ui, "Key file", |ui| {
                ui.add(widgets::text_field(&mut f.key_file))
                    .accessible_name("Key file");
            });
            widgets::form_row(ui, "Local folder", |ui| {
                ui.add(widgets::text_field(&mut f.local_folder))
                    .accessible_name("Local folder");
            });
            widgets::form_row(ui, "Initial remote folder", |ui| {
                ui.add(widgets::text_field(&mut f.initial_folder).hint("Home directory"))
                    .accessible_name("Initial remote folder");
            });
            widgets::form_row(ui, "Readonly", |ui| {
                ui.add(widgets::switch(&mut f.readonly, "").name("Readonly"));
            });
            if entry != Entry::UnsavedSession {
                widgets::form_row(ui, "Remember", |ui| {
                    ui.add(widgets::switch(&mut f.remember, "").name("Remember"));
                });
            }
        });
    }

    /// The Session the fields describe.
    fn request(&self) -> OpenRequest {
        let host = self.fields.host();
        let target = Target {
            address: host.address.clone(),
            port: host.port.unwrap_or(22),
            account: if host.account.is_empty() {
                local_user()
            } else {
                host.account.clone()
            },
        };
        let unsaved = session::label(&target);
        let (label, tooltip) = match self.entry {
            Entry::Saved(_) | Entry::NewHost if !host.display_name.is_empty() => {
                (host.display_name.clone(), Some(unsaved))
            }
            _ => (unsaved, None),
        };
        OpenRequest {
            folder: host.initial_folder.clone(),
            details: Details {
                label,
                tooltip,
                readonly: host.readonly,
                local_folder: host.local_folder,
            },
            target,
        }
    }
}

/// A button of the Host dialog that was clicked.
enum Clicked {
    Open,
    Save,
    Delete,
}

/// A Session being opened, and whatever it is asking the operator.
pub struct Pending {
    id: Id,
    /// What the Session takes from its Host once it is open.
    pub details: Details,
    opening: ssh::Opening,
    prompt: Option<Prompt>,
    password: String,
}

enum Prompt {
    Trust(TrustPrompt),
    Password(PasswordPrompt),
    Failed(Failure),
}

/// How a [`Pending`] ended.
pub enum Outcome {
    Opened(Opened),
    /// Cancelled, or refused and the refusal dismissed.
    Ended,
}

impl Pending {
    pub fn new(id: Id, details: Details, opening: ssh::Opening) -> Self {
        Pending {
            id,
            details,
            opening,
            prompt: None,
            password: String::new(),
        }
    }

    /// Whether it is showing a dialog, which takes the input from the window.
    pub fn is_modal(&self) -> bool {
        self.prompt.is_some()
    }

    /// Takes in what arrived and draws the prompt, if any, for one frame.
    pub fn show(&mut self, ctx: &Context) -> Option<Outcome> {
        if self.prompt.is_none() {
            match self.opening.try_next()? {
                Event::TrustHostKey(prompt) => self.prompt = Some(Prompt::Trust(prompt)),
                Event::Password(prompt) => {
                    self.password.clear();
                    self.prompt = Some(Prompt::Password(prompt));
                }
                Event::Opened(opened) => return Some(Outcome::Opened(opened)),
                Event::Failed(Failure::Cancelled) => return Some(Outcome::Ended),
                Event::Failed(failure) => self.prompt = Some(Prompt::Failed(failure)),
            }
        }
        match self.prompt.take()? {
            Prompt::Trust(prompt) => self.show_trust(ctx, prompt),
            Prompt::Password(prompt) => self.show_password(ctx, prompt),
            Prompt::Failed(failure) => self.show_failure(ctx, failure),
        }
    }

    fn show_trust(&mut self, ctx: &Context, prompt: TrustPrompt) -> Option<Outcome> {
        let mut answer = None;
        let shown = modal(ctx, self.id.with("trust"), "Unknown host key", |ui| {
            paragraph(
                ui,
                &format!(
                    "{} is not in known_hosts. Its key's fingerprint is\n{}\n\
                     Trust adds the key to known_hosts.",
                    prompt.host, prompt.fingerprint
                ),
            );
            widgets::button_row(ui, |ui| {
                if ui.add(widgets::button("Trust").primary()).clicked() {
                    answer = Some(true);
                }
                if ui.add(widgets::button("Cancel")).clicked() {
                    answer = Some(false);
                }
            });
        });
        let escape = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape));
        if shown.as_ref().is_some_and(|s| s.close_clicked) || escape {
            answer = Some(false);
        }
        match answer {
            Some(true) => prompt.trust(),
            Some(false) => prompt.cancel(),
            None => self.prompt = Some(Prompt::Trust(prompt)),
        }
        None
    }

    fn show_password(&mut self, ctx: &Context, prompt: PasswordPrompt) -> Option<Outcome> {
        let mut answer = None;
        let id = self.id.with("password");
        let shown = modal(ctx, id, "Password", |ui| {
            paragraph(ui, &format!("Password for {}", prompt.account));
            if prompt.rejected {
                ui.add(widgets::invalid_message("The password was rejected."));
            }
            let field = ui
                .add(
                    widgets::text_field(&mut self.password)
                        .id(id.with("field"))
                        .password(),
                )
                .accessible_name("Password");
            if !field.has_focus() && !ui.memory(|m| m.focused().is_some()) {
                field.request_focus();
            }
            widgets::button_row(ui, |ui| {
                if ui.add(widgets::button("Connect").primary()).clicked() {
                    answer = Some(true);
                }
                if ui.add(widgets::button("Cancel")).clicked() {
                    answer = Some(false);
                }
            });
        });
        let (enter, escape) = ctx.input_mut(|i| {
            (
                i.key_pressed(Key::Enter),
                i.consume_key(egui::Modifiers::NONE, Key::Escape),
            )
        });
        if enter {
            answer = Some(true);
        }
        if shown.as_ref().is_some_and(|s| s.close_clicked) || escape {
            answer = Some(false);
        }
        match answer {
            Some(true) => prompt.enter(std::mem::take(&mut self.password)),
            Some(false) => {
                self.password.clear();
                prompt.cancel();
            }
            None => self.prompt = Some(Prompt::Password(prompt)),
        }
        None
    }

    fn show_failure(&mut self, ctx: &Context, failure: Failure) -> Option<Outcome> {
        let title = match failure {
            Failure::HostKeyChanged(_) => "Host key changed",
            Failure::HostKeyRevoked(_) => "Host key revoked",
            _ => "Session did not open",
        };
        let mut done = false;
        let shown = modal(ctx, self.id.with("failed"), title, |ui| {
            paragraph(ui, &failure.to_string());
            widgets::button_row(ui, |ui| {
                done = ui.add(widgets::button("OK").primary()).clicked();
            });
        });
        let escape = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape));
        if done || escape || shown.is_some_and(|s| s.close_clicked) {
            return Some(Outcome::Ended);
        }
        self.prompt = Some(Prompt::Failed(failure));
        None
    }
}
