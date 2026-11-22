//! The key map registry: every named action the keyboard can trigger, grouped by area,
//! with its default binding and the binding in force.
//!
//! A ticket that adds a key adds its [`Action`] here: a variant, its [`Area`], its name
//! as the binding editor shows it, and its default. Code asks the map whether an action
//! was pressed with [`KeyMap::take`], which consumes the key so nothing else sees it.
//!
//! Bindings say Ctrl as egui's `command`, which is Cmd on macOS and Ctrl elsewhere.
//! Modifiers match exactly: Ctrl+= is not Ctrl+Shift+=. A key matches by what it types or
//! by the key pressed, so Ctrl+Shift+= is found on a layout where Shift+= types "+".
//! Escape is never bound: it always cancels.

use std::collections::BTreeMap;

use egui::{Event, InputState, Key, ModifierNames, Modifiers};

/// Where an action belongs, as the binding editor groups them.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Area {
    Browsing,
    Search,
    Sessions,
    Preview,
    Appearance,
}

impl Area {
    /// In the binding editor's order.
    pub const ALL: [Area; 5] = [
        Area::Browsing,
        Area::Search,
        Area::Sessions,
        Area::Preview,
        Area::Appearance,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Area::Browsing => "Browsing",
            Area::Search => "Search",
            Area::Sessions => "Sessions",
            Area::Preview => "Preview",
            Area::Appearance => "Appearance",
        }
    }
}

/// A named action the keyboard can trigger.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Action {
    /// Opens the Settings dialog; works at Launch too.
    Settings,
    UiScaleUp,
    UiScaleDown,
    UiScaleReset,
}

impl Action {
    /// Every action, in the binding editor's order within its area.
    pub const ALL: [Action; 4] = [
        Action::UiScaleUp,
        Action::UiScaleDown,
        Action::UiScaleReset,
        Action::Settings,
    ];

    pub fn area(self) -> Area {
        match self {
            Action::Settings | Action::UiScaleUp | Action::UiScaleDown | Action::UiScaleReset => {
                Area::Appearance
            }
        }
    }

    /// The action as the binding editor names it.
    pub fn name(self) -> &'static str {
        match self {
            Action::Settings => "Settings",
            Action::UiScaleUp => "UI scale up",
            Action::UiScaleDown => "UI scale down",
            Action::UiScaleReset => "UI scale reset",
        }
    }

    /// The binding a new install has; `None` for an action that starts unbound.
    pub fn default_binding(self) -> Option<Binding> {
        let cmd = Modifiers::COMMAND;
        let shift_cmd = Modifiers {
            shift: true,
            ..Modifiers::COMMAND
        };
        Some(match self {
            Action::Settings => Binding::new(cmd, Key::Comma),
            Action::UiScaleUp => Binding::new(shift_cmd, Key::Equals),
            Action::UiScaleDown => Binding::new(shift_cmd, Key::Minus),
            Action::UiScaleReset => Binding::new(shift_cmd, Key::Num0),
        })
    }
}

/// A key with its modifiers. Ctrl is written as `Modifiers::COMMAND`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Binding {
    pub modifiers: Modifiers,
    pub key: Key,
}

impl Binding {
    pub const fn new(modifiers: Modifiers, key: Key) -> Binding {
        Binding { modifiers, key }
    }

    /// How a key chip shows it: "Ctrl+Shift+=", or "⇧⌘=" on macOS.
    pub fn label(self, mac: bool) -> String {
        let names = if mac {
            ModifierNames::SYMBOLS
        } else {
            ModifierNames::NAMES
        };
        let mut label = names.format(&self.modifiers, mac);
        if !label.is_empty() {
            label += names.concat;
        }
        label + self.key.symbol_or_name()
    }

    /// Whether a key event with `key`, `physical` and `modifiers` is this binding.
    fn matches(self, key: Key, physical: Option<Key>, modifiers: Modifiers) -> bool {
        let b = self.modifiers;
        (key == self.key || physical == Some(self.key))
            && modifiers.shift == b.shift
            && modifiers.alt == b.alt
            && modifiers.cmd_ctrl_matches(b)
    }
}

/// The bindings in force: each action's default unless it was rebound.
#[derive(Clone, Debug, Default)]
pub struct KeyMap {
    /// Actions whose binding is not the default; `None` is unbound.
    changed: BTreeMap<Action, Option<Binding>>,
}

impl KeyMap {
    /// The binding `action` answers to, if any.
    pub fn binding(&self, action: Action) -> Option<Binding> {
        match self.changed.get(&action) {
            Some(binding) => *binding,
            None => action.default_binding(),
        }
    }

    /// Binds `action` to `binding`. The caller has checked [`KeyMap::bound_to`].
    pub fn set(&mut self, action: Action, binding: Binding) {
        if action.default_binding() == Some(binding) {
            self.changed.remove(&action);
        } else {
            self.changed.insert(action, Some(binding));
        }
    }

    /// Back to the default binding.
    pub fn reset(&mut self, action: Action) {
        self.changed.remove(&action);
    }

    /// Every action back to its default.
    pub fn reset_all(&mut self) {
        self.changed.clear();
    }

    /// Whether `action`'s binding differs from its default.
    pub fn is_changed(&self, action: Action) -> bool {
        self.changed.contains_key(&action)
    }

    /// The action `binding` triggers, if one does.
    pub fn bound_to(&self, binding: Binding) -> Option<Action> {
        Action::ALL
            .into_iter()
            .find(|a| self.binding(*a) == Some(binding))
    }

    /// Whether `action` was pressed this pass; the key press is consumed, so it is taken
    /// once.
    pub fn take(&self, input: &mut InputState, action: Action) -> bool {
        let Some(binding) = self.binding(action) else {
            return false;
        };
        let mut found = false;
        input.events.retain(|event| {
            let hit = matches!(
                event,
                Event::Key { key, physical_key, pressed: true, modifiers, .. }
                    if binding.matches(*key, *physical_key, *modifiers)
            );
            found |= hit;
            !hit
        });
        found
    }
}
