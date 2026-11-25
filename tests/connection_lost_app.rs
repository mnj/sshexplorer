//! Search, Download and Preview in the window when the connection drops mid-operation,
//! against the test server: while Disconnected they are off and their keys do nothing,
//! a running Search ends as lost keeping its Matches, a running Download pauses and
//! finishes after Reconnect, and Preview keeps what it loaded.

mod common;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use common::fixture::{self, Fixture};
use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use sshexplorer::app::{self, App};
use sshexplorer::hosts::{Host, HostStore};
use sshexplorer::settings::{AppSettings, ReconnectAttempts, Store};

const CTRL: Modifiers = Modifiers::COMMAND;

struct Window {
    harness: Harness<'static, App>,
    local: PathBuf,
    _dir: tempfile::TempDir,
}

/// The window with the fixture saved as a Host downloading to a temporary Local folder,
/// with Reconnect attempts Off so that only F5 reconnects.
fn window(fixture: &Fixture) -> Window {
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("downloads");
    std::fs::create_dir_all(&local).unwrap();
    let store = Store::at(dir.path().join("settings.toml"));
    let mut settings = AppSettings::default();
    settings.connections.reconnect_attempts = ReconnectAttempts::Off;
    store.save(&settings).unwrap();
    let host = Host {
        display_name: "Fixture".into(),
        address: fixture.address().into(),
        port: Some(fixture.port()),
        account: fixture::ACCOUNT.into(),
        local_folder: local.clone(),
        ..Host::default()
    };
    HostStore::beside(store.path()).save(&[host]).unwrap();
    let known_hosts = dir.path().join("known_hosts");
    std::fs::write(&known_hosts, format!("{}\n", fixture.known_hosts_line())).unwrap();
    let app = App::with_store(store)
        .known_hosts_at(known_hosts)
        .ssh_config_at(dir.path().join("no-config"))
        .agent_at(None);
    let harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 800.0))
        .with_context_setup(app::setup)
        .build_ui_state(|ui, app: &mut App| app.ui(ui), app);
    let mut w = Window {
        harness,
        local,
        _dir: dir,
    };
    open(&mut w.harness);
    w
}

fn until(
    h: &mut Harness<'static, App>,
    what: &str,
    found: impl Fn(&Harness<'static, App>) -> bool,
) {
    let start = Instant::now();
    while !found(h) {
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(10));
        h.step();
    }
    h.run_ok();
}

/// Steps the window for `time`, as the network answers off the frame thread.
fn wait(h: &mut Harness<'static, App>, time: Duration) {
    let start = Instant::now();
    while start.elapsed() < time {
        std::thread::sleep(Duration::from_millis(10));
        h.step();
    }
    h.run_ok();
}

fn has(h: &Harness<'static, App>, label: &str) -> bool {
    h.query_all_by_label_contains(label).next().is_some()
}

fn has_row(h: &Harness<'static, App>, label: &str) -> bool {
    h.query_by_role_and_label(Role::Row, label).is_some()
}

/// Opens the saved Host and lists its home in Folder view.
fn open(h: &mut Harness<'static, App>) {
    h.key_press_modifiers(CTRL, Key::O);
    h.run_ok();
    h.get_by_role_and_label(Role::ListItem, "Fixture").click();
    h.run_ok();
    h.get_by_role_and_label(Role::Button, "Open Session")
        .click();
    h.run_ok();
    until(h, "the password prompt", |h| {
        h.query_by_role(Role::PasswordInput).is_some()
    });
    h.get_by_role(Role::PasswordInput).focus();
    h.run_ok();
    h.get_by_role(Role::PasswordInput)
        .type_text(fixture::PASSWORD);
    h.run_ok();
    h.get_by_role_and_label(Role::Button, "Connect").click();
    h.run_ok();
    until(h, "the tree", |h| has_row(h, "tester"));
    h.key_press_modifiers(CTRL, Key::Num2);
    h.run_ok();
    until(h, "the listing", |h| has_row(h, "notes.txt"));
}

fn drop_connection(fixture: &Fixture, h: &mut Harness<'static, App>) {
    fixture.drop_connections();
    until(h, "the strip", |h| has(h, "Connection lost"));
}

fn reconnect(h: &mut Harness<'static, App>) {
    h.key_press(Key::F5);
    h.run_ok();
    until(h, "the reconnect", |h| !has(h, "Connection lost"));
}

fn disabled(h: &Harness<'static, App>, role: Role, label: &str) -> bool {
    h.get_by_role_and_label(role, label)
        .accesskit_node()
        .is_disabled()
}

fn big() -> Vec<u8> {
    (0..2 * 1024 * 1024).map(|i| (i % 251) as u8).collect()
}

#[test]
fn while_disconnected_search_and_download_are_off_and_their_keys_do_nothing() {
    let fixture = Fixture::start();
    let mut w = window(&fixture);
    let h = &mut w.harness;
    h.get_by_role_and_label(Role::Row, "notes.txt").click();
    h.run_ok();
    assert!(!disabled(h, Role::Button, "Download"));
    assert!(!disabled(h, Role::TextInput, "Search"));

    drop_connection(&fixture, h);
    assert!(disabled(h, Role::Button, "Download"));
    assert!(disabled(h, Role::TextInput, "Search"));
    h.key_press_modifiers(CTRL, Key::S);
    h.run_ok();
    h.key_press_modifiers(CTRL, Key::F);
    h.run_ok();
    wait(h, Duration::from_millis(300));
    assert!(
        !h.get_by_role_and_label(Role::TextInput, "Search")
            .is_focused()
    );
    assert!(!has(h, "Downloaded"));
    assert_eq!(std::fs::read_dir(&w.local).unwrap().count(), 0);

    reconnect(h);
    assert!(!disabled(h, Role::Button, "Download"));
    assert!(!disabled(h, Role::TextInput, "Search"));
}

#[test]
fn a_running_search_ends_with_connection_lost_and_its_matches_stay() {
    let fixture = Fixture::start();
    fixture.hold_search();
    let mut w = window(&fixture);
    let h = &mut w.harness;
    h.key_press_modifiers(CTRL, Key::F);
    h.run_ok();
    h.get_by_role_and_label(Role::TextInput, "Search")
        .type_text("notes");
    h.run_ok();
    h.key_press(Key::Enter);
    h.run_ok();
    until(h, "the Match", |h| has(h, "Searching… 1 match"));

    drop_connection(&fixture, h);
    until(h, "the end", |h| has(h, "Search stopped: connection lost"));
    assert!(has_row(h, "notes.txt"));
    assert!(!has(h, "Searching"));
}

#[test]
fn a_running_download_pauses_without_its_temp_file_and_finishes_after_reconnect() {
    let fixture = Fixture::start();
    std::fs::write(fixture.root().join("home/tester/big.bin"), big()).unwrap();
    let mut w = window(&fixture);
    let h = &mut w.harness;
    assert!(has_row(h, "big.bin"));
    fixture.slow_reads(Duration::from_millis(50));
    h.get_by_role_and_label(Role::Row, "big.bin").click();
    h.run_ok();
    h.key_press_modifiers(CTRL, Key::S);
    h.run_ok();
    until(h, "the current file", |h| has(h, "of 1 files"));
    until(h, "the in-flight file", |_| {
        std::fs::read_dir(&w.local).unwrap().next().is_some()
    });
    assert!(!w.local.join("big.bin").exists());

    drop_connection(&fixture, h);
    until(h, "the pause", |h| has(h, "Paused: connection lost"));
    assert!(std::fs::read_dir(&w.local).unwrap().next().is_none());

    fixture.slow_reads(Duration::ZERO);
    reconnect(h);
    until(h, "the finish", |h| has(h, "Downloaded 1 file"));
    assert_eq!(std::fs::read(w.local.join("big.bin")).unwrap(), big());
}

#[test]
fn preview_keeps_what_is_loaded_and_new_loads_wait_for_the_connection() {
    let fixture = Fixture::start();
    std::fs::write(fixture.root().join("home/tester/todo.txt"), "first line\n").unwrap();
    let mut w = window(&fixture);
    let h = &mut w.harness;
    assert!(has_row(h, "todo.txt"));
    h.get_by_role_and_label(Role::Row, "notes.txt").click();
    until(h, "the text", |h| h.query_by_label("notes").is_some());

    drop_connection(&fixture, h);
    wait(h, Duration::from_millis(300));
    assert!(h.query_by_label("notes").is_some(), "what is loaded stays");

    h.get_by_role_and_label(Role::Row, "todo.txt").click();
    until(h, "the line", |h| has(h, "Cannot read: connection lost"));
    reconnect(h);
    until(h, "the text", |h| h.query_by_label("first line").is_some());
}

#[test]
fn an_in_flight_preview_load_shows_connection_lost() {
    let fixture = Fixture::start();
    let mut w = window(&fixture);
    let h = &mut w.harness;
    fixture.slow_reads(Duration::from_secs(5));
    h.get_by_role_and_label(Role::Row, "notes.txt").click();
    h.run_ok();
    // The fetch starts once focus has rested.
    wait(h, Duration::from_millis(500));
    assert!(has(h, "Loading preview"));

    drop_connection(&fixture, h);
    until(h, "the line", |h| has(h, "Cannot read: connection lost"));
    fixture.slow_reads(Duration::ZERO);
    reconnect(h);
    until(h, "the text", |h| h.query_by_label("notes").is_some());
}

#[test]
fn audio_cut_short_by_the_drop_reloads_after_reconnect() {
    let fixture = Fixture::start();
    let home = fixture.root().join("home/tester");
    std::fs::copy("tests/media/tone.ogg", home.join("tone.ogg")).unwrap();
    let mut w = window(&fixture);
    let h = &mut w.harness;
    assert!(has_row(h, "tone.ogg"));
    // The player waits on its first read while the connection drops.
    fixture.slow_reads(Duration::from_secs(5));
    h.get_by_role_and_label(Role::Row, "tone.ogg").click();
    h.run_ok();
    wait(h, Duration::from_millis(500));
    assert!(has(h, "Loading preview"));

    drop_connection(&fixture, h);
    until(h, "the line", |h| has(h, "Cannot read: connection lost"));
    fixture.slow_reads(Duration::ZERO);
    reconnect(h);
    until(h, "the player", |h| {
        h.query_by_role_and_label(Role::Button, "Play").is_some()
    });
}
