//! The Preview panel in the window, against the test server: the details header, text
//! and source bodies, the size cap, failures, and moving the keyboard to and from it.

mod common;

use std::time::{Duration, Instant};

use common::fixture::{self, Fixture};
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use sshexplorer::app::{self, App};
use sshexplorer::settings::AppSettings;

const HOME: &str = "home/tester";

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

/// A window with a Session open on the fixture's home, with `settings`.
fn browsing(
    fixture: &Fixture,
    dir: &tempfile::TempDir,
    settings: AppSettings,
) -> Harness<'static, App> {
    let known_hosts = dir.path().join("known_hosts");
    std::fs::write(&known_hosts, format!("{}\n", fixture.known_hosts_line())).unwrap();
    let app = App::with_settings(settings)
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
    // A Session opens in Tree view, at the home directory, which Preview describes.
    until(&mut harness, "the tree", |h| {
        h.query_by_role_and_label(Role::Row, "tester").is_some()
    });
    until(&mut harness, "the home's details", |h| {
        detail(h, "Name", "tester")
    });
    assert!(detail(&harness, "Kind", "Directory"));
    // The rest of these tests browse in Folder view.
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Num2);
    harness.run_ok();
    until(&mut harness, "the listing", |h| {
        h.query_by_role_and_label(Role::Row, "notes.txt").is_some()
    });
    harness
}

fn click_row(harness: &mut Harness<'static, App>, name: &str) {
    harness.get_by_role_and_label(Role::Row, name).click();
    harness.run_ok();
}

fn press(harness: &mut Harness<'static, App>, key: egui::Key) {
    harness.key_press(key);
    harness.run_ok();
}

/// Whether the details header shows `value` beside `name`.
fn detail(harness: &Harness<'static, App>, name: &str, value: &str) -> bool {
    harness
        .query_by(|n| n.value().as_deref() == Some(value) && n.label().as_deref() == Some(name))
        .is_some()
}

fn has_label(harness: &Harness<'static, App>, label: &str) -> bool {
    harness.query_by_label(label).is_some()
}

fn preview_focused(harness: &Harness<'static, App>) -> bool {
    harness.get_by_label("Preview").is_focused()
}

#[test]
fn a_focused_file_shows_its_details_and_then_its_text() {
    let fixture = Fixture::start();
    std::fs::write(
        fixture.root().join(HOME).join("todo.txt"),
        "first line\nsecond line\n",
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut harness = browsing(&fixture, &dir, AppSettings::default());

    harness.get_by_role_and_label(Role::Row, "todo.txt").click();
    harness.step();
    harness.step();
    // At once: the header, and the loading line until the bytes are fetched.
    assert!(detail(&harness, "Name", "todo.txt"));
    assert!(detail(&harness, "Kind", "Text"));
    assert!(detail(&harness, "Size", "23 B"));
    assert!(detail(&harness, "Permissions", "-rw-r--r--"));
    // By name, as the listing's `ls -l` line gives them, not the numeric ids.
    assert!(detail(&harness, "Owner", fixture::OWNER));
    assert!(detail(&harness, "Group", fixture::GROUP));
    let modified = |n: &egui_kittest::kittest::AccessKitNode<'_>| {
        n.label().as_deref() == Some("Modified") && n.value().is_some_and(|v| !v.is_empty())
    };
    assert!(harness.query_by(modified).is_some());
    assert!(has_label(&harness, "Loading preview…"));
    until(&mut harness, "the text", |h| has_label(h, "second line"));
    assert!(has_label(&harness, "first line"));
    assert!(!has_label(&harness, "Loading preview…"));

    // A directory shows the header only.
    click_row(&mut harness, "docs");
    assert!(detail(&harness, "Kind", "Directory"));
    assert!(!has_label(&harness, "second line"));
    for _ in 0..30 {
        std::thread::sleep(Duration::from_millis(10));
        harness.step();
    }
    assert!(!has_label(&harness, "Loading preview…"));
    click_row(&mut harness, "..");
    assert!(detail(&harness, "Name", ".."));
    assert!(detail(&harness, "Kind", "Directory"));
}

#[test]
fn source_shows_with_numbered_lines_and_an_unknown_binary_file_says_so() {
    let fixture = Fixture::start();
    let home = fixture.root().join(HOME);
    std::fs::write(
        home.join("main.rs"),
        "fn main() {\n    println!(\"hi\");\n}\n",
    )
    .unwrap();
    std::fs::write(home.join("blob"), [0u8, 159, 146, 150, 0, 1]).unwrap();
    std::os::unix::fs::symlink("main.rs", home.join("current")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut harness = browsing(&fixture, &dir, AppSettings::default());

    click_row(&mut harness, "main.rs");
    assert!(detail(&harness, "Kind", "Source"));
    until(&mut harness, "the source", |h| has_label(h, "fn main() {"));
    assert!(has_label(&harness, "    println!(\"hi\");"));

    click_row(&mut harness, "blob");
    until(&mut harness, "the verdict", |h| {
        has_label(h, "No preview for this file type.")
    });

    // A symlink names its target in the header.
    click_row(&mut harness, "current");
    assert!(detail(&harness, "Link to", "main.rs"));
}

#[test]
fn a_large_file_shows_the_first_part_under_a_banner() {
    let fixture = Fixture::start();
    let line = "0123456789abcdef".repeat(4) + "\n";
    let text = line.repeat((3 << 20) / line.len());
    std::fs::write(fixture.root().join(HOME).join("big.log"), &text).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut settings = AppSettings::default();
    settings.preview.size_cap = 1;
    let mut harness = browsing(&fixture, &dir, settings);

    click_row(&mut harness, "big.log");
    until(&mut harness, "the banner", |h| {
        h.query_by_label_contains("Showing the first 1 MiB of")
            .is_some()
    });
    assert!(has_label(&harness, "Showing the first 1 MiB of 3 MiB"));
}

#[test]
fn an_archive_lists_its_members_and_one_over_the_cap_says_it_is_too_large() {
    use std::io::Write as _;

    let fixture = Fixture::start();
    let home = fixture.root().join(HOME);
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::from_date_and_time(2024, 5, 1, 12, 30, 0).unwrap());
    writer.start_file("docs/guide.txt", options).unwrap();
    writer.write_all(b"hello").unwrap();
    std::fs::write(
        home.join("bundle.zip"),
        writer.finish().unwrap().into_inner(),
    )
    .unwrap();
    // Over the 1 MiB cap whatever is inside.
    std::fs::write(home.join("huge.tgz"), vec![0u8; 2 << 20]).unwrap();
    std::fs::write(home.join("broken.zip"), b"PK\x03\x04 and then nothing").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut settings = AppSettings::default();
    settings.preview.size_cap = 1;
    let mut harness = browsing(&fixture, &dir, settings);

    click_row(&mut harness, "bundle.zip");
    assert!(detail(&harness, "Kind", "Archive"));
    until(&mut harness, "the listing", |h| {
        h.query_by_label_contains("docs/guide.txt").is_some()
    });
    assert!(has_label(
        &harness,
        "       5 B  2024-05-01 12:30  docs/guide.txt"
    ));

    click_row(&mut harness, "huge.tgz");
    until(&mut harness, "the verdict", |h| {
        has_label(h, "Too large to preview (2 MiB)")
    });
    assert!(has_label(&harness, "Download it to open it locally."));

    click_row(&mut harness, "broken.zip");
    until(&mut harness, "the failure", |h| {
        has_label(h, "Cannot preview: corrupt archive")
    });
}

#[test]
fn a_file_that_cannot_be_read_shows_one_line() {
    use std::os::unix::fs::PermissionsExt as _;

    let fixture = Fixture::start();
    let secret = fixture.root().join(HOME).join("secret.txt");
    std::fs::write(&secret, "no").unwrap();
    std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(&secret).is_ok() {
        // Running as root: nothing is refused.
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mut harness = browsing(&fixture, &dir, AppSettings::default());
    click_row(&mut harness, "secret.txt");
    until(&mut harness, "the failure", |h| {
        has_label(h, "Cannot read: permission denied")
    });
    // No dialog.
    assert!(harness.query_by_role(Role::Dialog).is_none());
}

#[test]
fn enter_moves_the_keyboard_to_preview_escape_returns_and_tab_cycles() {
    let fixture = Fixture::start();
    let long: String = (1..=400).map(|i| format!("line {i}\n")).collect();
    std::fs::write(fixture.root().join(HOME).join("long.txt"), long).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut harness = browsing(&fixture, &dir, AppSettings::default());

    click_row(&mut harness, "long.txt");
    until(&mut harness, "the text", |h| has_label(h, "line 1"));
    assert!(!preview_focused(&harness));
    press(&mut harness, egui::Key::Enter);
    assert!(preview_focused(&harness));

    // The move keys scroll Preview, not the listing.
    assert!(!has_label(&harness, "line 400"));
    press(&mut harness, egui::Key::End);
    assert!(has_label(&harness, "line 400"));
    assert!(!has_label(&harness, "line 1"));
    press(&mut harness, egui::Key::Home);
    assert!(has_label(&harness, "line 1"));
    assert_eq!(
        harness
            .get_all_by_role(Role::Row)
            .filter(|r| r.accesskit_node().is_selected() == Some(true))
            .map(|r| r.accesskit_node().label().unwrap_or_default())
            .collect::<Vec<_>>(),
        ["long.txt"]
    );

    // Escape gives the keyboard back to the listing, where Up moves the focus.
    press(&mut harness, egui::Key::Escape);
    assert!(!preview_focused(&harness));
    press(&mut harness, egui::Key::ArrowUp);
    assert!(!detail(&harness, "Name", "long.txt"));

    // Tab goes to the search field, then Preview, and on round to the listing;
    // Shift+Tab goes back.
    let search_focused = |h: &Harness<'static, App>| {
        h.get_by_role_and_label(Role::TextInput, "Search")
            .is_focused()
    };
    press(&mut harness, egui::Key::Tab);
    assert!(search_focused(&harness));
    press(&mut harness, egui::Key::Tab);
    assert!(preview_focused(&harness));
    press(&mut harness, egui::Key::Tab);
    assert!(!preview_focused(&harness) && !search_focused(&harness));
    harness.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::Tab);
    harness.run_ok();
    assert!(preview_focused(&harness));
}
