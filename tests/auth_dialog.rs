//! Logging in from the window: the Host's Authentication method and Key file reach the
//! connection, the config's `IdentityAgent` picks the agent, and the secret dialog asks
//! for a key passphrase or each prompt of a keyboard-interactive round.

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::fixture::{self, Accepts, Fixture, Keyboard};
use common::keys::{PASSPHRASE, key_path, public_key};
use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use sshexplorer::app::{self, App};
use sshexplorer::hosts::{AuthMethod, Host, HostStore};
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
            start.elapsed() < Duration::from_secs(20),
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(10));
        harness.step();
    }
    harness.run_ok();
}

fn press(harness: &mut Harness<'static, App>, button: &str) {
    harness.get_by_role_and_label(Role::Button, button).click();
    harness.run_ok();
}

fn type_into(harness: &mut Harness<'static, App>, role: Role, label: &str, text: &str) {
    harness.get_by_role_and_label(role, label).focus();
    harness.run_ok();
    harness.get_by_role_and_label(role, label).type_text(text);
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

/// A saved Host "Fixture" for `fixture`, logging in by `auth` with `key_file`.
fn host(fixture: &Fixture, auth: AuthMethod, key_file: &Path) -> Host {
    Host {
        display_name: "Fixture".into(),
        address: fixture.address().into(),
        port: Some(fixture.port()),
        account: fixture::ACCOUNT.into(),
        auth,
        key_file: key_file.to_string_lossy().into_owned(),
        ..Host::default()
    }
}

/// A window with `host` saved, the fixture's key known and `config` as the OpenSSH
/// config, reaching the agent at `agent` by default. Ctrl+O and Enter open the Host.
fn open(
    dir: &Path,
    fixture: &Fixture,
    host: Host,
    config: &str,
    agent: Option<PathBuf>,
) -> Harness<'static, App> {
    let store = Store::at(dir.join("settings.toml"));
    store.save(&AppSettings::default()).unwrap();
    HostStore::beside(store.path()).save(&[host]).unwrap();
    let known_hosts = dir.join("known_hosts");
    std::fs::write(&known_hosts, format!("{}\n", fixture.known_hosts_line())).unwrap();
    let config_path = dir.join("ssh_config");
    std::fs::write(&config_path, config).unwrap();
    let app = App::with_store(store)
        .known_hosts_at(known_hosts)
        .ssh_config_at(config_path)
        .agent_at(agent);
    let mut harness = window(app);
    harness.key_press_modifiers(Modifiers::COMMAND, Key::O);
    harness.run_ok();
    harness.key_press(Key::Enter);
    harness.run_ok();
    harness
}

#[test]
fn a_locked_key_file_asks_for_its_passphrase_until_it_unlocks() {
    let fixture = Fixture::start_accepting(Accepts {
        keys: vec![public_key("openssh_ed25519_locked")],
        ..Accepts::default()
    });
    let dir = tempfile::tempdir().unwrap();
    let key = key_path("openssh_ed25519_locked");
    let mut harness = open(
        dir.path(),
        &fixture,
        host(&fixture, AuthMethod::KeyFile, &key),
        "",
        None,
    );
    until(&mut harness, "the passphrase prompt", |h| {
        h.query_by_role_and_label(Role::PasswordInput, "Passphrase")
            .is_some()
    });
    assert!(shows(&harness, &key.display().to_string()));
    type_into(&mut harness, Role::PasswordInput, "Passphrase", "wrong");
    press(&mut harness, "Connect");
    until(&mut harness, "the rejection", |h| {
        shows(h, "The key passphrase was rejected.")
    });
    type_into(&mut harness, Role::PasswordInput, "Passphrase", PASSPHRASE);
    harness.key_press(Key::Enter);
    harness.run_ok();
    until(&mut harness, "the Session tab", has_tab);
}

#[test]
fn a_round_of_several_prompts_shows_each_in_the_secret_dialog() {
    let fixture = Fixture::start_accepting(Accepts {
        keyboard: Keyboard::PasswordAndCode,
        ..Accepts::default()
    });
    let dir = tempfile::tempdir().unwrap();
    let mut harness = open(
        dir.path(),
        &fixture,
        host(&fixture, AuthMethod::Automatic, Path::new("")),
        "",
        None,
    );
    until(&mut harness, "the prompts", |h| {
        h.query_by_role_and_label(Role::TextInput, "Verification code")
            .is_some()
    });
    type_into(
        &mut harness,
        Role::PasswordInput,
        "Password",
        fixture::PASSWORD,
    );
    type_into(
        &mut harness,
        Role::TextInput,
        "Verification code",
        fixture::CODE,
    );
    press(&mut harness, "Connect");
    until(&mut harness, "the Session tab", has_tab);
}

#[cfg(unix)]
#[test]
fn identity_agent_none_leaves_the_agent_method_without_an_agent() {
    // The default agent holds the key the host takes, but the config turns it off.
    let held = common::keys::TestAgent::holding_keys(&["openssh_ecdsa"]);
    let fixture = Fixture::start_accepting(Accepts {
        keys: vec![public_key("openssh_ecdsa")],
        ..Accepts::default()
    });
    let dir = tempfile::tempdir().unwrap();
    let mut harness = open(
        dir.path(),
        &fixture,
        host(&fixture, AuthMethod::Agent, Path::new("")),
        "Host *\n  IdentityAgent none\n",
        Some(held.socket()),
    );
    until(&mut harness, "the failure", |h| {
        shows(h, "No SSH agent is running.")
    });
}

#[cfg(unix)]
#[test]
fn identity_agent_names_the_agent_to_use() {
    let held = common::keys::TestAgent::holding_keys(&["openssh_ecdsa"]);
    let fixture = Fixture::start_accepting(Accepts {
        keys: vec![public_key("openssh_ecdsa")],
        ..Accepts::default()
    });
    let dir = tempfile::tempdir().unwrap();
    let config = format!("Host *\n  IdentityAgent {}\n", held.socket().display());
    let mut harness = open(
        dir.path(),
        &fixture,
        host(&fixture, AuthMethod::Agent, Path::new("")),
        &config,
        None,
    );
    until(&mut harness, "the Session tab", has_tab);
}
