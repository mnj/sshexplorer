//! The OpenSSH config in the window: the Host dialog's placeholders and Overrides hints,
//! Sessions opened through a config alias, a proxy the config asks for, an unreadable
//! config, and the "Use ~/.ssh/config" setting on Settings → Connections.

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::fixture::{self, Fixture};
use egui::accesskit::{Role, Toggled};
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use sshexplorer::app::{self, App};
use sshexplorer::settings::{AppSettings, Store};

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

fn placeholder(harness: &Harness<'static, App>, label: &str) -> String {
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .accesskit_node()
        .placeholder()
        .unwrap_or_default()
        .to_owned()
}

fn press(harness: &mut Harness<'static, App>, button: &str) {
    harness.get_by_role_and_label(Role::Button, button).click();
    harness.run_ok();
}

fn shows(harness: &Harness<'static, App>, text: &str) -> bool {
    harness.query_by_label(text).is_some()
}

/// A settings store, `known_hosts` and `~/.ssh/config` in `dir`, the config holding
/// `config`.
fn app_in(dir: &Path, config: &str, settings: AppSettings) -> (App, PathBuf) {
    let store = Store::at(dir.join("settings.toml"));
    store.save(&settings).unwrap();
    let config_path = dir.join("ssh_config");
    std::fs::write(&config_path, config).unwrap();
    let app = App::with_store(store)
        .known_hosts_at(dir.join("known_hosts"))
        .ssh_config_at(config_path.clone());
    (app, config_path)
}

const WEB: &str = "Host web\n  Port 2222\n  User deploy\n  IdentityFile ~/.ssh/id_web\n";

#[test]
fn empty_fields_show_the_config_values_for_the_address_as_it_is_typed() {
    let dir = tempfile::tempdir().unwrap();
    let (app, _) = app_in(dir.path(), WEB, AppSettings::default());
    let mut harness = window(app);
    press(&mut harness, "Hosts");

    fill(&mut harness, "Address", "web");
    assert_eq!(placeholder(&harness, "Port"), "2222 (from ~/.ssh/config)");
    assert_eq!(
        placeholder(&harness, "Account"),
        "deploy (from ~/.ssh/config)"
    );
    assert_eq!(
        placeholder(&harness, "Key file"),
        "~/.ssh/id_web (from ~/.ssh/config)"
    );

    fill(&mut harness, "Address", "db");
    assert_eq!(placeholder(&harness, "Port"), "22");
    assert!(!placeholder(&harness, "Account").contains("from"));
    assert_eq!(placeholder(&harness, "Key file"), "");
}

#[test]
fn a_filled_port_or_account_that_differs_from_the_config_says_it_overrides_it() {
    let dir = tempfile::tempdir().unwrap();
    let (app, _) = app_in(dir.path(), WEB, AppSettings::default());
    let mut harness = window(app);
    press(&mut harness, "Hosts");
    fill(&mut harness, "Address", "web");
    fill(&mut harness, "Port", "2222");
    fill(&mut harness, "Account", "deploy");
    assert!(!shows(&harness, "Overrides 2222 from ~/.ssh/config"));
    assert!(!shows(&harness, "Overrides deploy from ~/.ssh/config"));

    fill(&mut harness, "Port", "2200");
    fill(&mut harness, "Account", "root");
    assert!(shows(&harness, "Overrides 2222 from ~/.ssh/config"));
    assert!(shows(&harness, "Overrides deploy from ~/.ssh/config"));
    // It never blocks Save.
    let save = harness.get_by_role_and_label(Role::Button, "Save");
    assert!(!save.accesskit_node().is_disabled());
}

#[test]
fn with_the_setting_off_the_config_supplies_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut settings = AppSettings::default();
    settings.connections.use_ssh_config = false;
    let (app, _) = app_in(dir.path(), WEB, settings);
    let mut harness = window(app);
    press(&mut harness, "Hosts");
    fill(&mut harness, "Address", "web");
    fill(&mut harness, "Port", "2200");
    assert!(!placeholder(&harness, "Account").contains("from"));
    assert_eq!(placeholder(&harness, "Key file"), "");
    assert!(!shows(&harness, "Overrides 2222 from ~/.ssh/config"));
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

/// Picks Unsaved Session in the open Host dialog.
fn unsaved_session(harness: &mut Harness<'static, App>) {
    harness
        .get_by_role_and_label(Role::ListItem, "Unsaved Session")
        .click();
    harness.run_ok();
}

#[test]
fn an_alias_opens_through_its_host_name_port_and_user_read_fresh_at_open() {
    let fixture = Fixture::start();
    let dir = tempfile::tempdir().unwrap();
    let (app, config) = app_in(dir.path(), "", AppSettings::default());
    // The key is recorded under the HostName, not the alias: no trust prompt follows.
    std::fs::write(
        dir.path().join("known_hosts"),
        format!("{}\n", fixture.known_hosts_line()),
    )
    .unwrap();
    let mut harness = window(app);
    press(&mut harness, "Hosts");
    unsaved_session(&mut harness);
    fill(&mut harness, "Address", "fx");
    // Written after the dialog opened: the Session reads the file as it is at open.
    std::fs::write(
        &config,
        format!(
            "Host fx\n  HostName {}\n  Port {}\n  User {}\n",
            fixture.address(),
            fixture.port(),
            fixture::ACCOUNT
        ),
    )
    .unwrap();
    press(&mut harness, "Open Session");
    enter_password(&mut harness);
    let label = format!("{}@fx:{}", fixture::ACCOUNT, fixture.port());
    until(&mut harness, "the Session tab", |h| {
        h.query_by_role_and_label(Role::Tab, &label).is_some()
    });
    until(&mut harness, "the listing", |h| shows(h, "notes.txt"));
}

#[test]
fn a_host_that_needs_proxy_jump_fails_naming_the_keyword() {
    let dir = tempfile::tempdir().unwrap();
    let (app, _) = app_in(
        dir.path(),
        "Host jump\n  ProxyJump bastion.example.org\n",
        AppSettings::default(),
    );
    let mut harness = window(app);
    press(&mut harness, "Hosts");
    unsaved_session(&mut harness);
    fill(&mut harness, "Address", "jump");
    press(&mut harness, "Open Session");
    until(&mut harness, "the failure", |h| {
        shows(
            h,
            "This host needs ProxyJump, which sshexplorer does not support yet.",
        )
    });
}

#[test]
fn an_unreadable_config_opens_from_the_host_fields_with_a_note_naming_file_and_line() {
    let fixture = Fixture::start();
    let dir = tempfile::tempdir().unwrap();
    let (app, config) = app_in(
        dir.path(),
        "Host *\n  User nobody\n  Port lots\n",
        AppSettings::default(),
    );
    std::fs::write(
        dir.path().join("known_hosts"),
        format!("{}\n", fixture.known_hosts_line()),
    )
    .unwrap();
    let mut harness = window(app);
    press(&mut harness, "Hosts");
    unsaved_session(&mut harness);
    fill(&mut harness, "Address", fixture.address());
    fill(&mut harness, "Port", &fixture.port().to_string());
    fill(&mut harness, "Account", fixture::ACCOUNT);
    press(&mut harness, "Open Session");
    enter_password(&mut harness);
    until(&mut harness, "the listing", |h| shows(h, "notes.txt"));
    let note = format!(
        "{} line 3: bad Port \"lots\". Opened from the Host fields alone.",
        config.display()
    );
    assert!(shows(&harness, &note), "{note}");
}

#[test]
fn settings_connections_turns_the_config_off_and_saves_it() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::at(dir.path().join("settings.toml"));
    let mut harness = window(App::with_store(store.clone()));
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Comma);
    harness.run_ok();
    harness
        .get_by_role_and_label(Role::ListItem, "Connections")
        .click();
    harness.run_ok();
    let switch = harness.get_by_role_and_label(Role::Switch, "Use ~/.ssh/config");
    assert_eq!(switch.accesskit_node().toggled(), Some(Toggled::True));
    switch.click();
    harness.run_ok();
    assert!(!store.load().connections.use_ssh_config);
}
