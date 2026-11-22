//! Disconnected and Reconnect in the window, against the test server: a killed connection
//! marks the tab and shows the strip, the listing stays, Refresh is off and F5 means
//! Reconnect, retries run and can be stopped, and a reconnect refreshes the Current
//! directory or hands a rejected secret to its prompt.

mod common;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use common::fixture::{self, Fixture};
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use sshexplorer::app::{self, App};
use sshexplorer::settings::{AppSettings, ReconnectAttempts, Store};

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
            start.elapsed() < Duration::from_secs(15),
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

fn has(harness: &Harness<'static, App>, label: &str) -> bool {
    harness.query_by_label(label).is_some()
}

fn has_button(harness: &Harness<'static, App>, label: &str) -> bool {
    harness
        .query_by_role_and_label(Role::Button, label)
        .is_some()
}

/// A node's text: a label's is its value.
fn text(n: &egui_kittest::kittest::AccessKitNode<'_>) -> Option<String> {
    n.label().or_else(|| n.value())
}

fn reconnecting(harness: &Harness<'static, App>) -> Option<String> {
    harness
        // The label and its text run both match.
        .query_all_by(|n| text(n).is_some_and(|l| l.starts_with("Reconnecting")))
        .next()
        .and_then(|n| text(&n.accesskit_node()))
}

fn selected_rows(harness: &Harness<'static, App>) -> Vec<String> {
    harness
        .query_all_by_role(Role::Row)
        .filter(|r| r.accesskit_node().is_selected() == Some(true))
        .map(|r| r.accesskit_node().label().unwrap_or_default())
        .collect()
}

fn enter_password(harness: &mut Harness<'static, App>, password: &str) {
    until(harness, "the password prompt", |h| {
        h.query_by_role(Role::PasswordInput).is_some()
    });
    type_into(harness, Role::PasswordInput, "Password", password);
    harness
        .get_by_role_and_label(Role::Button, "Connect")
        .click();
    harness.run_ok();
}

/// A window with a Session open on the fixture's home in Tree view, home expanded.
fn session(
    fixture: &Fixture,
    dir: &tempfile::TempDir,
    attempts: ReconnectAttempts,
) -> Harness<'static, App> {
    let known_hosts: PathBuf = dir.path().join("known_hosts");
    std::fs::write(&known_hosts, format!("{}\n", fixture.known_hosts_line())).unwrap();
    let store = Store::at(dir.path().join("settings.toml"));
    let mut settings = AppSettings::default();
    settings.connections.reconnect_attempts = attempts;
    store.save(&settings).unwrap();
    let app = App::with_store(store)
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
    enter_password(&mut harness, fixture::PASSWORD);
    until(&mut harness, "the tree", |h| !selected_rows(h).is_empty());
    harness.key_press(egui::Key::ArrowRight);
    harness.run_ok();
    until(&mut harness, "home's entries", |h| has_row(h, "notes.txt"));
    harness
}

fn tab_description(harness: &Harness<'static, App>) -> String {
    harness
        .query_all_by_role(Role::Tab)
        .next()
        .expect("a tab")
        .accesskit_node()
        .description()
        .unwrap_or_default()
}

const LOST: &str = "Connection lost: the host closed the connection";

#[test]
fn a_dropped_connection_shows_the_strip_keeps_the_listing_and_reconnects_by_itself() {
    let fixture = Fixture::start();
    let dir = tempfile::tempdir().unwrap();
    let mut harness = session(&fixture, &dir, ReconnectAttempts::Five);
    assert!(!tab_description(&harness).contains("Disconnected"));

    fixture.refuse(true);
    fixture.drop_connections();
    until(&mut harness, "the strip", |h| has(h, LOST));
    assert!(tab_description(&harness).contains("Disconnected"));
    // The kept listing stays browsable.
    assert!(has_row(&harness, "notes.txt"));
    harness.get_by_role_and_label(Role::Row, "docs").click();
    harness.run_ok();
    assert_eq!(selected_rows(&harness), ["docs"]);
    // Refresh is off while Disconnected.
    let refresh = harness.get_by_role_and_label(Role::Button, "Refresh");
    assert!(refresh.accesskit_node().is_disabled());
    // Retries run, counted against the setting.
    until(&mut harness, "a retry", |h| {
        reconnecting(h).is_some_and(|s| s.contains("(attempt 2 of 5)"))
    });
    assert!(has_button(&harness, "Reconnect now"));
    assert!(has_button(&harness, "Stop"));

    // The connection comes back, and the Current directory is fetched again.
    harness.get_by_role_and_label(Role::Row, "tester").click();
    harness.run_ok();
    fixture.refuse(false);
    let home = fixture.root().join("home/tester");
    std::fs::write(home.join("added.txt"), "new").unwrap();
    until(&mut harness, "the reconnect", |h| !has(h, LOST));
    until(&mut harness, "the refreshed listing", |h| {
        has_row(h, "added.txt")
    });
    assert!(!tab_description(&harness).contains("Disconnected"));
    assert!(
        !harness
            .get_by_role_and_label(Role::Button, "Refresh")
            .accesskit_node()
            .is_disabled()
    );
}

#[test]
fn stop_ends_the_retries_and_reconnect_tries_again() {
    let fixture = Fixture::start();
    let dir = tempfile::tempdir().unwrap();
    let mut harness = session(&fixture, &dir, ReconnectAttempts::Unlimited);
    fixture.refuse(true);
    fixture.drop_connections();
    until(&mut harness, "the countdown", |h| {
        reconnecting(h).is_some_and(|s| s.contains("(attempt 1)"))
    });
    harness.get_by_role_and_label(Role::Button, "Stop").click();
    harness.run_ok();
    assert!(reconnecting(&harness).is_none());
    assert!(has(&harness, LOST));
    assert!(has_button(&harness, "Reconnect"));
    assert!(!has_button(&harness, "Stop"));

    fixture.refuse(false);
    harness
        .get_by_role_and_label(Role::Button, "Reconnect")
        .click();
    harness.run_ok();
    until(&mut harness, "the reconnect", |h| !has(h, LOST));
}

#[test]
fn with_reconnect_off_f5_reconnects() {
    let fixture = Fixture::start();
    let dir = tempfile::tempdir().unwrap();
    let mut harness = session(&fixture, &dir, ReconnectAttempts::Off);
    fixture.drop_connections();
    until(&mut harness, "the strip", |h| has(h, LOST));
    // Nothing retries by itself.
    for _ in 0..30 {
        std::thread::sleep(Duration::from_millis(50));
        harness.step();
    }
    assert!(has(&harness, LOST));
    assert!(reconnecting(&harness).is_none());

    harness.key_press(egui::Key::F5);
    harness.run_ok();
    until(&mut harness, "the reconnect", |h| !has(h, LOST));
    assert!(has_row(&harness, "notes.txt"));
}

#[test]
fn a_rejected_password_on_reconnect_asks_for_it() {
    let fixture = Fixture::start();
    let dir = tempfile::tempdir().unwrap();
    let mut harness = session(&fixture, &dir, ReconnectAttempts::Five);
    fixture.set_password("changed");
    fixture.drop_connections();
    until(&mut harness, "the password prompt", |h| {
        h.query_by_role(Role::PasswordInput).is_some()
    });
    assert!(has(&harness, "The password was rejected."));
    enter_password(&mut harness, "changed");
    until(&mut harness, "the reconnect", |h| !has(h, LOST));
    assert!(has_row(&harness, "notes.txt"));
}

#[test]
fn reconnect_attempts_are_chosen_on_the_connections_page() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::at(dir.path().join("settings.toml"));
    let mut harness = window(App::with_store(store.clone()));
    harness.run_ok();
    harness
        .get_by_role_and_label(Role::Button, "Settings")
        .click();
    harness.run_ok();
    harness
        .get_by_role_and_label(Role::ListItem, "Connections")
        .click();
    harness.run_ok();
    harness.get_by_role_and_label(Role::Label, "Reconnect attempts");
    harness.get_by_value("5").click();
    harness.run_ok();
    harness.get_by_label("Unlimited").click();
    harness.run_ok();
    assert_eq!(
        store.load().connections.reconnect_attempts,
        ReconnectAttempts::Unlimited
    );
}
