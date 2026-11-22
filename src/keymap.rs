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
use serde::{Deserialize, Deserializer, Serialize, Serializer};

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
    /// Closes the active Session.
    CloseSession,
    // Browsing: the listing's keys, taken while it holds the keyboard.
    MoveUp,
    MoveDown,
    MoveFirst,
    MoveLast,
    PageUp,
    PageDown,
    /// Lists a directory or `..`; on a file, moves to Preview.
    Open,
    /// Lists the parent and selects the directory just left.
    Parent,
    /// Parent's second key, which only Folder view answers to.
    ParentFolder,
    /// Tree view: expands a directory, or moves to its first child.
    Expand,
    /// Tree view: collapses a directory, or moves to the parent.
    Collapse,
    /// Switches the active Session to Tree view.
    TreeView,
    /// Switches the active Session to Folder view.
    FolderView,
    ExtendUp,
    ExtendDown,
    /// Moves the focus without changing the Selection.
    FocusUp,
    FocusDown,
    ToggleSelected,
    SelectAll,
    /// Toggles the "Show hidden entries" setting, for every Session.
    ShowHidden,
    /// Fetches the active Session's Current directory again, and in Tree view the
    /// expanded directories below it.
    Refresh,
    /// Sorts the active Session by the next column. Unbound by default.
    Sort,
    /// Brings the next Session's tab to the front, wrapping round.
    NextSession,
    /// Brings the previous Session's tab to the front, wrapping round.
    PreviousSession,
    /// Opens the Host dialog.
    HostDialog,
    /// Moves the keyboard on round the Session: listing, search field, Preview.
    CycleFocus,
    /// Moves the keyboard back round the Session.
    CycleFocusBack,
    /// Downloads the Selection into the Session's Local folder.
    Download,
    /// Preview's zoom toolbar: the next preset in or out, and back to Fit. Ctrl+wheel
    /// over the content zooms too, and is fixed.
    ZoomIn,
    ZoomOut,
    ZoomFit,
}

impl Action {
    /// Every action, in the binding editor's order within its area.
    pub const ALL: [Action; 36] = [
        Action::MoveUp,
        Action::MoveDown,
        Action::MoveFirst,
        Action::MoveLast,
        Action::PageUp,
        Action::PageDown,
        Action::Open,
        Action::Parent,
        Action::ParentFolder,
        Action::Expand,
        Action::Collapse,
        Action::ExtendUp,
        Action::ExtendDown,
        Action::FocusUp,
        Action::FocusDown,
        Action::ToggleSelected,
        Action::SelectAll,
        Action::ShowHidden,
        Action::Refresh,
        Action::TreeView,
        Action::FolderView,
        Action::Sort,
        Action::Download,
        Action::NextSession,
        Action::PreviousSession,
        Action::CloseSession,
        Action::HostDialog,
        Action::CycleFocus,
        Action::CycleFocusBack,
        Action::ZoomIn,
        Action::ZoomOut,
        Action::ZoomFit,
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
            Action::CloseSession
            | Action::NextSession
            | Action::PreviousSession
            | Action::HostDialog
            | Action::CycleFocus
            | Action::CycleFocusBack => Area::Sessions,
            Action::ZoomIn | Action::ZoomOut | Action::ZoomFit => Area::Preview,
            Action::MoveUp
            | Action::MoveDown
            | Action::MoveFirst
            | Action::MoveLast
            | Action::PageUp
            | Action::PageDown
            | Action::Open
            | Action::Parent
            | Action::ParentFolder
            | Action::Expand
            | Action::Collapse
            | Action::TreeView
            | Action::FolderView
            | Action::ExtendUp
            | Action::ExtendDown
            | Action::FocusUp
            | Action::FocusDown
            | Action::ToggleSelected
            | Action::SelectAll
            | Action::ShowHidden
            | Action::Refresh
            | Action::Sort
            | Action::Download => Area::Browsing,
        }
    }

    /// The action as the binding editor names it.
    pub fn name(self) -> &'static str {
        match self {
            Action::Settings => "Settings",
            Action::UiScaleUp => "UI scale up",
            Action::UiScaleDown => "UI scale down",
            Action::UiScaleReset => "UI scale reset",
            Action::CloseSession => "Close Session",
            Action::MoveUp => "Move up",
            Action::MoveDown => "Move down",
            Action::MoveFirst => "Move to first",
            Action::MoveLast => "Move to last",
            Action::PageUp => "Page up",
            Action::PageDown => "Page down",
            Action::Open => "Open",
            Action::Parent => "Parent",
            Action::ParentFolder => "Parent (Folder view)",
            Action::Expand => "Tree: expand / first child",
            Action::Collapse => "Tree: collapse / parent",
            Action::TreeView => "Tree view",
            Action::FolderView => "Folder view",
            Action::ExtendUp => "Extend Selection up",
            Action::ExtendDown => "Extend Selection down",
            Action::FocusUp => "Move focus up",
            Action::FocusDown => "Move focus down",
            Action::ToggleSelected => "Toggle Focused entry in Selection",
            Action::SelectAll => "Select all",
            Action::ShowHidden => "Show hidden entries",
            Action::Refresh => "Refresh",
            Action::Sort => "Sort",
            Action::NextSession => "Next Session",
            Action::PreviousSession => "Previous Session",
            Action::HostDialog => "Host dialog",
            Action::CycleFocus => "Cycle focus",
            Action::CycleFocusBack => "Cycle focus back",
            Action::Download => "Download",
            Action::ZoomIn => "Zoom in (or Ctrl+wheel)",
            Action::ZoomOut => "Zoom out (or Ctrl+wheel)",
            Action::ZoomFit => "Zoom to Fit",
        }
    }

    /// The binding a new install has; `None` for an action that starts unbound.
    pub fn default_binding(self) -> Option<Binding> {
        let cmd = Modifiers::COMMAND;
        let shift_cmd = Modifiers {
            shift: true,
            ..Modifiers::COMMAND
        };
        let none = Modifiers::NONE;
        let shift = Modifiers::SHIFT;
        Some(match self {
            Action::Sort => return None,
            Action::MoveUp => Binding::new(none, Key::ArrowUp),
            Action::MoveDown => Binding::new(none, Key::ArrowDown),
            Action::MoveFirst => Binding::new(none, Key::Home),
            Action::MoveLast => Binding::new(none, Key::End),
            Action::PageUp => Binding::new(none, Key::PageUp),
            Action::PageDown => Binding::new(none, Key::PageDown),
            Action::Open => Binding::new(none, Key::Enter),
            Action::Parent => Binding::new(none, Key::Backspace),
            Action::ParentFolder => Binding::new(Modifiers::ALT, Key::ArrowUp),
            Action::Expand => Binding::new(none, Key::ArrowRight),
            Action::Collapse => Binding::new(none, Key::ArrowLeft),
            Action::TreeView => Binding::new(cmd, Key::Num1),
            Action::FolderView => Binding::new(cmd, Key::Num2),
            Action::ExtendUp => Binding::new(shift, Key::ArrowUp),
            Action::ExtendDown => Binding::new(shift, Key::ArrowDown),
            Action::FocusUp => Binding::new(cmd, Key::ArrowUp),
            Action::FocusDown => Binding::new(cmd, Key::ArrowDown),
            Action::ToggleSelected => Binding::new(cmd, Key::Space),
            Action::SelectAll => Binding::new(cmd, Key::A),
            Action::ShowHidden => Binding::new(cmd, Key::H),
            Action::Refresh => Binding::new(none, Key::F5),
            Action::Settings => Binding::new(cmd, Key::Comma),
            Action::UiScaleUp => Binding::new(shift_cmd, Key::Equals),
            Action::UiScaleDown => Binding::new(shift_cmd, Key::Minus),
            Action::UiScaleReset => Binding::new(shift_cmd, Key::Num0),
            Action::CloseSession => Binding::new(cmd, Key::W),
            Action::NextSession => Binding::new(cmd, Key::Tab),
            Action::PreviousSession => Binding::new(shift_cmd, Key::Tab),
            Action::HostDialog => Binding::new(cmd, Key::O),
            Action::CycleFocus => Binding::new(none, Key::Tab),
            Action::CycleFocusBack => Binding::new(shift, Key::Tab),
            Action::Download => Binding::new(cmd, Key::S),
            Action::ZoomIn => Binding::new(cmd, Key::Equals),
            Action::ZoomOut => Binding::new(cmd, Key::Minus),
            Action::ZoomFit => Binding::new(cmd, Key::Num0),
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

    /// The binding a key press with `modifiers` records: Ctrl (Cmd on macOS) as
    /// `command`, and on macOS the Control key, which is not Cmd, as `ctrl`.
    pub fn pressed(modifiers: Modifiers, key: Key) -> Binding {
        let m = modifiers;
        Binding::new(
            Modifiers {
                alt: m.alt,
                shift: m.shift,
                command: m.command,
                ctrl: m.ctrl && !m.command,
                mac_cmd: false,
            },
            key,
        )
    }

    /// The binding as the settings file writes it: "Ctrl+Shift+Equals". Ctrl is Cmd on
    /// macOS; "Control" is the macOS Control key.
    fn to_text(self) -> String {
        let m = self.modifiers;
        let mut text = String::new();
        for (on, name) in [
            (m.command, "Ctrl"),
            (m.ctrl, "Control"),
            (m.alt, "Alt"),
            (m.shift, "Shift"),
        ] {
            if on {
                text += name;
                text += "+";
            }
        }
        text + self.key.name()
    }

    /// The binding [`Binding::to_text`] wrote, if `text` is one. Escape is never one.
    fn from_text(text: &str) -> Option<Binding> {
        let mut parts: Vec<&str> = text.split('+').collect();
        let key = parts.pop()?;
        let key = Key::from_name(key).filter(|k| *k != Key::Escape)?;
        let mut m = Modifiers::NONE;
        for name in parts {
            match name {
                "Ctrl" => m.command = true,
                "Control" => m.ctrl = true,
                "Alt" => m.alt = true,
                "Shift" => m.shift = true,
                _ => return None,
            }
        }
        Some(Binding::new(m, key))
    }

    /// How a key chip shows it: "Ctrl+Shift+=", or "⇧⌘=" on macOS; "Shift+Up" for an
    /// arrow.
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
        // The arrows by name: egui's arrow symbols are not in the bundled fonts.
        let key = match self.key {
            Key::ArrowUp | Key::ArrowDown | Key::ArrowLeft | Key::ArrowRight => self.key.name(),
            key => key.symbol_or_name(),
        };
        label + key
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
///
/// In the settings file it is the `[keys]` table of the rebound actions only, each by its
/// variant name: `Settings = "Ctrl+Shift+Comma"`, or `""` for unbound. An entry that names
/// no action, or no binding, is left out, and that action keeps its default.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
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

impl Action {
    /// The name the settings file knows the action by: its variant name.
    fn file_name(self) -> String {
        format!("{self:?}")
    }
}

impl Serialize for KeyMap {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let table: BTreeMap<String, String> = self
            .changed
            .iter()
            .map(|(action, binding)| {
                let text = binding.map_or(String::new(), Binding::to_text);
                (action.file_name(), text)
            })
            .collect();
        table.serialize(s)
    }
}

impl<'de> Deserialize<'de> for KeyMap {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<KeyMap, D::Error> {
        let table = BTreeMap::<String, toml::Value>::deserialize(d)?;
        let mut map = KeyMap::default();
        for (name, value) in table {
            let action = Action::ALL.into_iter().find(|a| a.file_name() == name);
            match (action, value.as_str()) {
                (Some(action), Some("")) if action.default_binding().is_some() => {
                    map.changed.insert(action, None);
                }
                (Some(action), Some(text)) => {
                    if let Some(binding) = Binding::from_text(text) {
                        map.set(action, binding);
                    }
                }
                _ => {}
            }
        }
        Ok(map)
    }
}
