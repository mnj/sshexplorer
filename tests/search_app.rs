//! Search in the window, against the test server: Ctrl+F and Enter, the Matches in place
//! of the listing, moving into them, opening one, stopping and clearing, Refresh during
//! and after a Search, a host that cannot search, and the Search settings page.

#![cfg(unix)]

mod common;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use common::fixture::{self, Fixture};
use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use sshexplorer::app::{self, App};
use sshexplorer::settings::Store;

fn window(app: App) -> Harness<'static, App> {
    Harness::builder()
        .with_size(egui::vec2(1100.0, 700.0))
        .with_context_setup(app::setup)
        .build_ui_state(|ui, app: &mut App| app.ui(ui), app)
}

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

fn has_text(harness: &Harness<'static, App>, text: &str) -> bool {
    harness.query_all_by_label(text).next().is_some()
}

fn selected_rows(harness: &Harness<'static, App>) -> Vec<String> {
    harness
        .query_all_by_role(Role::Row)
        .filter(|r| r.accesskit_node().is_selected() == Some(true))
        .map(|r| r.accesskit_node().label().unwrap_or_default())
        .collect()
}

fn press(harness: &mut Harness<'static, App>, key: Key) {
    harness.key_press(key);
    harness.run_ok();
}

/// A window with a Session open on the fixture's home in Tree view.
fn session(fixture: &Fixture, dir: &tempfile::TempDir) -> Harness<'static, App> {
    let known_hosts: PathBuf = dir.path().join("known_hosts");
    std::fs::write(&known_hosts, format!("{}\n", fixture.known_hosts_line())).unwrap();
    let app = App::with_store(Store::at(dir.path().join("settings.toml")))
        .known_hosts_at(known_hosts)
        .ssh_config_at(dir.path().join("no-config"))
        .agent_at(None);
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

/// Ctrl+F, the text, and Enter.
fn search_for(harness: &mut Harness<'static, App>, text: &str) {
    harness.key_press_modifiers(Modifiers::COMMAND, Key::F);
    harness.run_ok();
    assert!(
        harness
            .get_by_role_and_label(Role::TextInput, "Search")
            .is_focused(),
        "Ctrl+F focuses the search field"
    );
    harness
        .get_by_role_and_label(Role::TextInput, "Search")
        .type_text(text);
    harness.run_ok();
    press(harness, Key::Enter);
}

fn refresh_enabled(harness: &Harness<'static, App>) -> bool {
    !harness
        .get_by_role_and_label(Role::Button, "Refresh")
        .accesskit_node()
        .is_disabled()
}

#[test]
fn matches_replace_the_listing_and_escape_brings_it_back() {
    let fixture = Fixture::start();
    std::fs::write(fixture.root().join("home/tester/docs/Notes-2.txt"), "n").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut harness = session(&fixture, &dir);

    search_for(&mut harness, "notes");
    until(&mut harness, "the Matches", |h| has_text(h, "2 matches"));
    assert!(has_row(&harness, "notes.txt"));
    assert!(has_row(&harness, "docs/Notes-2.txt"));
    assert!(!has_row(&harness, "tester"), "the listing is replaced");
    assert!(has_text(&harness, "File"));
    assert!(refresh_enabled(&harness));

    press(&mut harness, Key::Escape);
    assert!(has_row(&harness, "tester"), "the listing is back");
    assert!(!has_row(&harness, "notes.txt"));
}

#[test]
fn down_moves_into_the_matches_and_enter_on_a_directory_goes_there() {
    let fixture = Fixture::start();
    let dir = tempfile::tempdir().unwrap();
    let mut harness = session(&fixture, &dir);

    search_for(&mut harness, "doc");
    until(&mut harness, "the Matches", |h| has_text(h, "1 match"));
    press(&mut harness, Key::ArrowDown);
    press(&mut harness, Key::ArrowDown);
    assert_eq!(selected_rows(&harness), vec!["docs"]);
    press(&mut harness, Key::Enter);

    until(&mut harness, "the listing", |h| has_row(h, "tester"));
    assert_eq!(selected_rows(&harness), vec!["docs"]);
    assert!(
        has_text(&harness, "/home/tester/docs"),
        "docs is the Current directory"
    );
}

#[test]
fn a_file_match_opens_in_its_parent_fetched_again_when_its_kept_listing_lacks_it() {
    let fixture = Fixture::start();
    let dir = tempfile::tempdir().unwrap();
    let mut harness = session(&fixture, &dir);
    // Home's listing is kept; a file added since is not in it.
    press(&mut harness, Key::ArrowRight);
    until(&mut harness, "home's entries", |h| has_row(h, "notes.txt"));
    std::fs::write(fixture.root().join("home/tester/fresh.txt"), "f").unwrap();

    search_for(&mut harness, "fresh");
    until(&mut harness, "the Match", |h| has_row(h, "fresh.txt"));
    harness
        .get_by_role_and_label(Role::Row, "fresh.txt")
        .click();
    harness.run_ok();
    press(&mut harness, Key::Enter);

    until(&mut harness, "the file in its parent", |h| {
        selected_rows(h) == vec!["fresh.txt"]
    });
    assert!(has_row(&harness, "notes.txt"));
}

#[test]
fn a_running_search_disables_refresh_and_escape_stops_it_keeping_the_matches() {
    let fixture = Fixture::start();
    fixture.hold_search();
    let dir = tempfile::tempdir().unwrap();
    let mut harness = session(&fixture, &dir);

    search_for(&mut harness, "notes");
    until(&mut harness, "the Match", |h| has_row(h, "notes.txt"));
    assert!(has_text(&harness, "Searching… 1 match"));
    assert!(!refresh_enabled(&harness));
    harness.get_by_role_and_label(Role::Button, "Stop");

    press(&mut harness, Key::Escape);
    until(&mut harness, "the stopped line", |h| {
        has_text(h, "Search stopped. 1 match")
    });
    assert!(has_row(&harness, "notes.txt"));
    assert!(refresh_enabled(&harness));
}

#[test]
fn refresh_after_a_search_runs_it_again_and_keeps_the_focused_match() {
    let fixture = Fixture::start();
    let dir = tempfile::tempdir().unwrap();
    let mut harness = session(&fixture, &dir);
    std::fs::write(fixture.root().join("home/tester/docs/a-mark"), "a").unwrap();
    std::fs::write(fixture.root().join("home/tester/docs/b-mark"), "b").unwrap();

    search_for(&mut harness, "mark");
    until(&mut harness, "the Matches", |h| has_text(h, "2 matches"));
    harness
        .get_by_role_and_label(Role::Row, "docs/b-mark")
        .click();
    harness.run_ok();
    std::fs::write(fixture.root().join("home/tester/c-mark"), "c").unwrap();

    press(&mut harness, Key::F5);
    until(&mut harness, "the Matches again", |h| {
        has_text(h, "3 matches")
    });
    assert_eq!(selected_rows(&harness), vec!["docs/b-mark"]);
}

#[test]
fn a_host_that_cannot_search_says_so_and_browsing_goes_on() {
    let fixture = Fixture::start();
    fixture.refuse_search();
    let dir = tempfile::tempdir().unwrap();
    let mut harness = session(&fixture, &dir);

    search_for(&mut harness, "notes");
    until(&mut harness, "the message", |h| {
        has_text(h, "Search is not available on this host.")
    });
    press(&mut harness, Key::Escape);
    press(&mut harness, Key::ArrowRight);
    until(&mut harness, "home's entries", |h| has_row(h, "notes.txt"));
}

#[test]
fn the_search_page_has_follow_shared_with_download_and_the_match_cap() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::at(dir.path().join("settings.toml"));
    let mut harness = window(App::with_store(store.clone()));
    harness
        .get_by_role_and_label(Role::Button, "Settings")
        .click();
    harness.run_ok();
    harness
        .get_by_role_and_label(Role::ListItem, "Search")
        .click();
    harness.run_ok();
    assert!(has_text(&harness, "Shared with Download"));
    assert!(has_text(&harness, "Match cap"));
    harness
        .get_by_role_and_label(Role::Switch, "Follow symlinked directories")
        .click();
    harness.run_ok();
    assert!(store.load().search.follow_links);
}
