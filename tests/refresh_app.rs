//! Refresh in the window, against the test server: F5 and the toolbar button fetch the
//! listing again and merge it in, a directory gone from under Folder view lists its
//! nearest ancestor with a note, and Auto-refresh is chosen on the Browsing page.

mod common;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use common::fixture::{self, Fixture};
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use sshexplorer::app::{self, App};
use sshexplorer::settings::{AutoRefresh, Store};

fn window(app: App) -> Harness<'static, App> {
    Harness::builder()
        .with_size(egui::vec2(1000.0, 700.0))
        .with_context_setup(app::setup)
        .build_ui_state(|ui, app: &mut App| app.ui(ui), app)
}

/// Steps the window until `found` holds, as the network answers off the frame thread.
fn until(
    harness: &mut Harness<'static, App>,
    what: &str,
    found: impl Fn(&Harness<'static, App>) -> bool,
) {
    let start = Instant::now();
    while !found(harness) {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(10));
        harness.step();
    }
    harness.run_ok();
}

fn type_into(harness: &mut Harness<'static, App>, role: Role, label: &str, text: &str) {
    harness.get_by_role_and_label(role, label).focus();
    harness.run_ok();
    harness.get_by_role_and_label(role, label).type_text(text);
    harness.run_ok();
}

fn has_row(harness: &Harness<'static, App>, label: &str) -> bool {
    harness.query_by_role_and_label(Role::Row, label).is_some()
}

fn selected_rows(harness: &Harness<'static, App>) -> Vec<String> {
    harness
        .query_all_by_role(Role::Row)
        .filter(|r| r.accesskit_node().is_selected() == Some(true))
        .map(|r| r.accesskit_node().label().unwrap_or_default())
        .collect()
}

/// A window with a Session open on the fixture's home in Tree view, `folder` as the
/// Initial remote folder (empty for home), once it is focused.
fn session(fixture: &Fixture, dir: &tempfile::TempDir, folder: &str) -> Harness<'static, App> {
    let known_hosts: PathBuf = dir.path().join("known_hosts");
    std::fs::write(&known_hosts, format!("{}\n", fixture.known_hosts_line())).unwrap();
    let app = App::with_store(Store::at(dir.path().join("settings.toml")))
        .known_hosts_at(known_hosts)
        .ssh_config_at(dir.path().join("no-config"));
    let mut harness = window(app);
    harness.get_by_role_and_label(Role::Button, "Hosts").click();
    harness.run_ok();
    harness
        .get_by_role_and_label(Role::ListItem, "New Host")
        .click();
    harness.run_ok();
    type_into(&mut harness, Role::TextInput, "Address", fixture.address());
    let port = fixture.port().to_string();
    type_into(&mut harness, Role::TextInput, "Port", &port);
    type_into(&mut harness, Role::TextInput, "Account", fixture::ACCOUNT);
    if !folder.is_empty() {
        type_into(
            &mut harness,
            Role::TextInput,
            "Initial remote folder",
            folder,
        );
    }
    harness
        .get_by_role_and_label(Role::Button, "Open Session")
        .click();
    harness.run_ok();
    until(&mut harness, "the password prompt", |h| {
        h.query_by_role(Role::PasswordInput).is_some()
    });
    type_into(
        &mut harness,
        Role::PasswordInput,
        "Password",
        fixture::PASSWORD,
    );
    harness
        .get_by_role_and_label(Role::Button, "Connect")
        .click();
    harness.run_ok();
    until(&mut harness, "the tree", |h| !selected_rows(h).is_empty());
    harness
}

fn press(harness: &mut Harness<'static, App>, key: egui::Key) {
    harness.key_press(key);
    harness.run_ok();
}

#[test]
fn f5_and_the_button_merge_new_and_removed_entries_into_tree_view() {
    let fixture = Fixture::start();
    let home = fixture.root().join("home/tester");
    let dir = tempfile::tempdir().unwrap();
    let mut harness = session(&fixture, &dir, "");
    // Expand home and docs.
    press(&mut harness, egui::Key::ArrowRight);
    until(&mut harness, "home's entries", |h| has_row(h, "notes.txt"));
    std::fs::write(home.join("docs/plan.txt"), "p").unwrap();
    harness.get_by_role_and_label(Role::Row, "docs").click();
    harness.run_ok();
    press(&mut harness, egui::Key::ArrowRight);
    until(&mut harness, "docs' entries", |h| has_row(h, "plan.txt"));
    harness
        .get_by_role_and_label(Role::Row, "notes.txt")
        .click();
    harness.run_ok();

    std::fs::write(home.join("added.txt"), "new").unwrap();
    std::fs::write(home.join("docs/later.txt"), "new").unwrap();
    std::fs::remove_file(home.join("notes.txt")).unwrap();
    press(&mut harness, egui::Key::F5);
    until(&mut harness, "the refreshed rows", |h| {
        has_row(h, "added.txt") && has_row(h, "later.txt") && !has_row(h, "notes.txt")
    });
    // docs stayed expanded; the Focused entry moved on to the next row.
    assert!(has_row(&harness, "plan.txt"));
    assert_eq!(selected_rows(&harness), ["readme.md"]);

    std::fs::write(home.join("by-button.txt"), "new").unwrap();
    harness
        .get_by_role_and_label(Role::Button, "Refresh")
        .click();
    harness.run_ok();
    until(&mut harness, "the button's Refresh", |h| {
        has_row(h, "by-button.txt")
    });
}

#[test]
fn a_gone_folder_lists_its_nearest_ancestor_with_a_note() {
    let fixture = Fixture::start();
    let home = fixture.root().join("home/tester");
    std::fs::create_dir_all(home.join("docs/deep/er")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut harness = session(&fixture, &dir, "docs/deep/er");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Num2);
    harness.run_ok();
    until(&mut harness, "Folder view", |h| has_row(h, ".."));

    std::fs::remove_dir_all(home.join("docs/deep")).unwrap();
    std::fs::write(home.join("docs/left.txt"), "x").unwrap();
    press(&mut harness, egui::Key::F5);
    until(&mut harness, "the ancestor", |h| has_row(h, "left.txt"));
    harness.get_by_label_contains("/home/tester/docs/deep/er no longer exists.");
}

#[test]
fn auto_refresh_is_chosen_on_the_browsing_page() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::at(dir.path().join("settings.toml"));
    let mut harness = window(App::with_store(store.clone()));
    harness.run_ok();
    harness
        .get_by_role_and_label(Role::Button, "Settings")
        .click();
    harness.run_ok();
    harness
        .get_by_role_and_label(Role::ListItem, "Browsing")
        .click();
    harness.run_ok();
    harness.get_by_role_and_label(Role::Label, "Auto-refresh");
    harness.get_by_value("Off").click();
    harness.run_ok();
    harness.get_by_label("30 s").click();
    harness.run_ok();
    assert_eq!(store.load().browsing.auto_refresh, AutoRefresh::Seconds30);
}
