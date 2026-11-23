//! PDF Preview in the window, against the test server: the page counter and the zoom
//! toolbar, scrolling through the pages, an encrypted PDF, and one over the size cap.

mod common;

use std::time::{Duration, Instant};

use common::fixture::{self, Fixture};
use common::pdf;
use egui::accesskit::{Role, Toggled};
use egui::{Key, Modifiers};
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

/// A window with a Session open in Folder view on the fixture's home.
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
    until(&mut harness, "the tree", |h| {
        h.query_by_role_and_label(Role::Row, "tester").is_some()
    });
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Num2);
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

fn has_label(harness: &Harness<'static, App>, label: &str) -> bool {
    harness.query_by_label(label).is_some()
}

/// The zoom choice in force, as the toolbar shows it.
fn zoom(harness: &Harness<'static, App>) -> String {
    let on: Vec<String> = harness
        .query_all_by_role(Role::Button)
        .filter(|n| n.accesskit_node().toggled() == Some(Toggled::True))
        .filter_map(|n| n.accesskit_node().label())
        .filter(|l| l == "Fit" || l.ends_with('%'))
        .collect();
    match on.as_slice() {
        [one] => one.clone(),
        [] => harness
            .query_by(|n| n.label().as_deref() == Some("Zoom"))
            .and_then(|n| n.value())
            .unwrap_or_default(),
        more => panic!("more than one zoom choice is on: {more:?}"),
    }
}

#[test]
fn a_pdf_shows_its_page_counter_and_the_zoom_toolbar() {
    let fixture = Fixture::start();
    let home = fixture.root().join(HOME);
    std::fs::write(home.join("report.pdf"), pdf::pages(&[(612, 792); 3])).unwrap();
    std::fs::write(home.join("letter.pdf"), pdf::pages(&[(612, 792)])).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut harness = browsing(&fixture, &dir, AppSettings::default());

    click_row(&mut harness, "report.pdf");
    until(&mut harness, "the page counter", |h| has_label(h, "1 / 3"));
    for preset in ["Fit", "50%", "100%", "200%", "400%"] {
        assert!(
            harness
                .query_by_role_and_label(Role::Button, preset)
                .is_some(),
            "{preset}"
        );
    }
    assert_eq!(zoom(&harness), "Fit");
    assert!(has_label(&harness, "PDF"), "the pages' view");

    // The zoom keys and the toolbar.
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Equals);
    harness.run_ok();
    assert_ne!(zoom(&harness), "Fit");
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Num0);
    harness.run_ok();
    assert_eq!(zoom(&harness), "Fit");
    harness.get_by_role_and_label(Role::Button, "200%").click();
    harness.run_ok();
    assert_eq!(zoom(&harness), "200%");

    // The right-click menu on the pages.
    harness.get_by_label("PDF").click_secondary();
    harness.run_ok();
    let items: Vec<_> = harness
        .query_all_by_role_and_label(Role::Button, "100%")
        .collect();
    assert_eq!(items.len(), 2, "the menu is open");
    let view = harness.get_by_label("PDF").rect();
    let item = items
        .into_iter()
        .find(|n| n.rect().center().y > view.min.y)
        .expect("the menu's 100 % is over the pages");
    item.click();
    harness.run_ok();
    assert_eq!(zoom(&harness), "100%");

    // With Preview holding the keyboard, End scrolls to the last page.
    harness.get_by_label("PDF").click();
    harness.run_ok();
    assert!(
        harness.get_by_label("Preview").is_focused(),
        "a click on the pages"
    );
    harness.key_press(Key::End);
    harness.run_ok();
    until(&mut harness, "the last page", |h| has_label(h, "3 / 3"));
    harness.key_press(Key::Home);
    harness.run_ok();
    until(&mut harness, "the first page", |h| has_label(h, "1 / 3"));

    // Another file starts at Fit.
    click_row(&mut harness, "letter.pdf");
    until(&mut harness, "the one-page counter", |h| {
        has_label(h, "1 / 1")
    });
    assert_eq!(zoom(&harness), "Fit");
}

#[test]
fn an_encrypted_pdf_says_it_cannot_be_previewed() {
    let fixture = Fixture::start();
    std::fs::write(
        fixture.root().join(HOME).join("locked.pdf"),
        pdf::encrypted(&[(612, 792)]),
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut harness = browsing(&fixture, &dir, AppSettings::default());
    click_row(&mut harness, "locked.pdf");
    until(&mut harness, "the failure line", |h| {
        h.query_by_label_contains("Cannot preview: encrypted PDF")
            .is_some()
    });
    assert!(harness.query_by_role(Role::PasswordInput).is_none());
}

#[test]
fn a_pdf_over_the_size_cap_says_so() {
    let fixture = Fixture::start();
    let mut big = pdf::pages(&[(612, 792)]);
    big.resize(2 << 20, b' ');
    std::fs::write(fixture.root().join(HOME).join("scan.pdf"), big).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut settings = AppSettings::default();
    settings.preview.size_cap = 1;
    let mut harness = browsing(&fixture, &dir, settings);
    click_row(&mut harness, "scan.pdf");
    until(&mut harness, "the verdict", |h| {
        has_label(h, "Too large to preview (2 MiB)")
    });
}
