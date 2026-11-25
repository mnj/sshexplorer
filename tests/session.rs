//! Opening a Session from the window: Hosts, the Host dialog, the trust and password
//! prompts, the Session tab with its Tree view and Folder view, and Ctrl+W, against the
//! test server.

mod common;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use common::fixture::{self, Fixture};
use egui::accesskit::{Role, Toggled};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use sshexplorer::app::{self, App};

fn window(known_hosts: PathBuf) -> Harness<'static, App> {
    Harness::builder()
        .with_size(egui::vec2(1000.0, 700.0))
        .with_context_setup(app::setup)
        .build_ui_state(
            |ui, app: &mut App| app.ui(ui),
            App::with_known_hosts(known_hosts).agent_at(None),
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
    // Tree view first, on the home directory; Ctrl+2 switches to Folder view.
    until(&mut harness, "the tree", |h| {
        h.query_by_role_and_label(Role::Row, "tester").is_some()
    });
    folder_view(&mut harness);
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

fn folder_view(harness: &mut Harness<'static, App>) {
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Num2);
    harness.run_ok();
}

/// A window with a Session open on the fixture's home in Tree view, its key already
/// trusted, once the tree has reached the home directory.
fn tree(fixture: &Fixture, dir: &tempfile::TempDir) -> Harness<'static, App> {
    let known_hosts = dir.path().join("known_hosts");
    std::fs::write(&known_hosts, format!("{}\n", fixture.known_hosts_line())).unwrap();
    let mut harness = window(known_hosts);
    open_session(&mut harness, fixture);
    enter_password(&mut harness, fixture::PASSWORD);
    until(&mut harness, "the tree", |h| selected_rows(h) == ["tester"]);
    harness
}

/// The same, switched to Folder view.
fn browsing(fixture: &Fixture, dir: &tempfile::TempDir) -> Harness<'static, App> {
    let mut harness = tree(fixture, dir);
    folder_view(&mut harness);
    until(&mut harness, "the listing", |h| {
        h.query_by_label("notes.txt").is_some()
    });
    harness
}

fn row_names(harness: &Harness<'static, App>) -> Vec<String> {
    let mut rows: Vec<_> = harness.get_all_by_role(Role::Row).collect();
    rows.sort_by(|a, b| a.rect().min.y.total_cmp(&b.rect().min.y));
    rows.iter()
        .map(|r| r.accesskit_node().label().unwrap_or_default())
        .collect()
}

fn selected_rows(harness: &Harness<'static, App>) -> Vec<String> {
    harness
        .query_all_by_role(Role::Row)
        .filter(|r| r.accesskit_node().is_selected() == Some(true))
        .map(|r| r.accesskit_node().label().unwrap_or_default())
        .collect()
}

fn press(harness: &mut Harness<'static, App>, key: egui::Key) {
    harness.key_press(key);
    harness.run_ok();
}

#[test]
fn folder_view_has_sortable_columns_and_lists_dots_and_directories_first() {
    let fixture = Fixture::start();
    std::fs::write(fixture.root().join("home/tester/file10"), "x").unwrap();
    std::fs::write(fixture.root().join("home/tester/file2"), "xyz").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut harness = browsing(&fixture, &dir);
    for column in ["Name", "Size", "Modified", "Permissions"] {
        harness.get_by_role_and_label(Role::Button, column);
    }
    assert_eq!(
        row_names(&harness),
        [
            ".",
            "..",
            "docs",
            "photos",
            "file2",
            "file10",
            "notes.txt",
            "readme.md"
        ]
    );
    // A file's size and permissions show; a directory's size is blank.
    assert!(harness.query_all_by_label("-rw-r--r--").count() >= 4);
    assert!(harness.query_by_label("3 B").is_some());
    harness.get_by_role_and_label(Role::Button, "Name").click();
    harness.run_ok();
    assert_eq!(
        row_names(&harness),
        [
            ".",
            "..",
            "photos",
            "docs",
            "readme.md",
            "notes.txt",
            "file10",
            "file2"
        ]
    );
}

#[test]
fn a_new_session_focuses_the_listing_and_the_keys_browse_it() {
    let fixture = Fixture::start();
    std::fs::write(fixture.root().join("home/tester/docs/plan.txt"), "x").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut harness = browsing(&fixture, &dir);

    // Down twice from `.`: `..`, then docs, selected.
    press(&mut harness, egui::Key::ArrowDown);
    press(&mut harness, egui::Key::ArrowDown);
    assert_eq!(selected_rows(&harness), ["docs"]);
    press(&mut harness, egui::Key::Enter);
    until(&mut harness, "docs listed", |h| {
        h.query_by_label("plan.txt").is_some()
    });
    assert!(harness.query_by_label("notes.txt").is_none());

    // Backspace goes back up and selects the directory just left.
    press(&mut harness, egui::Key::Backspace);
    until(&mut harness, "home listed again", |h| {
        h.query_by_label("notes.txt").is_some()
    });
    assert_eq!(selected_rows(&harness), ["docs"]);

    // Ctrl+A selects every entry but the dots; typing jumps to a prefix.
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.run_ok();
    assert_eq!(selected_rows(&harness).len(), 4);
    harness
        .input_mut()
        .events
        .push(egui::Event::Text("r".into()));
    harness.run_ok();
    assert_eq!(selected_rows(&harness), ["readme.md"]);

    // Ctrl+H shows the hidden entries.
    assert!(harness.query_by_label(".profile").is_none());
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::H);
    harness.run_ok();
    assert!(harness.query_by_label(".profile").is_some());

    // A click chooses a row; Ctrl+click adds one.
    harness.get_by_role_and_label(Role::Row, "photos").click();
    harness.run_ok();
    assert_eq!(selected_rows(&harness), ["photos"]);
}

#[cfg(unix)]
#[test]
fn a_symlinked_directory_is_marked_with_its_target_and_entered() {
    let fixture = Fixture::start();
    std::os::unix::fs::symlink("photos", fixture.root().join("home/tester/pics")).unwrap();
    std::fs::write(fixture.root().join("home/tester/photos/cat.jpg"), "x").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut harness = browsing(&fixture, &dir);
    assert!(has_label(&harness, "→ photos"));
    harness.get_by_role_and_label(Role::Row, "pics").click();
    harness.run_ok();
    press(&mut harness, egui::Key::Enter);
    until(&mut harness, "the link's target listed", |h| {
        h.query_by_label("cat.jpg").is_some()
    });
}

#[test]
fn a_session_opens_in_tree_view_on_its_initial_remote_folder() {
    let fixture = Fixture::start();
    std::fs::write(fixture.root().join("home/tester/docs/plan.txt"), "x").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut harness = tree(&fixture, &dir);

    // Rooted at `/`, the ancestors expanded and the home directory focused, collapsed.
    assert_eq!(row_names(&harness), ["/", "home", "tester"]);
    let tester = harness.get_by_role_and_label(Role::Row, "tester");
    assert_eq!(tester.accesskit_node().data().is_expanded(), Some(false));
    // The toolbar shows the place, and the switch says Tree.
    assert!(harness.query_by_label("/home/tester").is_some());
    let tree = harness.get_by_role_and_label(Role::RadioButton, "Tree");
    assert_eq!(tree.accesskit_node().toggled(), Some(Toggled::True));

    // Right expands; the home directory was fetched on open, so its entries show at once.
    press(&mut harness, egui::Key::ArrowRight);
    assert_eq!(
        row_names(&harness),
        [
            "/",
            "home",
            "tester",
            "docs",
            "photos",
            "notes.txt",
            "readme.md"
        ]
    );
    // Right again moves to the first child; Right on it fetches it.
    press(&mut harness, egui::Key::ArrowRight);
    assert_eq!(selected_rows(&harness), ["docs"]);
    assert!(harness.query_by_label("/home/tester/docs").is_some());
    press(&mut harness, egui::Key::ArrowRight);
    until(&mut harness, "docs fetched", |h| {
        h.query_by_label("plan.txt").is_some()
    });
    // The Current directory follows the Focused entry: a file's parent.
    press(&mut harness, egui::Key::ArrowRight);
    assert_eq!(selected_rows(&harness), ["plan.txt"]);
    assert!(harness.query_by_label("/home/tester/docs").is_some());

    // Left goes to the parent, then collapses it; Enter toggles it again.
    press(&mut harness, egui::Key::ArrowLeft);
    press(&mut harness, egui::Key::ArrowLeft);
    assert!(harness.query_by_label("plan.txt").is_none());
    press(&mut harness, egui::Key::Enter);
    assert!(harness.query_by_label("plan.txt").is_some());

    // Left climbs above the Initial remote folder, to `/`, which the toolbar then shows.
    for _ in 0..6 {
        press(&mut harness, egui::Key::ArrowLeft);
    }
    assert_eq!(row_names(&harness), ["/", "home"]);
    assert_eq!(harness.query_all_by_label("/").count(), 2);
    assert!(selected_rows(&harness).is_empty());
}

#[cfg(unix)]
#[test]
fn a_directory_that_cannot_be_read_shows_the_reason_as_its_child() {
    use std::os::unix::fs::PermissionsExt as _;
    let fixture = Fixture::start();
    let locked = fixture.root().join("home/tester/locked");
    std::fs::create_dir(&locked).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut harness = tree(&fixture, &dir);
    press(&mut harness, egui::Key::ArrowRight);
    harness.get_by_role_and_label(Role::Row, "locked").click();
    harness.run_ok();
    press(&mut harness, egui::Key::ArrowRight);
    until(&mut harness, "the reason", |h| {
        h.query_by_role_and_label(Role::Row, "Cannot read: permission denied")
            .is_some()
    });
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn switching_views_keeps_the_place() {
    let fixture = Fixture::start();
    std::fs::write(fixture.root().join("home/tester/docs/plan.txt"), "x").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut harness = tree(&fixture, &dir);
    press(&mut harness, egui::Key::ArrowRight);
    press(&mut harness, egui::Key::ArrowRight);
    press(&mut harness, egui::Key::ArrowRight);
    until(&mut harness, "docs fetched", |h| {
        h.query_by_label("plan.txt").is_some()
    });
    press(&mut harness, egui::Key::ArrowRight);
    assert_eq!(selected_rows(&harness), ["plan.txt"]);

    // Tree to Folder lists the Current directory with the same entry focused.
    folder_view(&mut harness);
    assert_eq!(row_names(&harness), [".", "..", "plan.txt"]);
    assert_eq!(selected_rows(&harness), ["plan.txt"]);
    let folder = harness.get_by_role_and_label(Role::RadioButton, "Folder");
    assert_eq!(folder.accesskit_node().toggled(), Some(Toggled::True));

    // Backspace goes up to home with docs selected; Ctrl+1 expands to it in the tree.
    press(&mut harness, egui::Key::Backspace);
    assert_eq!(selected_rows(&harness), ["docs"]);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Num1);
    harness.run_ok();
    assert_eq!(selected_rows(&harness), ["docs"]);
    assert!(harness.query_by_label("plan.txt").is_some());
    assert!(harness.query_by_label("/home/tester/docs").is_some());

    // The toolbar switch does the same, and the listing keeps the keyboard.
    harness
        .get_by_role_and_label(Role::RadioButton, "Folder")
        .click();
    harness.run_ok();
    assert_eq!(row_names(&harness), [".", "..", "plan.txt"]);
    assert!(selected_rows(&harness).is_empty());
    press(&mut harness, egui::Key::ArrowDown);
    press(&mut harness, egui::Key::ArrowDown);
    assert_eq!(selected_rows(&harness), ["plan.txt"]);
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

#[test]
fn a_large_directory_draws_only_the_rows_in_view_and_the_keys_reach_every_row() {
    let fixture = Fixture::start();
    let big = fixture.root().join("home/tester/docs");
    for i in 1..=2000 {
        std::fs::write(big.join(format!("f{i:04}")), "x").unwrap();
    }
    let dir = tempfile::tempdir().unwrap();
    let mut harness = browsing(&fixture, &dir);
    harness.get_by_role_and_label(Role::Row, "docs").click();
    harness.run_ok();
    press(&mut harness, egui::Key::Enter);
    until(&mut harness, "the large listing", |h| {
        h.query_by_role_and_label(Role::Row, "f0001").is_some()
    });
    let drawn = harness.get_all_by_role(Role::Row).count();
    assert!(drawn < 100, "{drawn} rows drawn of 2002");

    press(&mut harness, egui::Key::End);
    assert_eq!(selected_rows(&harness), ["f2000"]);
    assert!(
        harness
            .query_by_role_and_label(Role::Row, "f0001")
            .is_none()
    );
    press(&mut harness, egui::Key::PageUp);
    let page = selected_rows(&harness);
    assert_eq!(page.len(), 1);
    assert!(
        page[0].as_str() < "f2000" && page[0].as_str() > "f1900",
        "{page:?}"
    );
    // Home focuses `.`, which is never selected.
    press(&mut harness, egui::Key::Home);
    assert!(harness.query_by_role_and_label(Role::Row, ".").is_some());
    assert!(
        harness
            .query_by_role_and_label(Role::Row, "f2000")
            .is_none()
    );
}
