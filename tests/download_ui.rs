//! Download from the window: Ctrl+S and the toolbar button, the strip, the conflict
//! prompt that is modal to its Session only, Details, and closing during a Download.

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::fixture::{self, Fixture};
use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use sshexplorer::app::{self, App};
use sshexplorer::hosts::{Host, HostStore};
use sshexplorer::settings::Store;

const CTRL: Modifiers = Modifiers::COMMAND;

struct Window {
    harness: Harness<'static, App>,
    local: PathBuf,
    _dir: tempfile::TempDir,
}

fn host(fixture: &Fixture, name: &str, local: &Path) -> Host {
    Host {
        display_name: name.into(),
        address: fixture.address().into(),
        port: Some(fixture.port()),
        account: fixture::ACCOUNT.into(),
        local_folder: local.to_path_buf(),
        ..Host::default()
    }
}

/// The window with the fixture saved as the Hosts `names`, all downloading to one
/// Local folder.
fn window(fixture: &Fixture, names: &[&str]) -> Window {
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("downloads");
    std::fs::create_dir_all(&local).unwrap();
    let store = Store::at(dir.path().join("settings.toml"));
    let hosts: Vec<Host> = names.iter().map(|n| host(fixture, n, &local)).collect();
    HostStore::beside(store.path()).save(&hosts).unwrap();
    let known_hosts = dir.path().join("known_hosts");
    std::fs::write(&known_hosts, format!("{}\n", fixture.known_hosts_line())).unwrap();
    let app = App::with_store(store)
        .known_hosts_at(known_hosts)
        .agent_at(None);
    let harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 800.0))
        .with_context_setup(app::setup)
        .build_ui_state(|ui, app: &mut App| app.ui(ui), app);
    Window {
        harness,
        local,
        _dir: dir,
    }
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

fn has(h: &Harness<'static, App>, label: &str) -> bool {
    h.query_all_by_label_contains(label).next().is_some()
}

/// Opens the saved Host `name` from the Host dialog and waits for its listing.
fn open(h: &mut Harness<'static, App>, name: &str) {
    h.key_press_modifiers(CTRL, Key::O);
    h.run_ok();
    h.get_by_role_and_label(Role::ListItem, name).click();
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
    // A Session opens in Tree view on its home; Ctrl+2 lists it in Folder view.
    until(h, "the tree", |h| {
        h.query_by_role_and_label(Role::Row, "tester").is_some()
    });
    h.key_press_modifiers(CTRL, Key::Num2);
    h.run_ok();
    until(h, "the listing", |h| {
        h.query_by_label("notes.txt").is_some()
    });
}

/// Clicks the listing's row `name`, which selects it.
fn select(h: &mut Harness<'static, App>, name: &str) {
    h.get_by_label(name).click();
    h.run_ok();
}

#[test]
fn ctrl_s_downloads_the_selection_and_a_clean_finish_clears_the_strip() {
    let fixture = Fixture::start();
    let mut w = window(&fixture, &["Fixture"]);
    let h = &mut w.harness;
    open(h, "Fixture");
    // A new listing is focused on `.`, which is never selected: nothing to download.
    assert!(
        h.get_by_role_and_label(Role::Button, "Download")
            .accesskit_node()
            .is_disabled()
    );
    select(h, "notes.txt");
    h.key_press_modifiers(CTRL, Key::S);
    h.run_ok();

    until(h, "the finish", |h| has(h, "Downloaded 1 file"));
    assert_eq!(
        std::fs::read_to_string(w.local.join("notes.txt")).unwrap(),
        "notes\n"
    );
    assert!(h.query_by_role_and_label(Role::Button, "Details").is_none());
    until(h, "the strip to clear", |h| !has(h, "Downloaded 1 file"));

    // The toolbar button does the same.
    select(h, "docs");
    h.get_by_role_and_label(Role::Button, "Download").click();
    h.run_ok();
    until(h, "the second finish", |h| has(h, "Downloaded 0 files"));
    assert!(w.local.join("docs").is_dir());
}

#[test]
fn a_conflict_asks_in_its_session_and_a_skip_keeps_the_strip_with_details() {
    let fixture = Fixture::start();
    let mut w = window(&fixture, &["Fixture"]);
    std::fs::write(w.local.join("notes.txt"), "mine").unwrap();
    let h = &mut w.harness;
    open(h, "Fixture");
    select(h, "notes.txt");
    h.key_press_modifiers(CTRL, Key::S);
    h.run_ok();

    until(h, "the conflict prompt", |h| {
        has(h, "notes.txt already exists")
    });
    let name = h.get_by_role_and_label(Role::TextInput, "New name");
    assert_eq!(name.value().as_deref(), Some("notes (1).txt"));
    let apply = h.get_by_role_and_label(Role::CheckBox, "Apply to all remaining");
    assert!(!apply.accesskit_node().is_disabled());
    for button in ["Skip", "Overwrite", "Rename"] {
        h.get_by_role_and_label(Role::Button, button);
    }
    // Cancel is both the prompt's and the strip's.
    assert_eq!(
        h.query_all_by_role_and_label(Role::Button, "Cancel")
            .count(),
        2
    );
    // A name edited by hand cannot apply to all.
    h.get_by_role_and_label(Role::TextInput, "New name").focus();
    h.run_ok();
    h.get_by_role_and_label(Role::TextInput, "New name")
        .type_text("x");
    h.run_ok();
    assert!(
        h.get_by_role_and_label(Role::CheckBox, "Apply to all remaining")
            .accesskit_node()
            .is_disabled()
    );

    h.get_by_role_and_label(Role::Button, "Skip").click();
    h.run_ok();
    until(h, "the summary", |h| has(h, "1 skipped"));
    assert_eq!(
        std::fs::read_to_string(w.local.join("notes.txt")).unwrap(),
        "mine"
    );
    h.get_by_role_and_label(Role::Button, "Details").click();
    h.run_ok();
    assert!(has(h, "Skipped: 1"));
    assert!(has(h, "notes.txt — Already exists"));
    h.get_by_role_and_label(Role::Button, "Close").click();
    h.run_ok();
    h.get_by_role_and_label(Role::Button, "Dismiss").click();
    h.run_ok();
    assert!(!has(h, "1 skipped"));
}

#[test]
fn a_session_off_screen_is_marked_and_its_prompt_waits() {
    let fixture = Fixture::start();
    let mut w = window(&fixture, &["First", "Second"]);
    std::fs::write(w.local.join("readme.md"), "mine").unwrap();
    let h = &mut w.harness;
    open(h, "First");
    select(h, "readme.md");
    h.key_press_modifiers(CTRL, Key::S);
    h.run_ok();
    until(h, "the conflict prompt", |h| {
        has(h, "readme.md already exists")
    });

    // The prompt holds only its Session: another opens while it is up.
    open(h, "Second");
    assert!(!has(h, "readme.md already exists"));
    let first = h.get_by_role_and_label(Role::Tab, "First");
    let tip = first.accesskit_node().description().unwrap_or_default();
    assert!(tip.contains("waiting for an answer"), "{tip}");

    h.get_by_role_and_label(Role::Tab, "First").click();
    h.run_ok();
    assert!(has(h, "readme.md already exists"));
    h.get_by_role_and_label(Role::Button, "Overwrite").click();
    h.run_ok();
    until(h, "the finish", |h| has(h, "Downloaded 1 file"));
    assert_eq!(
        std::fs::read_to_string(w.local.join("readme.md")).unwrap(),
        "# readme\n"
    );
}

#[test]
fn closing_a_session_during_a_download_asks_first_and_the_strip_counts_the_queue() {
    let fixture = Fixture::start();
    std::fs::write(
        fixture.root().join("home/tester/big.bin"),
        vec![0u8; 8 << 20],
    )
    .unwrap();
    let mut w = window(&fixture, &["Fixture"]);
    let h = &mut w.harness;
    open(h, "Fixture");
    fixture.slow_reads(Duration::from_millis(100));
    select(h, "big.bin");
    h.key_press_modifiers(CTRL, Key::S);
    h.run_ok();
    h.key_press_modifiers(CTRL, Key::S);
    h.run_ok();
    until(h, "the strip", |h| has(h, "+1 queued"));
    until(h, "the current file", |h| has(h, "of 1 files"));
    assert!(has(h, "big.bin"));

    h.key_press_modifiers(CTRL, Key::W);
    h.run_ok();
    assert!(has(h, "A Download is running. Close anyway?"));
    // Escape is the dialog's Cancel.
    h.key_press(Key::Escape);
    h.run_ok();
    assert!(!has(h, "A Download is running. Close anyway?"));
    assert!(h.query_by_role_and_label(Role::Tab, "Fixture").is_some());

    h.key_press_modifiers(CTRL, Key::W);
    h.run_ok();
    h.get_by_role_and_label(Role::Button, "Close anyway")
        .click();
    h.run_ok();
    assert!(h.query_by_role_and_label(Role::Tab, "Fixture").is_none());
    // Closing cancelled the Download: nothing is left half-written.
    let deadline = Instant::now() + Duration::from_secs(5);
    while std::fs::read_dir(&w.local).unwrap().next().is_some() {
        assert!(Instant::now() < deadline, "the file in flight is removed");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Whether the last frame sent the root viewport `command`.
fn sent(h: &Harness<'static, App>, command: &egui::ViewportCommand) -> bool {
    h.output()
        .viewport_output
        .get(&egui::ViewportId::ROOT)
        .is_some_and(|v| v.commands.contains(command))
}

#[test]
fn quitting_during_a_download_asks_first() {
    let fixture = Fixture::start();
    std::fs::write(
        fixture.root().join("home/tester/big.bin"),
        vec![0u8; 8 << 20],
    )
    .unwrap();
    let mut w = window(&fixture, &["Fixture"]);
    let h = &mut w.harness;
    open(h, "Fixture");
    fixture.slow_reads(Duration::from_millis(100));
    select(h, "big.bin");
    h.key_press_modifiers(CTRL, Key::S);
    h.run_ok();
    until(h, "the strip", |h| has(h, "of 1 files"));

    h.input_mut()
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .events
        .push(egui::ViewportEvent::Close);
    h.step();
    assert!(sent(h, &egui::ViewportCommand::CancelClose));
    h.run_ok();
    assert!(has(h, "A Download is running. Close anyway?"));
    h.get_by_role_and_label(Role::Button, "Close anyway")
        .click();
    h.step();
    assert!(sent(h, &egui::ViewportCommand::Close));
}

#[test]
fn in_tree_view_the_selected_directory_is_downloaded() {
    let fixture = Fixture::start();
    let mut w = window(&fixture, &["Fixture"]);
    let h = &mut w.harness;
    open(h, "Fixture");
    // Back in Tree view the home directory is focused and selected.
    h.key_press_modifiers(CTRL, Key::Num1);
    h.run_ok();
    until(h, "the tree", |h| {
        h.query_by_role_and_label(Role::Row, "tester").is_some()
    });
    h.get_by_role_and_label(Role::Row, "tester").click();
    h.run_ok();
    h.key_press_modifiers(CTRL, Key::S);
    h.run_ok();
    // The whole directory, hidden entries too.
    until(h, "the finish", |h| has(h, "Downloaded 3 files"));
    assert_eq!(
        std::fs::read_to_string(w.local.join("tester/notes.txt")).unwrap(),
        "notes\n"
    );
    assert!(w.local.join("tester/docs").is_dir());
    assert!(w.local.join("tester/.profile").is_file());
}

#[test]
fn queued_jobs_can_be_inspected_and_removed_without_cancelling_the_active_download() {
    let fixture = Fixture::start();
    std::fs::write(
        fixture.root().join("home/tester/current.bin"),
        vec![7; 512 * 1024],
    )
    .unwrap();
    let mut w = window(&fixture, &["Fixture"]);
    let h = &mut w.harness;
    open(h, "Fixture");
    std::fs::write(w.local.join("notes.txt"), "local").unwrap();
    fixture.slow_reads(Duration::from_millis(100));
    select(h, "current.bin");
    h.key_press_modifiers(CTRL, Key::S);
    h.run_ok();
    select(h, "notes.txt");
    h.key_press_modifiers(CTRL, Key::S);
    h.run_ok();
    until(h, "queued", |h| has(h, "+1 queued"));
    h.get_by_role_and_label(Role::Button, "+1 queued").click();
    h.run_ok();
    assert!(has(h, "/home/tester/notes.txt"));
    h.get_by_role_and_label(Role::Button, "Remove").click();
    h.run_ok();
    until(h, "completed destination", |h| {
        h.query_by_role_and_label(Role::Button, "Show in file manager")
            .is_some()
    });
    assert!(has(h, "No downloads queued"));
    assert!(w.local.join("current.bin").exists());
    assert_eq!(
        std::fs::read_to_string(w.local.join("notes.txt")).unwrap(),
        "local"
    );
}
