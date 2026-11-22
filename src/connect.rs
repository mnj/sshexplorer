//! Opening a Session from the window: the Host dialog, and the prompts and refusals that
//! come up while the connection opens.
//!
//! The Host dialog here is the tracer's minimal one (issue #22): Address with its Port,
//! and Account. The saved-Host column, the other fields and OpenSSH config come later.
//! Each Session being opened is a [`Pending`], which polls its [`ssh::Opening`] once a
//! frame and never waits on it.

use egui::{Context, Id, Key, LayerId, Order};

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

/// The Port field's value: `Ok(None)` when empty, `Err` unless 1 to 65535.
fn port_value(text: &str) -> Result<Option<u16>, ()> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    match text.parse::<u16>() {
        Ok(0) | Err(_) => Err(()),
        Ok(port) => Ok(Some(port)),
    }
}

/// The local OS user name: the account when the Host leaves it empty.
fn local_user() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default()
}

/// The Host dialog's fields while it is open.
#[derive(Default)]
pub struct HostDialog {
    address: String,
    port: String,
    account: String,
    shown_before: bool,
}

/// What the operator did in the Host dialog.
pub enum HostAction {
    Open(Target),
    Dismiss,
}

impl HostDialog {
    /// Draws the dialog for one frame.
    pub fn show(&mut self, ctx: &Context) -> Option<HostAction> {
        let id = Id::NULL.with("host-dialog");
        let address_id = id.with("address");
        let port = port_value(&self.port);
        let can_open = port.is_ok() && !self.address.trim().is_empty();
        let mut action = None;
        let shown = modal(ctx, id, "Hosts", |ui| {
            let t = ui.tokens();
            widgets::form(id.with("form")).show(ui, |ui| {
                widgets::form_row(ui, "Address", |ui| {
                    ui.horizontal(|ui| {
                        let address = ui
                            .add(widgets::text_field(&mut self.address).id(address_id))
                            .accessible_name("Address");
                        if !self.shown_before {
                            address.request_focus();
                        }
                        ui.add(
                            widgets::number_field(&mut self.port)
                                .hint("22")
                                .width(t.size.field / 4.0)
                                .invalid(port.is_err()),
                        )
                        .accessible_name("Port");
                    });
                });
                if port.is_err() {
                    ui.label("");
                    ui.add(widgets::invalid_message("Port must be 1 to 65535"));
                    ui.end_row();
                }
                widgets::form_row(ui, "Account", |ui| {
                    ui.add(widgets::text_field(&mut self.account).hint(local_user()))
                        .accessible_name("Account");
                });
            });
            widgets::button_row(ui, |ui| {
                let open = ui.add_enabled(can_open, widgets::button("Open Session").primary());
                if open.clicked() {
                    action = Some(true);
                }
            });
        });
        // The click that opened the dialog is not a click on its backdrop.
        let first = !std::mem::replace(&mut self.shown_before, true);
        let shown = shown?;
        let enter = ctx.input(|i| i.key_pressed(Key::Enter));
        if action == Some(true) || (enter && can_open) {
            return Some(HostAction::Open(self.target()));
        }
        let escape = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape));
        let backdrop = !first && clicked_outside(ctx, shown.response.rect);
        if shown.close_clicked || escape || backdrop {
            return Some(HostAction::Dismiss);
        }
        None
    }

    fn target(&self) -> Target {
        let account = self.account.trim();
        Target {
            address: self.address.trim().to_string(),
            port: port_value(&self.port).ok().flatten().unwrap_or(22),
            account: if account.is_empty() {
                local_user()
            } else {
                account.to_string()
            },
        }
    }
}

/// A Session being opened, and whatever it is asking the operator.
pub struct Pending {
    id: Id,
    pub target: Target,
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
    pub fn new(id: Id, target: Target, opening: ssh::Opening) -> Self {
        Pending {
            id,
            target,
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
