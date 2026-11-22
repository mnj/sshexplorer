//! Opening a Session from the window: Hosts, the Host dialog, the trust and password
//! prompts, the Session tab with its Folder view, and Ctrl+W, against the test server.

mod common;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use common::fixture::{self, Fixture};
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use sshexplorer::app::{self, App};

fn window(known_hosts: PathBuf) -> Harness<'static, App> {
    Harness::builder()
        .with_size(egui::vec2(1000.0, 700.0))
        .with_context_setup(app::setup)
        .build_ui_state(
            |ui, app: &mut App| app.ui(ui),
            App::with_known_hosts(known_hosts),
        )
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

fn has_label(harness: &Harness<'static, App>, label: &str) -> bool {
    harness.query_by_label_contains(label).is_some()
}

/// Focuses the field of `role` named `label` and types `text` into it.
fn type_into(harness: &mut Harness<'static, App>, role: Role, label: &str, text: &str) {
    harness.get_by_role_and_label(role, label).focus();
    harness.run_ok();
    harness.get_by_role_and_label(role, label).type_text(text);
    harness.run_ok();
}

/// Presses Hosts and fills in the Host dialog for the fixture, then presses Open Session.
fn open_session(harness: &mut Harness<'static, App>, fixture: &Fixture) {
    harness.get_by_role_and_label(Role::Button, "Hosts").click();
    harness.run_ok();
    type_into(harness, Role::TextInput, "Address", fixture.address());
    type_into(
        harness,
        Role::TextInput,
        "Port",
        &fixture.port().to_string(),
    );
    type_into(harness, Role::TextInput, "Account", fixture::ACCOUNT);
    harness
        .get_by_role_and_label(Role::Button, "Open Session")
        .click();
    harness.run_ok();
}

fn enter_password(harness: &mut Harness<'static, App>, password: &str) {
    until(harness, "the password prompt", |h| {
        h.query_by_role(Role::PasswordInput).is_some()
    });
    type_into(harness, Role::PasswordInput, "Password", password);
    // The field shows dots, never the password.
    let field = harness.get_by_role(Role::PasswordInput);
    assert!(!field.value().unwrap_or_default().contains(password));
    harness
        .get_by_role_and_label(Role::Button, "Connect")
        .click();
    harness.run_ok();
}

#[test]
fn hosts_opens_a_session_that_lists_home_in_folder_view_and_ctrl_w_closes_it() {
    let fixture = Fixture::start();
    let dir = tempfile::tempdir().unwrap();
    let known_hosts = dir.path().join("known_hosts");
    let mut harness = window(known_hosts.clone());

    open_session(&mut harness, &fixture);

    // The unknown key's prompt names `[address]:port` and the fingerprint.
    until(&mut harness, "the trust prompt", |h| {
        h.query_by_role_and_label(Role::Button, "Trust").is_some()
    });
    assert!(has_label(
        &harness,
        &format!("[127.0.0.1]:{}", fixture.port())
    ));
    assert!(has_label(&harness, fixture::HOST_KEY_FINGERPRINT));
    harness.get_by_role_and_label(Role::Button, "Trust").click();
    harness.run_ok();

    // A wrong password asks again, saying so.
    enter_password(&mut harness, "wrong");
    until(&mut harness, "the rejection", |h| {
        has_label(h, "The password was rejected.")
    });
    enter_password(&mut harness, fixture::PASSWORD);

    let label = format!("{}@127.0.0.1:{}", fixture::ACCOUNT, fixture.port());
    until(&mut harness, "the Session tab", |h| {
        h.query_by_role_and_label(Role::Tab, &label).is_some()
    });
    // Folder view: `.` and `..`, then the home directory's entries; hidden ones are not shown.
    until(&mut harness, "the listing", |h| {
        h.query_by_label("notes.txt").is_some()
    });
    for name in [".", ".."].iter().chain(fixture::HOME_ENTRIES) {
        assert!(harness.query_by_label(name).is_some(), "{name} is listed");
    }
    assert!(harness.query_by_label(".profile").is_none());
    assert!(
        std::fs::read_to_string(&known_hosts)
            .unwrap()
            .contains(&fixture.known_hosts_line())
    );

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::W);
    harness.run_ok();
    assert!(harness.query_by_role(Role::Tab).is_none());
    assert!(harness.query_by_label("notes.txt").is_none());
}

#[test]
fn cancelling_the_password_opens_no_session() {
    let fixture = Fixture::start();
    let dir = tempfile::tempdir().unwrap();
    let known_hosts = dir.path().join("known_hosts");
    std::fs::write(&known_hosts, format!("{}\n", fixture.known_hosts_line())).unwrap();
    let mut harness = window(known_hosts);

    open_session(&mut harness, &fixture);
    until(&mut harness, "the password prompt", |h| {
        h.query_by_role(Role::PasswordInput).is_some()
    });
    harness
        .get_by_role_and_label(Role::Button, "Cancel")
        .click();
    harness.run_ok();
    // Give a wrongly opened Session time to arrive.
    for _ in 0..20 {
        std::thread::sleep(Duration::from_millis(10));
        harness.step();
    }
    assert!(harness.query_by_role(Role::PasswordInput).is_none());
    assert!(harness.query_by_role(Role::Tab).is_none());
}

#[test]
fn a_changed_key_shows_both_fingerprints_and_the_line_and_opens_no_session() {
    let fixture = Fixture::start();
    let dir = tempfile::tempdir().unwrap();
    let known_hosts = dir.path().join("known_hosts");
    let recorded = fixture.known_hosts_line_with(fixture::OTHER_KEY);
    std::fs::write(&known_hosts, format!("{recorded}\n")).unwrap();
    let mut harness = window(known_hosts);

    open_session(&mut harness, &fixture);
    until(&mut harness, "the refusal", |h| has_label(h, "has changed"));
    assert!(has_label(&harness, fixture::HOST_KEY_FINGERPRINT));
    assert!(has_label(&harness, fixture::OTHER_KEY_FINGERPRINT));
    assert!(has_label(&harness, &recorded));
    assert!(
        harness
            .query_by_role_and_label(Role::Button, "Trust")
            .is_none()
    );
    harness.get_by_role_and_label(Role::Button, "OK").click();
    harness.run_ok();
    assert!(!has_label(&harness, "has changed"));
    assert!(harness.query_by_role(Role::Tab).is_none());
}

#[test]
fn a_port_out_of_range_disables_open_session() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(dir.path().join("known_hosts"));
    harness.get_by_role_and_label(Role::Button, "Hosts").click();
    harness.run_ok();
    type_into(&mut harness, Role::TextInput, "Address", "example.org");
    type_into(&mut harness, Role::TextInput, "Port", "70000");
    harness.run_ok();
    let open = harness.get_by_role_and_label(Role::Button, "Open Session");
    assert!(open.accesskit_node().is_disabled());
    // Escape dismisses the dialog without opening anything.
    harness.key_press(egui::Key::Escape);
    harness.run_ok();
    assert!(
        harness
            .query_by_role_and_label(Role::TextInput, "Address")
            .is_none()
    );
    assert!(harness.query_by_role(Role::Tab).is_none());
}
