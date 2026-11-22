//! The key map registry: named actions grouped by area, each with a default binding, with
//! Cmd in place of Ctrl on macOS.

use egui::{Event, Key, Modifiers, RawInput};
use sshexplorer::keymap::{Action, Area, Binding, KeyMap};

/// Presses `key` (and, when it differs, the `physical` key) with `modifiers` in one pass
/// and returns what the map takes from it.
fn taken(map: &KeyMap, key: Key, physical: Key, modifiers: Modifiers) -> Vec<Action> {
    let ctx = egui::Context::default();
    let mut input = RawInput::default();
    input.events.push(Event::Key {
        key,
        physical_key: Some(physical),
        pressed: true,
        repeat: false,
        modifiers,
    });
    let mut out = Vec::new();
    let mut output = ctx.run_ui(input, |ui| {
        ui.input_mut(|i| {
            for action in Action::ALL {
                if map.take(i, action) {
                    out.push(action);
                }
            }
        });
    });
    output.textures_delta.clear();
    out
}

/// Ctrl as egui-winit reports it off macOS: `ctrl` and `command` both set.
const CTRL: Modifiers = Modifiers::COMMAND;
const CTRL_SHIFT: Modifiers = Modifiers {
    shift: true,
    ..Modifiers::COMMAND
};

#[test]
fn settings_and_ui_scale_are_registered_under_appearance() {
    let map = KeyMap::default();
    let shift_cmd = Modifiers::COMMAND | Modifiers::SHIFT;
    for (action, key, modifiers) in [
        (Action::Settings, Key::Comma, Modifiers::COMMAND),
        (Action::UiScaleUp, Key::Equals, shift_cmd),
        (Action::UiScaleDown, Key::Minus, shift_cmd),
        (Action::UiScaleReset, Key::Num0, shift_cmd),
    ] {
        assert_eq!(action.area(), Area::Appearance);
        assert_eq!(map.binding(action), Some(Binding::new(modifiers, key)));
        assert_eq!(action.default_binding(), Some(Binding::new(modifiers, key)));
    }
    assert_eq!(Action::Settings.name(), "Settings");
    assert_eq!(Action::UiScaleUp.name(), "UI scale up");
}

#[test]
fn bindings_read_ctrl_elsewhere_and_cmd_on_macos() {
    let b = Action::UiScaleUp.default_binding().unwrap();
    assert_eq!(b.label(false), "Ctrl+Shift+=");
    assert_eq!(b.label(true), "⇧⌘=");
    assert_eq!(
        Action::Settings.default_binding().unwrap().label(false),
        "Ctrl+,"
    );
}

#[test]
fn a_recorded_press_is_ctrl_on_every_os_and_control_stays_control_on_macos() {
    let linux_ctrl = Modifiers::CTRL | Modifiers::COMMAND;
    let mac_cmd = Modifiers::MAC_CMD | Modifiers::COMMAND;
    let ctrl_k = Binding::new(Modifiers::COMMAND, Key::K);
    assert_eq!(Binding::pressed(linux_ctrl, Key::K), ctrl_k);
    assert_eq!(Binding::pressed(mac_cmd, Key::K), ctrl_k);
    assert_eq!(
        Binding::pressed(Modifiers::CTRL, Key::K),
        Binding::new(Modifiers::CTRL, Key::K),
        "the macOS Control key"
    );
    // A recorded default is the default, not a change.
    let mut map = KeyMap::default();
    map.set(Action::Settings, Binding::pressed(linux_ctrl, Key::Comma));
    assert!(!map.is_changed(Action::Settings));
}

#[test]
fn the_areas_are_the_binding_editors_groups_in_order() {
    let names: Vec<&str> = Area::ALL.iter().map(|a| a.name()).collect();
    assert_eq!(
        names,
        ["Browsing", "Search", "Sessions", "Preview", "Appearance"]
    );
}

#[test]
fn no_two_actions_share_a_default_binding() {
    let map = KeyMap::default();
    for a in Action::ALL {
        if let Some(b) = map.binding(a) {
            assert_eq!(map.bound_to(b), Some(a));
        }
    }
}

#[test]
fn a_pressed_binding_is_taken_once_by_its_action() {
    let map = KeyMap::default();
    assert_eq!(
        taken(&map, Key::Comma, Key::Comma, CTRL),
        [Action::Settings]
    );
    assert_eq!(
        taken(&map, Key::Num0, Key::Num0, CTRL_SHIFT),
        [Action::UiScaleReset]
    );
}

#[test]
fn shift_must_match_so_ctrl_equals_is_not_ui_scale() {
    let map = KeyMap::default();
    assert!(taken(&map, Key::Equals, Key::Equals, CTRL).is_empty());
    assert!(taken(&map, Key::Comma, Key::Comma, CTRL_SHIFT).is_empty());
}

#[test]
fn a_shifted_key_matches_by_the_key_pressed_whatever_it_types() {
    // On a US layout Ctrl+Shift+= types "+" and Ctrl+Shift+- types "_".
    let map = KeyMap::default();
    assert_eq!(
        taken(&map, Key::Plus, Key::Equals, CTRL_SHIFT),
        [Action::UiScaleUp]
    );
    assert_eq!(
        taken(&map, Key::Minus, Key::Minus, CTRL_SHIFT),
        [Action::UiScaleDown]
    );
}

#[test]
fn a_rebound_action_answers_to_its_new_keys_and_resets_to_its_default() {
    let mut map = KeyMap::default();
    let f2 = Binding::new(Modifiers::NONE, Key::F2);
    map.set(Action::Settings, f2);
    assert_eq!(map.binding(Action::Settings), Some(f2));
    assert!(map.is_changed(Action::Settings));
    assert_eq!(
        taken(&map, Key::F2, Key::F2, Modifiers::NONE),
        [Action::Settings]
    );
    assert!(taken(&map, Key::Comma, Key::Comma, CTRL).is_empty());
    map.reset(Action::Settings);
    assert!(!map.is_changed(Action::Settings));
    assert_eq!(
        taken(&map, Key::Comma, Key::Comma, CTRL),
        [Action::Settings]
    );
}

#[test]
fn the_browsing_keys_are_registered_under_browsing() {
    let none = Modifiers::NONE;
    let shift = Modifiers::SHIFT;
    let alt = Modifiers::ALT;
    let expected = [
        (Action::MoveUp, "Move up", Some((none, Key::ArrowUp))),
        (Action::MoveDown, "Move down", Some((none, Key::ArrowDown))),
        (Action::MoveFirst, "Move to first", Some((none, Key::Home))),
        (Action::MoveLast, "Move to last", Some((none, Key::End))),
        (Action::PageUp, "Page up", Some((none, Key::PageUp))),
        (Action::PageDown, "Page down", Some((none, Key::PageDown))),
        (Action::Open, "Open", Some((none, Key::Enter))),
        (Action::Parent, "Parent", Some((none, Key::Backspace))),
        (
            Action::ParentFolder,
            "Parent (Folder view)",
            Some((alt, Key::ArrowUp)),
        ),
        (
            Action::ExtendUp,
            "Extend Selection up",
            Some((shift, Key::ArrowUp)),
        ),
        (
            Action::ExtendDown,
            "Extend Selection down",
            Some((shift, Key::ArrowDown)),
        ),
        (Action::FocusUp, "Move focus up", Some((CTRL, Key::ArrowUp))),
        (
            Action::FocusDown,
            "Move focus down",
            Some((CTRL, Key::ArrowDown)),
        ),
        (
            Action::ToggleSelected,
            "Toggle Focused entry in Selection",
            Some((CTRL, Key::Space)),
        ),
        (Action::SelectAll, "Select all", Some((CTRL, Key::A))),
        (
            Action::ShowHidden,
            "Show hidden entries",
            Some((CTRL, Key::H)),
        ),
        (Action::Sort, "Sort", None),
    ];
    let map = KeyMap::default();
    for (action, name, binding) in expected {
        assert!(Action::ALL.contains(&action), "{name} is in ALL");
        assert_eq!(action.area(), Area::Browsing, "{name}");
        assert_eq!(action.name(), name);
        let binding = binding.map(|(m, k)| Binding::new(m, k));
        assert_eq!(map.binding(action), binding, "{name}");
    }
    assert_eq!(
        taken(&map, Key::ArrowUp, Key::ArrowUp, Modifiers::SHIFT),
        [Action::ExtendUp]
    );
    // Arrows read by name: the bundled fonts have no arrow symbols.
    let extend = map.binding(Action::ExtendUp).unwrap();
    assert_eq!(extend.label(false), "Shift+Up");
}
