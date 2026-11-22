//! Settings → Keys, the binding editor: every action of the key map by area, each with
//! its key chip; a click on a chip records new keys, which apply and are kept at once.

use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use sshexplorer::app::{self, App};
use sshexplorer::keymap::{Action, Binding};
use sshexplorer::settings::Store;

struct Launched {
    harness: Harness<'static, App>,
    store: Store,
    _dir: tempfile::TempDir,
}

/// Launches the app on an empty settings file and opens Settings → Keys.
fn keys_page() -> Launched {
    let dir = tempfile::tempdir().expect("a temp dir");
    let store = Store::at(dir.path().join("settings.toml"));
    let mut l = relaunch(store, dir);
    open_keys(&mut l.harness);
    l
}

/// Launches the app on `store`'s settings file, which lives in `dir`.
fn relaunch(store: Store, dir: tempfile::TempDir) -> Launched {
    let app = App::with_store(store.clone());
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 900.0))
        .with_context_setup(app::setup)
        .build_ui_state(|ui, app: &mut App| app.ui(ui), app);
    harness.run();
    Launched {
        harness,
        store,
        _dir: dir,
    }
}

fn open_keys(h: &mut Harness<'static, App>) {
    h.get_by_role_and_label(Role::Button, "Settings").click();
    h.run();
    h.get_by_role_and_label(Role::ListItem, "Keys").click();
    h.run();
}

fn is_open(h: &Harness<'static, App>) -> bool {
    h.query_by_role_and_label(Role::Dialog, "Changes apply at once")
        .is_some()
}

fn middle(h: &Harness<'static, App>, role: Role, label: &str) -> f32 {
    h.get_by_role_and_label(role, label).rect().center().y
}

fn click_chip(h: &mut Harness<'static, App>, keys: &str) {
    h.get_by_role_and_label(Role::Button, keys).click();
    h.run();
}

fn is_recording(h: &Harness<'static, App>) -> bool {
    h.query_by_role_and_label(Role::Button, "Press keys…")
        .is_some()
        && h.query_by_label("Press the new keys. Escape cancels.")
            .is_some()
}

const CTRL: Modifiers = Modifiers::COMMAND;

#[test]
fn the_table_lists_each_action_with_its_chip_under_its_area_in_order() {
    let l = keys_page();
    let h = &l.harness;
    let sessions = middle(h, Role::Label, "Sessions");
    let close = middle(h, Role::Label, "Close Session");
    let appearance = middle(h, Role::Label, "Appearance");
    let settings = middle(h, Role::Label, "Settings");
    assert!(sessions < close && close < appearance && appearance < settings);
    // Each chip is a button on its action's row.
    assert!((middle(h, Role::Button, "Ctrl+W") - close).abs() < 4.0);
    assert!((middle(h, Role::Button, "Ctrl+,") - settings).abs() < 4.0);
    h.get_by_role_and_label(Role::Button, "Ctrl+Shift+=");
    // Nothing differs from the default, so no row has Reset.
    assert!(h.query_by_role_and_label(Role::Button, "Reset").is_none());
    h.get_by_role_and_label(Role::Button, "Reset all keys");
}

#[test]
fn a_click_on_a_chip_records_and_a_free_combination_is_saved_at_once() {
    let mut l = keys_page();
    let h = &mut l.harness;
    click_chip(h, "Ctrl+,");
    assert!(is_recording(h));
    h.key_press(Key::F2);
    h.run();
    assert!(!is_recording(h));
    h.get_by_role_and_label(Role::Button, "F2");
    let f2 = Binding::new(Modifiers::NONE, Key::F2);
    assert_eq!(l.store.load().keys.binding(Action::Settings), Some(f2));
}

#[test]
fn a_bound_combination_names_its_action_and_recording_continues() {
    let mut l = keys_page();
    let h = &mut l.harness;
    click_chip(h, "Ctrl+,");
    h.key_press_modifiers(CTRL, Key::W);
    h.run();
    h.get_by_label("Ctrl+W is already bound to “Close Session”. Change that one first.");
    assert!(is_recording(h));
    // The next free combination is saved.
    h.key_press_modifiers(CTRL, Key::K);
    h.run();
    assert!(!is_recording(h));
    h.get_by_role_and_label(Role::Button, "Ctrl+K");
    h.get_by_role_and_label(Role::Button, "Ctrl+W");
}

#[test]
fn the_settings_keys_are_recorded_not_run_while_recording() {
    let mut l = keys_page();
    let h = &mut l.harness;
    click_chip(h, "Ctrl+W");
    h.key_press_modifiers(CTRL, Key::Comma);
    h.run();
    assert!(is_open(h));
    h.get_by_label("Ctrl+, is already bound to “Settings”. Change that one first.");
}

#[test]
fn escape_cancels_recording_and_is_not_bound() {
    let mut l = keys_page();
    let h = &mut l.harness;
    click_chip(h, "Ctrl+,");
    h.key_press(Key::Escape);
    h.run();
    assert!(!is_recording(h));
    assert!(is_open(h), "Escape cancels recording, not the dialog");
    h.get_by_role_and_label(Role::Button, "Ctrl+,");
    assert!(!l.store.load().keys.is_changed(Action::Settings));
}

#[test]
fn reset_shows_on_a_changed_row_and_puts_its_default_back() {
    let mut l = keys_page();
    let h = &mut l.harness;
    click_chip(h, "Ctrl+W");
    h.key_press(Key::F4);
    h.run();
    let reset = h.get_by_role_and_label(Role::Button, "Reset");
    assert!((reset.rect().center().y - middle(h, Role::Label, "Close Session")).abs() < 4.0);
    reset.click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Ctrl+W");
    assert!(h.query_by_role_and_label(Role::Button, "Reset").is_none());
    assert!(!l.store.load().keys.is_changed(Action::CloseSession));
}

#[test]
fn reset_all_keys_puts_every_default_back() {
    let mut l = keys_page();
    let h = &mut l.harness;
    click_chip(h, "Ctrl+W");
    h.key_press(Key::F4);
    h.run();
    click_chip(h, "Ctrl+,");
    h.key_press(Key::F2);
    h.run();
    h.get_by_role_and_label(Role::Button, "Reset all keys")
        .click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Ctrl+W");
    h.get_by_role_and_label(Role::Button, "Ctrl+,");
    assert_eq!(l.store.load().keys, Default::default());
}

#[test]
fn rebound_keys_are_kept_across_a_restart_and_answer_at_once() {
    let mut l = keys_page();
    let h = &mut l.harness;
    click_chip(h, "Ctrl+,");
    h.key_press(Key::F2);
    h.run();
    let Launched { store, _dir, .. } = l;
    let mut l = relaunch(store, _dir);
    let h = &mut l.harness;
    h.key_press_modifiers(CTRL, Key::Comma);
    h.run();
    assert!(!is_open(h), "Ctrl+, no longer opens Settings");
    h.key_press(Key::F2);
    h.run();
    assert!(is_open(h));
    h.get_by_role_and_label(Role::ListItem, "Keys").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "F2");
}

#[test]
fn cmd_shows_in_place_of_ctrl_on_macos() {
    let mut l = keys_page();
    let h = &mut l.harness;
    h.ctx.set_os(egui::os::OperatingSystem::Mac);
    h.run();
    h.get_by_role_and_label(Role::Button, "⌘W");
    h.get_by_role_and_label(Role::Button, "⇧⌘=");
}
