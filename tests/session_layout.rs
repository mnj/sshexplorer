//! The Session's layout in the window: the split between the listing and Preview starts
//! at about half the width and stays where its edge is dragged to, and one click on a
//! directory's chevron opens or closes it.

mod common;

use std::time::{Duration, Instant};

use common::fixture::{self, Fixture};
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use sshexplorer::app::{self, App};
use sshexplorer::settings::AppSettings;

const WIDTH: f32 = 1200.0;

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

fn browsing(fixture: &Fixture, dir: &tempfile::TempDir) -> Harness<'static, App> {
    let known_hosts = dir.path().join("known_hosts");
    std::fs::write(&known_hosts, format!("{}\n", fixture.known_hosts_line())).unwrap();
    let app = App::with_settings(AppSettings::default())
        .known_hosts_at(known_hosts)
        .agent_at(None);
    let mut harness = Harness::builder()
        .with_step_dt(1.0 / 60.0)
        .with_size(egui::vec2(WIDTH, 700.0))
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
    harness
}

/// Where the edge between the listing and Preview is, across the window.
fn edge(harness: &Harness<'static, App>) -> f32 {
    pane_edge(harness).center().x
}

fn pane_edge(harness: &Harness<'static, App>) -> egui::Rect {
    harness
        .get_all_by_role(Role::Splitter)
        .map(|n| n.rect())
        .max_by(|a, b| a.height().total_cmp(&b.height()))
        .unwrap()
}

fn drag(harness: &mut Harness<'static, App>, from: f32, to: f32) {
    let y = pane_edge(harness).center().y;
    drag_at(harness, from, to, y);
}

fn drag_at(harness: &mut Harness<'static, App>, from: f32, to: f32, y: f32) {
    let at = |x| egui::pos2(x, y);
    harness.hover_at(at(from));
    harness.run_ok();
    harness.event(egui::Event::PointerButton {
        pos: at(from),
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
    for i in 1..=10 {
        let x = from + (to - from) * i as f32 / 10.0;
        harness.event(egui::Event::PointerMoved(at(x)));
        harness.step();
    }
    harness.event(egui::Event::PointerButton {
        pos: at(to),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
    harness.event(egui::Event::PointerMoved(at(to + 200.0)));
    harness.run_ok();
}

#[test]
fn the_listing_and_preview_start_about_half_each() {
    let fixture = Fixture::start();
    let dir = tempfile::tempdir().unwrap();
    let harness = browsing(&fixture, &dir);
    let x = edge(&harness);
    assert!(
        (x - WIDTH / 2.0).abs() < WIDTH * 0.05,
        "the edge is at {x} of {WIDTH}"
    );
}

#[test]
fn a_dragged_edge_stays_where_it_was_let_go() {
    let fixture = Fixture::start();
    let dir = tempfile::tempdir().unwrap();
    let mut harness = browsing(&fixture, &dir);
    for to in [WIDTH * 0.7, WIDTH * 0.3] {
        let from = edge(&harness);
        drag(&mut harness, from, to);
        for _ in 0..5 {
            harness.step();
        }
        let x = edge(&harness);
        assert!((x - to).abs() < 12.0, "dragged to {to}, the edge is at {x}");
    }
}

fn expanded(harness: &Harness<'static, App>, name: &str) -> Option<bool> {
    harness
        .get_by_role_and_label(Role::Row, name)
        .accesskit_node()
        .data()
        .is_expanded()
}

fn click_at(harness: &mut Harness<'static, App>, at: egui::Pos2) {
    harness.hover_at(at);
    harness.step();
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        harness.step();
    }
    // Long enough apart that two clicks are never one double-click.
    for _ in 0..60 {
        harness.step();
    }
    harness.run_ok();
}

#[test]
fn one_click_on_a_chevron_opens_and_closes_its_directory() {
    let fixture = Fixture::start();
    let dir = tempfile::tempdir().unwrap();
    let mut harness = browsing(&fixture, &dir);
    assert_eq!(expanded(&harness, "tester"), Some(false));
    let row = harness.get_by_role_and_label(Role::Row, "tester").rect();
    // A click on the name only focuses the row.
    click_at(&mut harness, egui::pos2(row.left() + 120.0, row.center().y));
    assert_eq!(expanded(&harness, "tester"), Some(false));
    // The chevron sits in the lead before the name: one click there opens the directory.
    let mut x = row.left();
    while expanded(&harness, "tester") == Some(false) {
        assert!(x < row.left() + 100.0, "no chevron took a click");
        click_at(&mut harness, egui::pos2(x, row.center().y));
        x += 4.0;
    }
    assert_eq!(expanded(&harness, "tester"), Some(true));
    click_at(&mut harness, egui::pos2(x - 4.0, row.center().y));
    assert_eq!(expanded(&harness, "tester"), Some(false));
}

#[test]
fn the_session_tab_is_at_the_top_and_global_actions_share_the_status_bar() {
    let fixture = Fixture::start();
    let dir = tempfile::tempdir().unwrap();
    let harness = browsing(&fixture, &dir);
    let tab = harness.get_by_role(Role::Tab).rect();
    let hosts = harness.get_by_role_and_label(Role::Button, "Hosts").rect();
    assert!(
        tab.top() < tab.height() && hosts.top() > tab.bottom(),
        "tab {tab:?}, Hosts {hosts:?}"
    );
    let gear = harness
        .get_by_role_and_label(Role::Button, "Settings")
        .rect();
    assert!(hosts.right() <= gear.left() && gear.left() - hosts.right() < hosts.width());
    assert_eq!(hosts.center().y, gear.center().y);
    let downloads = harness
        .get_by_role_and_label(Role::Button, "Downloads")
        .rect();
    assert!((hosts.center().y - downloads.center().y).abs() < 2.0);
    assert!(hosts.left() > WIDTH * 0.75);
    assert!(
        harness
            .query_by_role_and_label(Role::Button, "Download")
            .is_none()
    );
    for (role, label) in [
        (Role::Button, "Refresh"),
        (Role::RadioButton, "Tree"),
        (Role::RadioButton, "Folder"),
    ] {
        let rect = harness.get_by_role_and_label(role, label).rect();
        assert!(
            rect.top() >= tab.bottom(),
            "{label} {rect:?} is not under the tab {tab:?}"
        );
    }
    let place = harness.get_by_label("/home/tester").rect();
    assert!(
        place.top() >= tab.bottom(),
        "the place is not under the tab"
    );
}

#[test]
fn the_tree_context_menu_copies_the_clicked_path() {
    let fixture = Fixture::start();
    let dir = tempfile::tempdir().unwrap();
    let mut h = browsing(&fixture, &dir);
    h.get_by_role_and_label(Role::Row, "/").click_secondary();
    h.run_ok();
    let download = h.get_by_label_contains("Download ");
    assert!(download.accesskit_node().is_disabled());
    h.get_by_role_and_label(Role::Button, "Copy path").click();
    h.step();
    assert!(
        h.output()
            .platform_output
            .commands
            .iter()
            .any(|c| matches!(c, egui::OutputCommand::CopyText(path) if path == "/"))
    );
}

#[test]
fn tree_column_dividers_can_be_dragged_from_the_edge_of_their_hit_area() {
    let fixture = Fixture::start();
    let dir = tempfile::tempdir().unwrap();
    let mut h = browsing(&fixture, &dir);
    let divider = |h: &Harness<'static, App>| h.get_by_label("Resize column 1").rect();
    let before = divider(&h);
    let from = before.center().x + 4.0;
    drag_at(&mut h, from, from - 65.0, before.center().y);
    assert!((divider(&h).center().x - (before.center().x - 65.0)).abs() < 5.0);
    h.get_by_label("Resize column 1").click();
    h.step();
    h.get_by_label("Resize column 1").click();
    h.run_ok();
    assert!((divider(&h).center().x - before.center().x).abs() < 2.0);
}

#[test]
fn preview_text_uses_the_bottom_of_the_pane() {
    let fixture = Fixture::start();
    let text: String = (1..=100).map(|i| format!("preview line {i}\n")).collect();
    std::fs::write(fixture.root().join("home/tester/notes.txt"), text).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut h = browsing(&fixture, &dir);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Num2);
    h.run_ok();
    until(&mut h, "the listing", |h| {
        h.query_by_role_and_label(Role::Row, "notes.txt").is_some()
    });
    h.get_by_role_and_label(Role::Row, "notes.txt").click();
    h.run_ok();
    until(&mut h, "preview text", |h| {
        h.query_by_label("preview line 1").is_some()
    });
    let edge = pane_edge(&h);
    assert!(h.query_all_by_label_contains("preview line ").any(|n| {
        let y = n.rect().center().y;
        y > edge.top() + 0.85 * edge.height() && y <= edge.bottom()
    }));
}
