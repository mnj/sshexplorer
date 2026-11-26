//! The Preview panel in the window, against the test server: file properties, text
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
    // A Session opens in Tree view, at the home directory.
    until(&mut harness, "the tree", |h| {
        h.query_by_role_and_label(Role::Row, "tester").is_some()
    });
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

fn properties(h: &mut Harness<'static, App>, name: &str) {
    h.get_by_role_and_label(Role::Row, name).click_secondary();
    h.run_ok();
    h.get_by_role_and_label(Role::Button, "Properties").click();
    h.run_ok();
}

fn close_properties(h: &mut Harness<'static, App>) {
    h.get_by_role_and_label(Role::Button, "Close").click();
    h.run_ok();
}

fn press(harness: &mut Harness<'static, App>, key: egui::Key) {
    harness.key_press(key);
    harness.run_ok();
}

/// Whether file properties shows `value` beside `name`.
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
fn the_preview_shows_contents_and_properties_describes_the_clicked_entry() {
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
    assert!(!detail(&harness, "Name", "todo.txt"));
    assert!(has_label(&harness, "Loading preview…"));
    until(&mut harness, "the text", |h| has_label(h, "second line"));
    assert!(has_label(&harness, "first line"));
    assert!(!has_label(&harness, "Loading preview…"));

    properties(&mut harness, "todo.txt");
    assert!(detail(&harness, "Path", "/home/tester/todo.txt"));
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
    let header = harness.get_by_role_and_label(Role::Button, "Close").rect();
    let filename = harness
        .get_by(|n| {
            n.label().as_deref() == Some("Name") && n.value().as_deref() == Some("todo.txt")
        })
        .rect();
    assert!(header.bottom() <= filename.top());
    for (column, permission) in [
        ("Read", "Owner · Read: yes"),
        ("Write", "Owner · Write: yes"),
        ("Exec", "Owner · Execute: no"),
    ] {
        let title = harness.get_by_label(column).rect();
        let cell = harness.get_by_label(permission).rect();
        assert!((title.center().x - cell.center().x).abs() < 2.0);
    }
    harness
        .get_by_role_and_label(Role::Button, "Copy path")
        .click();
    harness.step();
    assert!(harness.output().platform_output.commands.iter().any(
        |c| matches!(c, egui::OutputCommand::CopyText(path) if path == "/home/tester/todo.txt")
    ));
    close_properties(&mut harness);

    properties(&mut harness, "docs");
    assert!(detail(&harness, "Kind", "Directory"));
    assert!(has_label(&harness, "first line"));
    close_properties(&mut harness);
    // A directory has no preview body.
    click_row(&mut harness, "docs");
    assert!(!has_label(&harness, "second line"));
    for _ in 0..30 {
        std::thread::sleep(Duration::from_millis(10));
        harness.step();
    }
    assert!(!has_label(&harness, "Loading preview…"));
    properties(&mut harness, "..");
    assert!(detail(&harness, "Name", ".."));
    assert!(detail(&harness, "Kind", "Directory"));
}

#[test]
fn long_paths_wrap_and_access_remains_reachable_in_a_short_window() {
    let fixture = Fixture::start();
    let name = format!("{}.txt", "x".repeat(150));
    std::fs::write(fixture.root().join(HOME).join(&name), "test").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut h = browsing(&fixture, &dir, AppSettings::default());
    properties(&mut h, &name);
    h.set_size(egui::vec2(480.0, 420.0));
    h.run_ok();
    let path = format!("/home/tester/{name}");
    let path_rect = h
        .get_by(|n| n.label().as_deref() == Some("Path") && n.value().as_deref() == Some(&path))
        .rect();
    assert!(path_rect.left() >= 0.0 && path_rect.right() <= 480.0);
    assert!(
        path_rect.height() > 30.0,
        "long path should wrap: {path_rect:?}"
    );
    let last = "Others · Execute: no";
    let before = h.get_by_label(last).rect();
    let point = egui::pos2(path_rect.center().x, 210.0);
    h.hover_at(point);
    h.run_ok();
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Page,
        delta: egui::vec2(0.0, -3.0),
        modifiers: egui::Modifiers::NONE,
        source: egui::MouseWheelSource::Wheel,
        phase: egui::TouchPhase::Move,
    });
    h.run_steps(20);
    let bottom_row = h.get_by_label(last).rect();
    assert!(
        bottom_row.top() > 0.0 && bottom_row.bottom() < 420.0,
        "path {path_rect:?}, last row {before:?} → {bottom_row:?}, wheel at {point:?}"
    );
    close_properties(&mut h);
}

#[cfg(unix)]
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

    until(&mut harness, "the source", |h| has_label(h, "fn main() {"));
    assert!(has_label(&harness, "    println!(\"hi\");"));

    click_row(&mut harness, "blob");
    until(&mut harness, "the verdict", |h| {
        has_label(h, "No preview for this file type.")
    });

    properties(&mut harness, "current");
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

#[cfg(unix)]
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
    assert_ne!(
        harness
            .get_by_role_and_label(Role::Row, "long.txt")
            .accesskit_node()
            .is_selected(),
        Some(true)
    );

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

#[test]
fn latest_and_follow_handle_append_truncation_and_rotation() {
    use std::io::Write as _;
    let fixture = Fixture::start();
    let path = fixture.root().join(HOME).join("active.log");
    std::fs::write(
        &path,
        format!("first record\n{}\nlast record\n", "x".repeat(70_000)),
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut h = browsing(&fixture, &dir, AppSettings::default());
    click_row(&mut h, "active.log");
    h.get_by_role_and_label(Role::Button, "Show latest").click();
    h.run_ok();
    until(&mut h, "the log tail", |h| has_label(h, "last record"));
    assert!(!has_label(&h, "first record"));
    h.get_by_role_and_label(Role::Switch, "Follow").click();
    h.run_ok();
    writeln!(
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap(),
        "appended record"
    )
    .unwrap();
    until(&mut h, "appended text", |h| has_label(h, "appended record"));
    std::fs::write(&path, "truncated record\n").unwrap();
    until(&mut h, "truncation", |h| has_label(h, "truncated record"));
    assert!(!has_label(&h, "appended record"));
    std::fs::rename(&path, path.with_extension("old")).unwrap();
    std::fs::write(&path, "rotated log with more bytes\n").unwrap();
    until(&mut h, "replacement by path", |h| {
        has_label(h, "rotated log with more bytes")
    });
    assert!(!has_label(&h, "truncated record"));
    h.get_by_role_and_label(Role::Button, "Show beginning")
        .click();
    h.run_ok();
    until(&mut h, "normal preview", |h| {
        has_label(h, "rotated log with more bytes")
    });
    assert!(
        h.query_by_role_and_label(Role::Button, "Show beginning")
            .is_none()
    );
}
