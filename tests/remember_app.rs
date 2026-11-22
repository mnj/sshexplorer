//! Remember from the window: a remembering saved Host opens with its kept secret, the
//! secret dialog says what the credential store did, and saving or deleting the Host
//! removes the secret by the rules. The store is a MemoryStore, never the OS one, and
//! no test reaches the operator's SSH agent.

mod common;

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::fixture::{self, Fixture};
use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use sshexplorer::app::{self, App};
use sshexplorer::credentials::{self, Kind, MemoryStore, Store as _};
use sshexplorer::hosts::{AuthMethod, Host, HostStore};
use sshexplorer::settings::{AppSettings, Store};

const ID: &str = "fixture-host";

fn key(kind: Kind) -> credentials::Key {
    credentials::Key::new(ID, kind)
}

/// The window, with the fixture saved as a remembering Host "Fixture" whose secrets
/// `store` keeps. Ctrl+O opens the Host dialog on it.
fn window(dir: &Path, fixture: &Fixture, store: Arc<MemoryStore>) -> Harness<'static, App> {
    let settings = Store::at(dir.join("settings.toml"));
    settings.save(&AppSettings::default()).unwrap();
    let host = Host {
        id: ID.into(),
        display_name: "Fixture".into(),
        address: fixture.address().into(),
        port: Some(fixture.port()),
        account: fixture::ACCOUNT.into(),
        auth: AuthMethod::Password,
        remember: true,
        ..Host::default()
    };
    HostStore::beside(settings.path()).save(&[host]).unwrap();
    let known_hosts = dir.join("known_hosts");
    std::fs::write(&known_hosts, format!("{}\n", fixture.known_hosts_line())).unwrap();
    let config = dir.join("ssh_config");
    std::fs::write(&config, "").unwrap();
    let app = App::with_store(settings)
        .known_hosts_at(known_hosts)
        .ssh_config_at(config)
        .agent_at(None)
        .credentials_at(store);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 800.0))
        .with_context_setup(app::setup)
        .build_ui_state(|ui, app: &mut App| app.ui(ui), app);
    harness.key_press_modifiers(Modifiers::COMMAND, Key::O);
    harness.run_ok();
    harness
}

fn open_session(harness: &mut Harness<'static, App>) {
    harness.key_press(Key::Enter);
    harness.run_ok();
}

fn until(
    harness: &mut Harness<'static, App>,
    what: &str,
    found: impl Fn(&Harness<'static, App>) -> bool,
) {
    let start = Instant::now();
    while !found(harness) {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(10));
        harness.step();
    }
    harness.run_ok();
}

fn shows(harness: &Harness<'static, App>, text: &str) -> bool {
    harness.query_by_label_contains(text).is_some()
}

fn has_tab(harness: &Harness<'static, App>) -> bool {
    harness
        .query_by_role_and_label(Role::Tab, "Fixture")
        .is_some()
}

fn asks_password(harness: &Harness<'static, App>) -> bool {
    harness
        .query_by_role_and_label(Role::PasswordInput, "Password")
        .is_some()
}

fn press(harness: &mut Harness<'static, App>, button: &str) {
    harness.get_by_role_and_label(Role::Button, button).click();
    harness.run_ok();
}

fn enter_password(harness: &mut Harness<'static, App>, password: &str) {
    let field = harness.get_by_role_and_label(Role::PasswordInput, "Password");
    field.focus();
    harness.run_ok();
    harness
        .get_by_role_and_label(Role::PasswordInput, "Password")
        .type_text(password);
    harness.run_ok();
    press(harness, "Connect");
}

#[test]
fn a_kept_password_opens_the_session_without_the_dialog() {
    let fixture = Fixture::start();
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(MemoryStore::default());
    store.set(&key(Kind::Password), fixture::PASSWORD).unwrap();
    let mut harness = window(dir.path(), &fixture, store);
    open_session(&mut harness);
    until(&mut harness, "the Session tab", has_tab);
    assert!(!asks_password(&harness));
}

#[test]
fn with_no_kept_secret_the_dialog_is_the_ordinary_one_and_the_password_is_kept() {
    let fixture = Fixture::start();
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(MemoryStore::default());
    let mut harness = window(dir.path(), &fixture, store.clone());
    open_session(&mut harness);
    until(&mut harness, "the password prompt", asks_password);
    assert!(!shows(&harness, "saved"));
    assert!(!shows(&harness, "credential store"));
    enter_password(&mut harness, fixture::PASSWORD);
    until(&mut harness, "the Session tab", has_tab);
    assert_eq!(
        store.secret(&key(Kind::Password)).as_deref(),
        Some(fixture::PASSWORD)
    );
}

#[test]
fn a_rejected_kept_password_says_so_once_then_asks_plainly() {
    let fixture = Fixture::start();
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(MemoryStore::default());
    store.set(&key(Kind::Password), "old").unwrap();
    let mut harness = window(dir.path(), &fixture, store.clone());
    open_session(&mut harness);
    until(&mut harness, "the saved-password line", |h| {
        shows(
            h,
            "The saved password was rejected. Enter it to open this Session.",
        )
    });
    enter_password(&mut harness, "wrong");
    until(&mut harness, "the plain rejection", |h| {
        shows(h, "The password was rejected.") && !shows(h, "The saved password")
    });
    assert_eq!(store.secret(&key(Kind::Password)).as_deref(), Some("old"));
    enter_password(&mut harness, fixture::PASSWORD);
    until(&mut harness, "the Session tab", has_tab);
    assert_eq!(
        store.secret(&key(Kind::Password)).as_deref(),
        Some(fixture::PASSWORD)
    );
}

#[test]
fn a_store_that_returns_nothing_says_so_and_remember_stays_on() {
    let fixture = Fixture::start();
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(MemoryStore::default());
    store.fail_get(true);
    let mut harness = window(dir.path(), &fixture, store);
    open_session(&mut harness);
    until(&mut harness, "the store line", |h| {
        shows(h, credentials::UNAVAILABLE)
    });
    enter_password(&mut harness, fixture::PASSWORD);
    until(&mut harness, "the Session tab", has_tab);
    let hosts = HostStore::beside(&dir.path().join("settings.toml")).load();
    assert!(hosts[0].remember);
}

#[test]
fn a_refused_write_shows_a_note_the_operator_can_dismiss() {
    let fixture = Fixture::start();
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(MemoryStore::default());
    store.fail_set(true);
    let mut harness = window(dir.path(), &fixture, store);
    open_session(&mut harness);
    until(&mut harness, "the password prompt", asks_password);
    enter_password(&mut harness, fixture::PASSWORD);
    until(&mut harness, "the Session tab", has_tab);
    assert!(shows(&harness, credentials::NOT_KEPT));
    press(&mut harness, "Dismiss");
    assert!(!shows(&harness, credentials::NOT_KEPT));
}

#[test]
fn saving_with_remember_off_removes_the_kept_secrets() {
    let fixture = Fixture::start();
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(MemoryStore::default());
    store.set(&key(Kind::Password), "kept").unwrap();
    store.set(&key(Kind::Passphrase), "kept").unwrap();
    let mut harness = window(dir.path(), &fixture, store.clone());
    harness
        .get_by_role_and_label(Role::Switch, "Remember")
        .click();
    harness.run_ok();
    press(&mut harness, "Save");
    until(&mut harness, "the secrets to go", |_| {
        store.secret(&key(Kind::Password)).is_none()
            && store.secret(&key(Kind::Passphrase)).is_none()
    });
}

#[test]
fn renaming_the_host_keeps_the_secret() {
    let fixture = Fixture::start();
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(MemoryStore::default());
    store.set(&key(Kind::Password), "kept").unwrap();
    let mut harness = window(dir.path(), &fixture, store.clone());
    let field = harness.get_by_role_and_label(Role::TextInput, "Display name");
    field.focus();
    harness.run_ok();
    harness
        .get_by_role_and_label(Role::TextInput, "Display name")
        .type_text(" 2");
    harness.run_ok();
    press(&mut harness, "Save");
    for _ in 0..20 {
        std::thread::sleep(Duration::from_millis(10));
        harness.step();
    }
    let hosts = HostStore::beside(&dir.path().join("settings.toml")).load();
    assert_eq!(hosts[0].display_name, "Fixture 2");
    assert_eq!(hosts[0].id, ID);
    assert_eq!(store.secret(&key(Kind::Password)).as_deref(), Some("kept"));
}

#[test]
fn a_failed_removal_still_deletes_the_host_and_says_so() {
    let fixture = Fixture::start();
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(MemoryStore::default());
    store.set(&key(Kind::Password), "kept").unwrap();
    store.fail_delete(true);
    let mut harness = window(dir.path(), &fixture, store);
    press(&mut harness, "Delete");
    until(&mut harness, "the removal line", |h| {
        shows(h, credentials::NOT_REMOVED)
    });
    let hosts = HostStore::beside(&dir.path().join("settings.toml")).load();
    assert!(hosts.is_empty());
}

#[test]
fn a_host_saved_before_it_had_an_id_gets_one_at_load() {
    let dir = tempfile::tempdir().unwrap();
    let settings = Store::at(dir.path().join("settings.toml"));
    settings.save(&AppSettings::default()).unwrap();
    let hosts = HostStore::beside(settings.path());
    hosts
        .save(&[Host {
            address: "old".into(),
            ..Host::default()
        }])
        .unwrap();
    let _app = App::with_store(settings);
    let loaded = hosts.load();
    assert!(!loaded[0].id.is_empty());
}
