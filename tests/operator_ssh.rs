//! Only the app itself reads the operator's own `~/.ssh`: an App built as tests build
//! it reads no OpenSSH config, while [`App::operator_ssh`], which main.rs calls, does.
//!
//! The test runs itself again in a child process whose HOME is a temporary home.

use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use sshexplorer::app::{self, App};

fn window(app: App) -> Harness<'static, App> {
    Harness::builder()
        .with_size(egui::vec2(1100.0, 800.0))
        .with_context_setup(app::setup)
        .build_ui_state(|ui, app: &mut App| app.ui(ui), app)
}

/// The Port placeholder once the Host dialog's Address reads `web`.
fn port_hint(app: App) -> String {
    let mut harness = window(app);
    harness.get_by_role_and_label(Role::Button, "Hosts").click();
    harness.run_ok();
    harness
        .get_by_role_and_label(Role::TextInput, "Address")
        .focus();
    harness.run_ok();
    harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
    harness.key_press(Key::Backspace);
    harness.run_ok();
    harness
        .get_by_role_and_label(Role::TextInput, "Address")
        .type_text("web");
    harness.run_ok();
    harness
        .get_by_role_and_label(Role::TextInput, "Port")
        .accesskit_node()
        .placeholder()
        .unwrap_or_default()
        .to_owned()
}

/// Set in the child run, whose HOME is the temporary home.
const CHILD: &str = "SSHEXPLORER_OPERATOR_SSH_CHILD";

#[test]
fn a_test_built_app_reads_no_config_from_the_operators_home() {
    if std::env::var_os(CHILD).is_none() {
        let home = tempfile::tempdir().unwrap();
        let ssh = home.path().join(".ssh");
        std::fs::create_dir_all(&ssh).unwrap();
        std::fs::write(ssh.join("config"), "Host web\n  Port 2222\n").unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "a_test_built_app_reads_no_config_from_the_operators_home",
                "--exact",
                "--test-threads=1",
            ])
            .env("HOME", home.path())
            .env(CHILD, "1")
            .status()
            .unwrap();
        assert!(status.success(), "the run with the temporary home failed");
        return;
    }

    assert_eq!(port_hint(App::default()), "22");
    assert_eq!(port_hint(App::with_settings(Default::default())), "22");
    // Not vacuous: the operator's own config, here the temporary home's, is read.
    assert_eq!(
        port_hint(App::default().operator_ssh()),
        "2222 (from ~/.ssh/config)"
    );
}
