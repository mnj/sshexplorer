//! Settings → Keys, the binding editor: every action of the key map, by area, with its
//! key chip, and a Reset on each row whose binding is not the default.
//!
//! A click on a chip records: the next key pressed, with its modifiers, becomes the
//! action's binding at once, unless another action has it, which says so and keeps
//! recording. Escape cancels and is never bound. While recording, the editor takes every
//! key before anything else sees it, so a combination that is bound elsewhere (Ctrl+,
//! included) is recorded rather than run.
//!
//! The table is drawn from the registry in [`crate::keymap`]; an area with no actions is
//! left out.

use egui::{Event, Key, Modifiers};

use crate::keymap::{Action, Area, Binding, KeyMap};
use crate::widgets;

/// What the chip of the action being recorded shows.
const RECORDING: &str = "Press keys…";
const HINT: &str = "Press the new keys. Escape cancels.";

/// The binding editor's state across frames.
#[derive(Clone, Debug, Default)]
pub struct BindingEditor {
    /// The action whose chip is recording.
    recording: Option<Action>,
    /// The last combination pressed while recording that another action has.
    taken: Option<(Binding, Action)>,
}

impl BindingEditor {
    /// Stops recording, leaving the binding as it was.
    pub fn stop(&mut self) {
        self.recording = None;
        self.taken = None;
    }

    pub fn is_recording(&self) -> bool {
        self.recording.is_some()
    }

    /// While recording, takes this frame's keys, before anything else reads them, and
    /// binds the first one pressed. Call it once a frame, first.
    pub fn record(&mut self, ctx: &egui::Context, keymap: &mut KeyMap) {
        let Some(action) = self.recording else {
            return;
        };
        let pressed = ctx.input_mut(|i| {
            let mut first = None;
            i.events.retain(|event| match event {
                Event::Key {
                    key,
                    physical_key,
                    pressed,
                    modifiers,
                    ..
                } => {
                    if *pressed && first.is_none() {
                        first = Some((*key, *physical_key, *modifiers));
                    }
                    false
                }
                Event::Text(_) | Event::Copy | Event::Cut | Event::Paste(_) => false,
                _ => true,
            });
            first
        });
        let Some((key, physical, modifiers)) = pressed else {
            return;
        };
        if key == Key::Escape {
            self.stop();
            return;
        }
        let binding = Binding::pressed(modifiers, key);
        match holder(keymap, binding, physical, modifiers) {
            Some(other) if other != action => self.taken = Some((binding, other)),
            _ => {
                keymap.set(action, binding);
                self.stop();
            }
        }
    }

    /// Draws the table of bindings.
    pub fn ui(&mut self, ui: &mut egui::Ui, keymap: &mut KeyMap) {
        let mac = ui.ctx().os() == egui::os::OperatingSystem::Mac;
        for area in Area::ALL {
            let actions: Vec<Action> = Action::ALL
                .into_iter()
                .filter(|a| a.area() == area)
                .collect();
            if actions.is_empty() {
                continue;
            }
            ui.add(widgets::form_group(area.name()));
            widgets::form(("keys", area.name())).show(ui, |ui| {
                for action in actions {
                    self.row(ui, keymap, action, mac);
                }
            });
        }
    }

    fn row(&mut self, ui: &mut egui::Ui, keymap: &mut KeyMap, action: Action, mac: bool) {
        let recording = self.recording == Some(action);
        ui.add(widgets::form_name(action.name()));
        let chip = if recording {
            RECORDING.to_owned()
        } else {
            keymap
                .binding(action)
                .map_or("Unbound".to_owned(), |b| b.label(mac))
        };
        let clicked = ui
            .add(widgets::key_chip_button(chip).recording(recording))
            .clicked();
        if clicked {
            self.stop();
            if !recording {
                self.recording = Some(action);
            }
        }
        if keymap.is_changed(action) {
            if ui.add(widgets::button("Reset").flat()).clicked() {
                keymap.reset(action);
                if recording {
                    self.stop();
                }
            }
        } else {
            ui.label("");
        }
        ui.end_row();
        if self.recording == Some(action) {
            ui.label("");
            ui.vertical(|ui| {
                ui.add(widgets::form_hint(HINT));
                if let Some((binding, other)) = self.taken {
                    let message = format!(
                        "{} is already bound to “{}”. Change that one first.",
                        binding.label(mac),
                        other.name()
                    );
                    ui.add(widgets::warning_message(message));
                }
            });
            ui.end_row();
        }
    }
}

/// The action a key press already triggers: by the key it typed or by the key pressed,
/// as [`KeyMap::take`] matches them.
fn holder(
    keymap: &KeyMap,
    binding: Binding,
    physical: Option<Key>,
    modifiers: Modifiers,
) -> Option<Action> {
    keymap.bound_to(binding).or_else(|| {
        let physical = physical.filter(|p| *p != binding.key)?;
        keymap.bound_to(Binding::pressed(modifiers, physical))
    })
}
