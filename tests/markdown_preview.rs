//! Markdown in the Preview panel, against the test server: rendered by default, the
//! Rendered / Source switch, images as their alt text, and which links leave the app.

mod common;

use std::time::{Duration, Instant};

use common::fixture::{self, Fixture};
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use sshexplorer::app::{self, App};
use sshexplorer::settings::AppSettings;

const HOME: &str = "home/tester";

const README: &str = "# Project title\n\
\n\
Read ![the build badge](badge.svg) and the [guide](docs/guide.md) or the [site](https://example.com/docs).\n\
\n\
```rust\n\
fn main() {}\n\
```\n";

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

/// A window with a Session open on the fixture's home, in Folder view.
fn browsing(fixture: &Fixture, dir: &tempfile::TempDir) -> Harness<'static, App> {
    let known_hosts = dir.path().join("known_hosts");
    std::fs::write(&known_hosts, format!("{}\n", fixture.known_hosts_line())).unwrap();
    let app = App::with_settings(AppSettings::default())
        .known_hosts_at(known_hosts)
        .agent_at(None);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 700.0))
        .with_context_setup(app::setup)
        .build_ui_state(|ui, app: &mut App| app.ui(ui), app);
    harness.get_by_role_and_label(Role::Button, "Hosts").click();
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
    until(&mut harness, "the tree", |h| {
        h.query_by_role_and_label(Role::Row, "tester").is_some()
    });
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Num2);
    harness.run_ok();
    until(&mut harness, "the listing", |h| {
        h.query_by_role_and_label(Role::Row, "GUIDE.md").is_some()
    });
    harness
}

fn has_label(harness: &Harness<'static, App>, label: &str) -> bool {
    harness.query_by_label(label).is_some()
}

fn has_label_containing(harness: &Harness<'static, App>, part: &str) -> bool {
    harness.query_by_label_contains(part).is_some()
}

/// The URLs the last frame asked the system to open.
fn opened(harness: &Harness<'static, App>) -> Vec<String> {
    harness
        .output()
        .platform_output
        .commands
        .iter()
        .filter_map(|c| match c {
            egui::OutputCommand::OpenUrl(open) => Some(open.url.clone()),
            _ => None,
        })
        .collect()
}

/// The file is not named `README.md`: the fixture's home already holds `readme.md`, and
/// the two are one file on a case-insensitive disk (macOS, Windows).
fn with_readme() -> (Fixture, tempfile::TempDir, Harness<'static, App>) {
    let fixture = Fixture::start();
    std::fs::write(fixture.root().join(HOME).join("GUIDE.md"), README).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut harness = browsing(&fixture, &dir);
    harness.get_by_role_and_label(Role::Row, "GUIDE.md").click();
    harness.run_ok();
    until(&mut harness, "the rendered Markdown", |h| {
        has_label(h, "Project title")
    });
    (fixture, dir, harness)
}

#[test]
fn markdown_shows_rendered_and_the_switch_shows_its_source() {
    let (_fixture, _dir, mut harness) = with_readme();
    // Rendered: the heading without its marks, the image as its alt text, and the code
    // block's code.
    assert!(!has_label(&harness, "# Project title"));
    assert!(has_label_containing(&harness, "the build badge"));
    assert!(!has_label_containing(&harness, "badge.svg"));
    assert!(
        harness.get_all_by_role(Role::MultilineTextInput).any(|n| n
            .accesskit_node()
            .value()
            .as_deref()
            == Some("fn main() {}"))
    );

    harness
        .get_by_role_and_label(Role::RadioButton, "Source")
        .click();
    harness.run_ok();
    until(&mut harness, "the source", |h| {
        has_label(h, "# Project title")
    });
    assert!(has_label(&harness, "```rust"));
    assert!(!has_label(&harness, "Project title"));

    harness
        .get_by_role_and_label(Role::RadioButton, "Rendered")
        .click();
    harness.run_ok();
    assert!(has_label(&harness, "Project title"));
}

#[test]
fn a_web_link_opens_in_the_browser_and_a_relative_link_does_nothing() {
    let (_fixture, _dir, mut harness) = with_readme();

    harness.get_by_role_and_label(Role::Link, "guide").click();
    harness.step();
    assert_eq!(opened(&harness), Vec::<String>::new());
    harness.run_ok();

    harness.get_by_role_and_label(Role::Link, "site").click();
    harness.step();
    assert_eq!(opened(&harness), ["https://example.com/docs"]);
}

#[test]
fn the_move_keys_scroll_the_rendered_view() {
    let fixture = Fixture::start();
    let home = fixture.root().join(HOME);
    std::fs::write(home.join("GUIDE.md"), README).unwrap();
    let long: String = (1..=200).map(|i| format!("Paragraph {i}\n\n")).collect();
    std::fs::write(home.join("long.md"), long).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut harness = browsing(&fixture, &dir);
    harness.get_by_role_and_label(Role::Row, "long.md").click();
    harness.run_ok();
    until(&mut harness, "the rendered Markdown", |h| {
        has_label(h, "Paragraph 1")
    });
    harness.key_press(egui::Key::Enter);
    harness.run_ok();
    assert!(!has_label(&harness, "Paragraph 200"));
    harness.key_press(egui::Key::End);
    harness.run_ok();
    assert!(has_label(&harness, "Paragraph 200"));
    assert!(!has_label(&harness, "Paragraph 1"));
    harness.key_press(egui::Key::Home);
    harness.run_ok();
    assert!(has_label(&harness, "Paragraph 1"));
}
