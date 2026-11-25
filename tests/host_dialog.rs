//! The Host dialog: its left column of saved Hosts, New Host and Unsaved Session, every
//! field, Save, Delete and Open Session; and the Sessions opened from it, with their tab
//! labels, the tab-bar edge, Ctrl+Tab and the toolbar's Readonly and Local folder.

mod common;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use common::fixture::{self, Fixture};
use egui::accesskit::{Role, Toggled};
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use sshexplorer::app::{self, App};
use sshexplorer::hosts::{self, Host, HostStore};
use sshexplorer::settings::{AppSettings, Edge, Store};

fn window(app: App) -> Harness<'static, App> {
    Harness::builder()
        .with_size(egui::vec2(1100.0, 800.0))
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

fn ctrl_o(harness: &mut Harness<'static, App>) {
    harness.key_press_modifiers(Modifiers::COMMAND, Key::O);
    harness.run_ok();
}

fn field(harness: &Harness<'static, App>, label: &str) -> String {
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .value()
        .unwrap_or_default()
}

/// Replaces what the field `label` holds with `text`.
fn fill(harness: &mut Harness<'static, App>, label: &str, text: &str) {
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .focus();
    harness.run_ok();
    harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
    harness.key_press(Key::Backspace);
    harness.run_ok();
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .type_text(text);
    harness.run_ok();
}

fn press(harness: &mut Harness<'static, App>, button: &str) {
    harness.get_by_role_and_label(Role::Button, button).click();
    harness.run_ok();
}

fn is_on(harness: &Harness<'static, App>, role: Role, label: &str) -> bool {
    harness
        .get_by_role_and_label(role, label)
        .accesskit_node()
        .toggled()
        == Some(Toggled::True)
}

fn dialog_open(harness: &Harness<'static, App>) -> bool {
    harness
        .query_by_role_and_label(Role::TextInput, "Address")
        .is_some()
}

fn store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::at(dir.path().join("settings.toml"));
    (dir, store)
}

#[test]
fn ctrl_o_opens_a_new_host_with_every_field_and_escape_dismisses() {
    let mut harness = window(App::default());
    ctrl_o(&mut harness);

    assert!(is_on(&harness, Role::ListItem, "New Host"));
    assert!(!is_on(&harness, Role::ListItem, "Unsaved Session"));
    for name in [
        "Display name",
        "Address",
        "Port",
        "Account",
        "Key file",
        "Local folder",
        "Initial remote folder",
    ] {
        assert!(
            harness
                .query_by_role_and_label(Role::TextInput, name)
                .is_some(),
            "{name} is a field"
        );
    }
    let auth = harness.get_by_role_and_label(Role::ComboBox, "Authentication method");
    assert_eq!(auth.value().as_deref(), Some("Automatic"));
    assert_eq!(
        field(&harness, "Local folder"),
        hosts::downloads_dir().display().to_string()
    );
    assert_eq!(field(&harness, "Initial remote folder"), "");
    assert!(is_on(&harness, Role::Switch, "Readonly"));
    assert!(!is_on(&harness, Role::Switch, "Remember"));

    // Down picks Unsaved Session, which has no Remember.
    harness.key_press(Key::ArrowDown);
    harness.run_ok();
    assert!(is_on(&harness, Role::ListItem, "Unsaved Session"));
    assert!(
        harness
            .query_by_role_and_label(Role::Switch, "Remember")
            .is_none()
    );
    harness.key_press(Key::ArrowUp);
    harness.run_ok();
    assert!(is_on(&harness, Role::ListItem, "New Host"));

    harness.key_press(Key::Escape);
    harness.run_ok();
    assert!(!dialog_open(&harness));
}

#[test]
fn save_keeps_the_dialog_open_and_the_host_persists_and_delete_removes_it() {
    let (dir, settings) = store();
    let mut harness = window(App::with_store(settings.clone()));
    ctrl_o(&mut harness);
    fill(&mut harness, "Display name", "Web");
    fill(&mut harness, "Address", "web.example.org");
    fill(&mut harness, "Port", "2222");
    fill(&mut harness, "Initial remote folder", "/var/www");
    press(&mut harness, "Save");

    assert!(dialog_open(&harness));
    assert!(is_on(&harness, Role::ListItem, "Web"));
    let saved = HostStore::at(dir.path().join("hosts.toml")).load();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].display_name, "Web");
    assert_eq!(saved[0].address, "web.example.org");
    assert_eq!(saved[0].port, Some(2222));
    assert_eq!(saved[0].initial_folder, "/var/www");
    assert!(saved[0].readonly);

    // A new run lists it, first and picked.
    drop(harness);
    let mut harness = window(App::with_store(settings));
    ctrl_o(&mut harness);
    assert!(is_on(&harness, Role::ListItem, "Web"));
    assert_eq!(field(&harness, "Address"), "web.example.org");
    assert_eq!(field(&harness, "Port"), "2222");

    press(&mut harness, "Delete");
    assert!(dialog_open(&harness));
    assert!(
        harness
            .query_by_role_and_label(Role::ListItem, "Web")
            .is_none()
    );
    assert!(
        HostStore::at(dir.path().join("hosts.toml"))
            .load()
            .is_empty()
    );
}

#[test]
fn a_bad_port_disables_save_and_open_session() {
    let mut harness = window(App::default());
    ctrl_o(&mut harness);
    fill(&mut harness, "Address", "example.org");
    fill(&mut harness, "Port", "0");
    assert!(harness.query_by_label("Port must be 1 to 65535").is_some());
    for button in ["Save", "Open Session"] {
        let node = harness.get_by_role_and_label(Role::Button, button);
        assert!(node.accesskit_node().is_disabled(), "{button} is disabled");
    }
    fill(&mut harness, "Port", "");
    assert!(harness.query_by_label("Port must be 1 to 65535").is_none());
    let node = harness.get_by_role_and_label(Role::Button, "Save");
    assert!(!node.accesskit_node().is_disabled());
}

#[test]
fn leaving_address_moves_a_pasted_port_and_account_into_empty_fields() {
    let mut harness = window(App::default());
    ctrl_o(&mut harness);
    fill(&mut harness, "Address", "deploy@example.org:2222");
    harness
        .get_by_role_and_label(Role::TextInput, "Display name")
        .focus();
    harness.run_ok();
    assert_eq!(field(&harness, "Address"), "example.org");
    assert_eq!(field(&harness, "Port"), "2222");
    assert_eq!(field(&harness, "Account"), "deploy");

    // Now both are set: the pasted text stays, and says why.
    fill(&mut harness, "Address", "root@example.org:22");
    harness
        .get_by_role_and_label(Role::TextInput, "Display name")
        .focus();
    harness.run_ok();
    assert_eq!(field(&harness, "Address"), "root@example.org:22");
    assert_eq!(field(&harness, "Port"), "2222");
    assert_eq!(field(&harness, "Account"), "deploy");
    assert!(harness.query_by_label("Port is already set").is_some());
    assert!(harness.query_by_label("Account is already set").is_some());
}

/// A saved Host for the fixture, opening in `docs`, not readonly, downloading into `local`.
fn fixture_host(fixture: &Fixture, local: PathBuf) -> Host {
    Host {
        display_name: "Fixture".into(),
        address: fixture.address().into(),
        port: Some(fixture.port()),
        account: fixture::ACCOUNT.into(),
        initial_folder: "docs".into(),
        local_folder: local,
        readonly: false,
        ..Host::default()
    }
}

/// A window whose settings and Hosts are in `dir`, with `hosts` saved and the fixture's
/// key known.
fn window_with_hosts(
    dir: &tempfile::TempDir,
    fixture: &Fixture,
    hosts: &[Host],
    settings: AppSettings,
) -> Harness<'static, App> {
    let store = Store::at(dir.path().join("settings.toml"));
    store.save(&settings).unwrap();
    HostStore::beside(store.path()).save(hosts).unwrap();
    let known_hosts = dir.path().join("known_hosts");
    std::fs::write(&known_hosts, format!("{}\n", fixture.known_hosts_line())).unwrap();
    window(
        App::with_store(store)
            .known_hosts_at(known_hosts)
            .agent_at(None),
    )
}

fn enter_password(harness: &mut Harness<'static, App>) {
    until(harness, "the password prompt", |h| {
        h.query_by_role(Role::PasswordInput).is_some()
    });
    harness.get_by_role(Role::PasswordInput).focus();
    harness.run_ok();
    harness
        .get_by_role(Role::PasswordInput)
        .type_text(fixture::PASSWORD);
    harness.run_ok();
    press(harness, "Connect");
}

fn tab_label_for(harness: &Harness<'static, App>, label: &str) -> bool {
    harness.query_by_role_and_label(Role::Tab, label).is_some()
}

#[test]
fn enter_opens_a_saved_host_in_its_initial_remote_folder_under_its_display_name() {
    let fixture = Fixture::start();
    std::fs::write(fixture.root().join("home/tester/docs/plan.txt"), "plan\n").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("downloads");
    let mut harness = window_with_hosts(
        &dir,
        &fixture,
        &[fixture_host(&fixture, local.clone())],
        AppSettings::default(),
    );
    ctrl_o(&mut harness);
    assert!(is_on(&harness, Role::ListItem, "Fixture"));
    harness.key_press(Key::Enter);
    harness.run_ok();
    assert!(!dialog_open(&harness));
    enter_password(&mut harness);

    until(&mut harness, "the Session tab", |h| {
        tab_label_for(h, "Fixture")
    });
    // Tree view opens on the Initial remote folder; Folder view lists it.
    until(&mut harness, "the tree", |h| {
        h.query_by_role_and_label(Role::Row, "docs").is_some()
    });
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Num2);
    harness.run_ok();
    until(&mut harness, "the listing", |h| {
        h.query_by_label("plan.txt").is_some()
    });
    assert!(harness.query_by_label("notes.txt").is_none());
    // The port is not 22, so the tab's tooltip names it.
    let tab = harness.get_by_role_and_label(Role::Tab, "Fixture");
    let tooltip = tab.accesskit_node().description().unwrap_or_default();
    assert!(
        tooltip.contains(&format!(":{}", fixture.port())),
        "{tooltip}"
    );
    // The toolbar shows the Local folder, and no Readonly for a writable Host.
    assert!(
        harness
            .query_by_label(&local.display().to_string())
            .is_some()
    );
    assert!(harness.query_by_label("Readonly").is_none());
}

#[test]
fn tabs_follow_the_tab_bar_edge_and_ctrl_tab_cycles_sessions() {
    let fixture = Fixture::start();
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("downloads");
    let readonly = Host {
        display_name: "Second".into(),
        readonly: true,
        initial_folder: String::new(),
        ..fixture_host(&fixture, local.clone())
    };
    let mut settings = AppSettings::default();
    settings.appearance.tab_bar_edge = Edge::Left;
    let mut harness = window_with_hosts(
        &dir,
        &fixture,
        &[fixture_host(&fixture, local), readonly],
        settings,
    );
    for (pick, tab) in [(0, "Fixture"), (1, "Second")] {
        ctrl_o(&mut harness);
        for _ in 0..pick {
            harness.key_press(Key::ArrowDown);
            harness.run_ok();
        }
        press(&mut harness, "Open Session");
        enter_password(&mut harness);
        until(&mut harness, "the Session tab", |h| tab_label_for(h, tab));
    }
    // The second, readonly Session is in front, and the toolbar says Readonly.
    assert!(is_on(&harness, Role::Tab, "Second"));
    assert!(harness.query_by_label("Readonly").is_some());

    // On the left edge the tabs are stacked down the left of the listing.
    let first = harness.get_by_role_and_label(Role::Tab, "Fixture").rect();
    let second = harness.get_by_role_and_label(Role::Tab, "Second").rect();
    until(&mut harness, "the tree", |h| {
        h.query_by_role_and_label(Role::Row, "tester").is_some()
    });
    let listing = harness.get_by_role_and_label(Role::Row, "tester").rect();
    assert!(second.top() >= first.bottom() - 1.0, "{first:?} {second:?}");
    assert!(first.right() <= listing.left(), "{first:?} {listing:?}");

    let ctrl_shift = Modifiers {
        shift: true,
        ..Modifiers::COMMAND
    };
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Tab);
    harness.run_ok();
    assert!(is_on(&harness, Role::Tab, "Fixture"));
    assert!(harness.query_by_label("Readonly").is_none());
    harness.key_press_modifiers(ctrl_shift, Key::Tab);
    harness.run_ok();
    assert!(is_on(&harness, Role::Tab, "Second"));
    harness.key_press_modifiers(ctrl_shift, Key::Tab);
    harness.run_ok();
    assert!(is_on(&harness, Role::Tab, "Fixture"));
}

#[test]
fn a_host_save_can_be_retried_after_the_directory_becomes_writable() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config");
    std::fs::write(&config, "blocks directory creation").unwrap();
    let settings = Store::at(config.join("settings.toml"));
    let hosts = HostStore::beside(settings.path());
    let mut h = window(App::with_store(settings));
    ctrl_o(&mut h);
    fill(&mut h, "Address", "build-host");
    press(&mut h, "Save");
    assert_eq!(field(&h, "Address"), "build-host");
    h.get_by_role_and_label(Role::Button, "Retry");
    std::fs::remove_file(config).unwrap();
    press(&mut h, "Retry");
    assert!(h.query_by_role_and_label(Role::Button, "Retry").is_none());
    assert_eq!(hosts.load()[0].address, "build-host");
}
