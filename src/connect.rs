//! Opening a Session from the window: the Host dialog, and the prompts and refusals that
//! come up while the connection opens.
//!
//! The Host dialog lists the saved Hosts, New Host and Unsaved Session on the left, and
//! the picked entry's fields on the right. Up and Down pick an entry, Enter opens a
//! Session, and Escape, the close control or a backdrop click dismisses it. Save and
//! Delete change the saved Hosts and keep the dialog open; a save or delete that drops
//! a remembered secret says which entries to remove from the credential store, and the
//! dialog says when the store did not remove one. An empty Port, Account or Key file shows what the
//! OpenSSH config gives the Address typed, and a filled Port or Account that differs
//! from it says so under the field.
//! Each Session being opened is a [`Pending`], which polls its [`ssh::Opening`] once a
//! frame and never waits on it.

use std::path::PathBuf;

use egui::{Context, Id, Key, LayerId, Order};

use crate::credentials::{self, Kind};
use crate::hosts::{self, AuthMethod, Host, PasteError};
use crate::openssh_config::{CONFIG_NAME, Config, HostFields, NeedsProxy, Resolved};
use crate::session::{self, Details};
use crate::ssh::{
    self, Event, Failure, Opened, PassphrasePrompt, PasswordPrompt, QuestionsPrompt, Target,
    TrustPrompt,
};
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

pub use crate::hosts::local_user;

/// The Authentication method choices, as the dropdown names them.
const AUTH_METHODS: [(AuthMethod, &str); 4] = [
    (AuthMethod::Automatic, "Automatic"),
    (AuthMethod::Password, "Password"),
    (AuthMethod::KeyFile, "Key file"),
    (AuthMethod::Agent, "Agent"),
];

/// An entry of the Host dialog's left column.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Entry {
    /// The saved Host at this index.
    Saved(usize),
    NewHost,
    UnsavedSession,
}

/// The fields as typed, for the entry picked.
#[derive(Clone)]
struct Fields {
    id: String,
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
            id: host.id.clone(),
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
            id: self.id.clone(),
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
    drafts: std::collections::HashMap<Entry, Fields>,
    filter: String,
    focus_row: bool,
    list_focus: Option<Id>,
    /// The OpenSSH config the placeholders come from, read when the dialog opened.
    config: Config,
    /// Why pasted text stayed in Address, since focus last left it.
    pasted: Vec<PasteError>,
    shown_before: bool,
    /// Credential store entries the last save or delete dropped, for the app to remove.
    forgotten: Vec<credentials::Key>,
    /// Why a kept secret may still be in the store.
    notice: Option<String>,
}

/// What the operator did in the Host dialog.
pub enum HostAction {
    Open(Box<OpenRequest>),
    /// A Host was saved or deleted: the list is to be written out.
    Changed,
    Reload,
    Overwrite,
    Dismiss,
}

/// A Session to open, as the dialog's fields describe it, before the OpenSSH config
/// is applied.
pub struct OpenRequest {
    host: Host,
    /// Whether the tab takes the display name: a saved Host or a named New Host.
    named: bool,
    /// The saved Host's id, when it remembers its secret.
    remember: Option<String>,
}

/// A Session to open, once the OpenSSH config is applied to its request.
pub struct Plan {
    /// Where and how to connect, or why the Session cannot.
    pub resolved: Result<Resolved, NeedsProxy>,
    /// The Initial remote folder; empty for the home directory.
    pub folder: String,
    /// The Host's Authentication method.
    pub auth: AuthMethod,
    /// The saved Host's id, when it remembers its secret.
    pub remember: Option<String>,
    pub details: Details,
}

impl OpenRequest {
    /// Applies `config`: a non-empty field wins, then the config, then the defaults.
    pub fn plan(self, config: &Config) -> Plan {
        let host = self.host;
        let fields = HostFields {
            address: host.address.clone(),
            port: host.port,
            account: host.account.clone(),
            key_file: host.key_file.clone(),
        };
        let resolved = config.resolve(&fields, &local_user());
        // The tab names the address as typed, with the account and port it resolved to.
        let shown = match &resolved {
            Ok(r) => Target {
                address: host.address.clone(),
                ..r.target.clone()
            },
            Err(_) => Target {
                address: host.address.clone(),
                port: host.port.unwrap_or(22),
                account: if host.account.is_empty() {
                    local_user()
                } else {
                    host.account.clone()
                },
            },
        };
        let unsaved = session::label(&shown);
        let (label, tooltip) = if self.named && !host.display_name.is_empty() {
            (host.display_name.clone(), Some(unsaved))
        } else {
            (unsaved, None)
        };
        Plan {
            resolved,
            folder: host.initial_folder.clone(),
            auth: host.auth,
            remember: self.remember,
            details: Details {
                label,
                tooltip,
                readonly: host.readonly,
                local_folder: host.local_folder,
                note: config.note(),
                dismissible: None,
            },
        }
    }
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
    /// Placeholders come from `config`: an empty one when the config is not used.
    pub fn new(hosts: &[Host], config: Config) -> HostDialog {
        let mut dialog = HostDialog {
            drafts: Default::default(),
            filter: String::new(),
            focus_row: true,
            list_focus: None,
            entry: Entry::NewHost,
            fields: Fields::of(&Host::default()),
            config,
            pasted: Vec::new(),
            shown_before: false,
            forgotten: Vec::new(),
            notice: None,
        };
        if !hosts.is_empty() {
            dialog.pick(Entry::Saved(0), hosts);
        }
        dialog
    }

    /// The credential store entries that the saves and deletes since the last call
    /// dropped: the app removes them, off the frame thread.
    pub fn take_forgotten(&mut self) -> Vec<credentials::Key> {
        std::mem::take(&mut self.forgotten)
    }

    /// Shows `text` under the buttons, such as "did not remove the saved secret".
    pub fn set_notice(&mut self, text: impl Into<String>) {
        self.notice = Some(text.into());
    }

    /// Notes the entries that saving `old` as `new` (or deleting it) drops.
    fn forget(&mut self, old: &Host, new: Option<&Host>) {
        let kinds: Vec<Kind> = credentials::forgotten(old, new);
        if old.id.is_empty() {
            return;
        }
        self.forgotten.extend(
            kinds
                .into_iter()
                .map(|kind| credentials::Key::new(old.id.clone(), kind)),
        );
    }

    /// Restores the draft for this entry, or fills the form from the saved host.
    fn pick(&mut self, entry: Entry, hosts: &[Host]) {
        self.entry = entry;
        self.pasted.clear();
        self.fields = self.drafts.remove(&entry).unwrap_or_else(|| match entry {
            Entry::Saved(i) => Fields::of(&hosts[i]),
            Entry::NewHost => Fields::of(&Host::default()),
            Entry::UnsavedSession => Fields {
                remember: false,
                ..Fields::of(&Host::default())
            },
        });
    }

    fn switch(&mut self, entry: Entry, hosts: &[Host]) {
        if self.entry != entry {
            self.drafts.insert(self.entry, self.fields.clone());
            self.pick(entry, hosts);
        }
        self.focus_row = true;
    }

    /// Every entry, in the left column's order.
    fn entries(&self, hosts: &[Host]) -> Vec<Entry> {
        let filter = self.filter.to_lowercase();
        (0..hosts.len())
            .filter(|&i| {
                entry_name(&hosts[i]).to_lowercase().contains(&filter)
                    || hosts[i].address.to_lowercase().contains(&filter)
            })
            .map(Entry::Saved)
            .chain([Entry::NewHost, Entry::UnsavedSession])
            .collect()
    }

    /// Picks the entry `step` away from the one picked, if there is one.
    fn step(&mut self, step: isize, hosts: &[Host]) {
        let entries = self.entries(hosts);
        let at = entries.iter().position(|e| *e == self.entry).unwrap_or(0);
        if let Some(next) = at.checked_add_signed(step).and_then(|i| entries.get(i)) {
            self.switch(*next, hosts);
        }
    }

    /// Draws the dialog for one frame. Save and Delete change `hosts` and say so with
    /// [`HostAction::Changed`].
    pub fn show(&mut self, ctx: &Context, hosts: &mut Vec<Host>) -> Option<HostAction> {
        self.show_with_save_error(ctx, hosts, None, false)
    }

    pub(crate) fn show_with_save_error(
        &mut self,
        ctx: &Context,
        hosts: &mut Vec<Host>,
        save_error: Option<&str>,
        conflict: bool,
    ) -> Option<HostAction> {
        let id = Id::NULL.with("host-dialog");
        let mut open_selected = false;
        if !egui::Popup::is_any_open(ctx)
            && self
                .list_focus
                .is_some_and(|row| ctx.memory(|m| m.has_focus(row)))
        {
            open_selected = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter));
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
        let filter_id = id.with("filter");
        if ctx.memory(|m| m.has_focus(filter_id))
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter))
            && let Some(entry) = self.entries(hosts).first()
        {
            self.switch(*entry, hosts);
        }
        let port = hosts::port_value(&self.fields.port);
        let filled = port.is_ok() && !self.fields.address.trim().is_empty();
        let mut clicked = None;
        let mut picked = None;
        let shown = modal(ctx, id, "Hosts", |ui| {
            let t = ui.tokens();
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(t.size.host_list);
                    ui.add(
                        widgets::text_field(&mut self.filter)
                            .id(id.with("filter"))
                            .hint("Filter hosts")
                            .width(t.size.host_list),
                    )
                    .accessible_name("Filter hosts");
                    egui::ScrollArea::vertical()
                        .id_salt("host-list")
                        .max_height((ctx.content_rect().height() - 130.0).max(80.0))
                        .show(ui, |ui| {
                            widgets::rows(ui, |ui| {
                                for entry in self.entries(hosts) {
                                    let name = match entry {
                                        Entry::Saved(i) => entry_name(&hosts[i]),
                                        Entry::NewHost => "New Host",
                                        Entry::UnsavedSession => "Unsaved Session",
                                    };
                                    let selection = if entry == self.entry {
                                        widgets::Highlight::Live
                                    } else {
                                        widgets::Highlight::None
                                    };
                                    let row = widgets::list_row(name)
                                        .selection(selection)
                                        .width(t.size.host_list)
                                        .show(ui);
                                    if entry == self.entry {
                                        self.list_focus = Some(row.response.id);
                                    }
                                    if entry == self.entry && self.focus_row {
                                        row.response.scroll_to_me(None);
                                        row.response.request_focus();
                                        self.focus_row = false;
                                    }
                                    if row.response.clicked() {
                                        picked = Some(entry);
                                    }
                                }
                            })
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
                    if conflict {
                        if ui.add(widgets::button("Keep my changes")).clicked() {
                            clicked = Some(Clicked::Overwrite);
                        }
                        if ui.add(widgets::button("Use saved Hosts")).clicked() {
                            clicked = Some(Clicked::Reload);
                        }
                    }
                    if widgets::retry_save(ui, save_error) {
                        clicked = Some(Clicked::Retry);
                    }
                    if let Some(notice) = &self.notice {
                        ui.add(widgets::warning_message(notice.as_str()));
                    }
                });
            });
        });
        // The click that opened the dialog is not a click on its backdrop.
        let first = !std::mem::replace(&mut self.shown_before, true);
        let shown = shown?;
        if let Some(entry) = picked
            && !(open_selected && entry == self.entry)
        {
            self.switch(entry, hosts);
            return None;
        }
        let enter = open_selected || ctx.input(|i| i.key_pressed(Key::Enter));
        match clicked {
            Some(Clicked::Open) => return Some(HostAction::Open(Box::new(self.request(hosts)))),
            Some(Clicked::Reload) => return Some(HostAction::Reload),
            Some(Clicked::Overwrite) => return Some(HostAction::Overwrite),
            Some(Clicked::Retry) => return Some(HostAction::Changed),
            Some(Clicked::Save) => {
                if self.fields.id.is_empty() {
                    self.fields.id = hosts::new_id();
                }
                let host = self.fields.host();
                match self.entry {
                    Entry::Saved(i) => {
                        let old = std::mem::replace(&mut hosts[i], host.clone());
                        self.forget(&old, Some(&host));
                    }
                    _ => {
                        hosts.push(host);
                        self.entry = Entry::Saved(hosts.len() - 1);
                    }
                }
                return Some(HostAction::Changed);
            }
            Some(Clicked::Delete) => {
                if let Entry::Saved(i) = self.entry {
                    let drafts = std::mem::take(&mut self.drafts);
                    for (entry, fields) in drafts {
                        let key = match entry {
                            Entry::Saved(j) if j == i => continue,
                            Entry::Saved(j) if j > i => Entry::Saved(j - 1),
                            _ => entry,
                        };
                        self.drafts.insert(key, fields);
                    }
                    let old = hosts.remove(i);
                    self.forget(&old, None);
                    let next = match hosts.len() {
                        0 => Entry::NewHost,
                        n => Entry::Saved(i.min(n - 1)),
                    };
                    self.pick(next, hosts);
                }
                return Some(HostAction::Changed);
            }
            None if enter && filled => {
                return Some(HostAction::Open(Box::new(self.request(hosts))));
            }
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
        let matched = self.config.lookup(&self.fields.address);
        let f = &mut self.fields;
        let pasted = &mut self.pasted;
        let entry = self.entry;
        let message = |ui: &mut egui::Ui, text: &str| {
            ui.label("");
            ui.add(widgets::invalid_message(text));
            ui.end_row();
        };
        let hint = |ui: &mut egui::Ui, text: &str| {
            ui.label("");
            ui.add(widgets::form_hint(text));
            ui.end_row();
        };
        let from_config = |value: &str| format!("{value} (from {CONFIG_NAME})");
        let overrides = |value: &str| format!("Overrides {value} from {CONFIG_NAME}");
        let port_hint = matched
            .port
            .map_or("22".to_owned(), |p| from_config(&p.to_string()));
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
                    // Wide enough for a placeholder that names the config.
                    let width = if f.port.is_empty() && matched.port.is_some() {
                        t.size.field * 0.75
                    } else {
                        t.size.field / 4.0
                    };
                    ui.add(
                        widgets::number_field(&mut f.port)
                            .hint(port_hint)
                            .width(width)
                            .invalid(bad_port),
                    )
                    .accessible_name("Port");
                });
            });
            for error in pasted.iter() {
                message(ui, &error.to_string());
            }
            match (hosts::port_value(&f.port), matched.port) {
                (Err(_), _) => message(ui, "Port must be 1 to 65535"),
                (Ok(Some(port)), Some(config)) if port != config => {
                    hint(ui, &overrides(&config.to_string()));
                }
                _ => {}
            }
            let account_hint = matched.user.as_deref().map_or_else(local_user, from_config);
            widgets::form_row(ui, "Account", |ui| {
                ui.add(widgets::text_field(&mut f.account).hint(account_hint))
                    .accessible_name("Account");
            });
            let account = f.account.trim();
            if let Some(user) = &matched.user
                && !account.is_empty()
                && account != user
            {
                hint(ui, &overrides(user));
            }
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
            let key_hint = matched
                .identity_files
                .first()
                .map(|k| from_config(k))
                .unwrap_or_default();
            widgets::form_row(ui, "Key file", |ui| {
                ui.add(widgets::text_field(&mut f.key_file).hint(key_hint))
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

    /// The Session the fields describe. It remembers its secret only for a saved Host
    /// with Remember on.
    fn request(&self, hosts: &[Host]) -> OpenRequest {
        let remember = match self.entry {
            Entry::Saved(i) if self.fields.remember && hosts[i].remember => {
                Some(hosts[i].id.clone()).filter(|id| !id.is_empty())
            }
            _ => None,
        };
        OpenRequest {
            host: self.fields.host(),
            named: self.entry != Entry::UnsavedSession,
            remember,
        }
    }
}

/// A button of the Host dialog that was clicked.
enum Clicked {
    Reload,
    Overwrite,
    Retry,
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
    /// An event already taken from `opening`, handled before the next.
    first: Option<Event>,
    prompt: Option<Prompt>,
    /// What is typed into the secret dialog: one entry per field.
    answers: Vec<String>,
}

enum Prompt {
    Trust(TrustPrompt),
    Password(PasswordPrompt),
    Passphrase(PassphrasePrompt),
    Questions(QuestionsPrompt),
    Failed(Failure),
}

/// One field of the secret dialog.
struct Field {
    label: String,
    hidden: bool,
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
            first: None,
            prompt: None,
            answers: Vec::new(),
        }
    }

    /// An opening, such as a reconnect, whose `first` event needs the operator: it was
    /// taken from `opening` before the operator was asked.
    pub fn resume(id: Id, details: Details, opening: ssh::Opening, first: Event) -> Self {
        Pending {
            first: Some(first),
            ..Pending::new(id, details, opening)
        }
    }

    pub(crate) fn progress(&mut self, ui: &mut egui::Ui) -> bool {
        if self.is_modal() {
            return false;
        }
        ui.push_id(self.id, |ui| {
            ui.horizontal(|ui| {
                ui.add(widgets::spinner());
                ui.label(format!("{} · {}", self.details.label, self.opening.stage()));
                ui.add(widgets::button("Cancel")).clicked()
            })
            .inner
        })
        .inner
    }

    /// Whether it is showing a dialog, which takes the input from the window.
    pub fn is_modal(&self) -> bool {
        self.prompt.is_some() || self.first.is_some()
    }

    /// Takes in what arrived and draws the prompt, if any, for one frame.
    pub fn show(&mut self, ctx: &Context) -> Option<Outcome> {
        if self.prompt.is_none() {
            let event = match self.first.take() {
                Some(event) => event,
                None => self.opening.try_next()?,
            };
            match event {
                Event::TrustHostKey(prompt) => self.prompt = Some(Prompt::Trust(prompt)),
                Event::Password(prompt) => {
                    self.answers = vec![String::new()];
                    self.prompt = Some(Prompt::Password(prompt));
                }
                Event::Passphrase(prompt) => {
                    self.answers = vec![String::new()];
                    self.prompt = Some(Prompt::Passphrase(prompt));
                }
                Event::Questions(prompt) => {
                    self.answers = vec![String::new(); prompt.questions.len()];
                    self.prompt = Some(Prompt::Questions(prompt));
                }
                Event::Opened(opened) => return Some(Outcome::Opened(opened)),
                Event::Failed(Failure::Cancelled) => return Some(Outcome::Ended),
                Event::Failed(failure) => self.prompt = Some(Prompt::Failed(failure)),
            }
        }
        match self.prompt.take()? {
            Prompt::Trust(prompt) => self.show_trust(ctx, prompt),
            Prompt::Password(prompt) => self.show_password(ctx, prompt),
            Prompt::Passphrase(prompt) => self.show_passphrase(ctx, prompt),
            Prompt::Questions(prompt) => self.show_questions(ctx, prompt),
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
        let intro = vec![format!("Password for {}", prompt.account)];
        let saved = prompt.saved.line(Kind::Password);
        let rejected = saved
            .as_deref()
            .or(prompt.rejected.then_some("The password was rejected."));
        let fields = [Field {
            label: "Password".into(),
            hidden: true,
        }];
        match self.secret(ctx, "password", "Password", &intro, rejected, &fields) {
            Some(true) => prompt.enter(self.answers.remove(0)),
            Some(false) => prompt.cancel(),
            None => self.prompt = Some(Prompt::Password(prompt)),
        }
        None
    }

    fn show_passphrase(&mut self, ctx: &Context, prompt: PassphrasePrompt) -> Option<Outcome> {
        let intro = vec![format!("Passphrase for the key {}", prompt.key.display())];
        let saved = prompt.saved.line(Kind::Passphrase);
        let rejected = saved.as_deref().or(prompt
            .rejected
            .then_some("The key passphrase was rejected."));
        let fields = [Field {
            label: "Passphrase".into(),
            hidden: true,
        }];
        match self.secret(
            ctx,
            "passphrase",
            "Key passphrase",
            &intro,
            rejected,
            &fields,
        ) {
            Some(true) => prompt.enter(self.answers.remove(0)),
            Some(false) => prompt.cancel(),
            None => self.prompt = Some(Prompt::Passphrase(prompt)),
        }
        None
    }

    fn show_questions(&mut self, ctx: &Context, prompt: QuestionsPrompt) -> Option<Outcome> {
        let mut intro = vec![format!("The host asks {}:", prompt.account)];
        intro.extend(
            [&prompt.name, &prompt.instructions]
                .into_iter()
                .filter(|t| !t.trim().is_empty())
                .map(|t| t.trim().to_owned()),
        );
        let rejected = prompt.rejected.then_some("The answers were rejected.");
        let fields: Vec<Field> = prompt
            .questions
            .iter()
            .map(|q| Field {
                label: q.prompt.trim().trim_end_matches(':').trim_end().to_owned(),
                hidden: !q.echo,
            })
            .collect();
        match self.secret(ctx, "questions", "Sign in", &intro, rejected, &fields) {
            Some(true) => prompt.answer(std::mem::take(&mut self.answers)),
            Some(false) => prompt.cancel(),
            None => self.prompt = Some(Prompt::Questions(prompt)),
        }
        None
    }

    /// The secret dialog for one frame: `intro`, the rejection line, and `fields` typed
    /// into `self.answers`. `Some(true)` on Connect or Enter, `Some(false)` on Cancel,
    /// Escape or the close control. Cancelling clears what was typed.
    fn secret(
        &mut self,
        ctx: &Context,
        name: &str,
        title: &str,
        intro: &[String],
        rejected: Option<&str>,
        fields: &[Field],
    ) -> Option<bool> {
        let mut answer = None;
        let id = self.id.with(name);
        self.answers.resize(fields.len(), String::new());
        let answers = &mut self.answers;
        let shown = modal(ctx, id, title, |ui| {
            for line in intro {
                paragraph(ui, line);
            }
            if let Some(rejected) = rejected {
                ui.add(widgets::invalid_message(rejected));
            }
            let mut first = None;
            for (i, (field, text)) in fields.iter().zip(answers.iter_mut()).enumerate() {
                if fields.len() > 1 {
                    paragraph(ui, &field.label);
                }
                let mut edit = widgets::text_field(text).id(id.with("field").with(i));
                if field.hidden {
                    edit = edit.password();
                }
                let response = ui.add(edit).accessible_name(field.label.as_str());
                first.get_or_insert(response);
            }
            if let Some(first) = first
                && !ui.memory(|m| m.focused().is_some())
            {
                first.request_focus();
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
        if answer == Some(false) {
            self.answers.clear();
        }
        answer
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
