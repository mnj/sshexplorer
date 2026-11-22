//! Image Preview in the window, against the test server: the zoom toolbar with its
//! presets, keys, right-click menu and Ctrl+wheel, Fit for each newly focused file, SVG,
//! and an image over the size cap.

mod common;

use std::io::Cursor;
use std::time::{Duration, Instant};

use common::fixture::{self, Fixture};
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

fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba([10, 120, 200, 255]));
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

fn click_row(harness: &mut Harness<'static, App>, name: &str) {
    harness.get_by_role_and_label(Role::Row, name).click();
    harness.run_ok();
}

fn ctrl(harness: &mut Harness<'static, App>, key: Key) {
    harness.key_press_modifiers(Modifiers::COMMAND, key);
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
        // Ctrl+wheel between presets: none is on, and the toolbar says the scale.
        [] => harness
            .query_by(|n| n.label().as_deref() == Some("Zoom"))
            .and_then(|n| n.value())
            .unwrap_or_default(),
        more => panic!("more than one zoom choice is on: {more:?}"),
    }
}

#[test]
fn an_image_shows_with_the_zoom_toolbar_its_keys_and_its_menu() {
    let fixture = Fixture::start();
    let home = fixture.root().join(HOME);
    std::fs::write(home.join("photo.png"), png(2000, 1500)).unwrap();
    std::fs::write(home.join("small.png"), png(40, 30)).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut harness = browsing(&fixture, &dir, AppSettings::default());

    click_row(&mut harness, "photo.png");
    until(&mut harness, "the pixel dimensions", |h| {
        has_label(h, "2000 × 1500")
    });
    for preset in ["Fit", "50%", "100%", "200%", "400%"] {
        assert!(
            harness
                .query_by_role_and_label(Role::Button, preset)
                .is_some(),
            "{preset}"
        );
    }
    assert_eq!(zoom(&harness), "Fit");

    // The keys, while the listing still has the keyboard.
    ctrl(&mut harness, Key::Equals);
    assert_eq!(zoom(&harness), "50%");
    ctrl(&mut harness, Key::Equals);
    assert_eq!(zoom(&harness), "100%");
    ctrl(&mut harness, Key::Minus);
    assert_eq!(zoom(&harness), "50%");
    ctrl(&mut harness, Key::Num0);
    assert_eq!(zoom(&harness), "Fit");

    // The toolbar.
    harness.get_by_role_and_label(Role::Button, "200%").click();
    harness.run_ok();
    assert_eq!(zoom(&harness), "200%");

    // The right-click menu on the image.
    harness.get_by_label("Image").click_secondary();
    harness.run_ok();
    // The menu's items are buttons too: the toolbar's 400 % and the menu's.
    let items: Vec<_> = harness
        .query_all_by_role_and_label(Role::Button, "400%")
        .collect();
    assert_eq!(items.len(), 2, "the menu is open");
    let view = harness.get_by_label("Image").rect();
    let item = items
        .into_iter()
        .find(|n| n.rect().center().y > view.min.y)
        .expect("the menu's 400 % is over the image");
    item.click();
    harness.run_ok();
    assert_eq!(
        harness
            .query_all_by_role_and_label(Role::Button, "400%")
            .count(),
        1,
        "the menu closed"
    );
    assert_eq!(zoom(&harness), "400%");

    // Ctrl+wheel over the image, out from 400 %.
    let at = harness.get_by_label("Image").rect().center();
    harness.hover_at(egui::pos2(at.x as f32, at.y as f32));
    harness.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: egui::vec2(0.0, -3.0),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Wheel,
        modifiers: Modifiers::COMMAND,
    });
    harness.run_ok();
    let wheeled = zoom(&harness);
    assert_ne!(wheeled, "400%");
    assert!(wheeled.ends_with('%'), "{wheeled}");

    // Another file starts at Fit, and so does this one when it is focused again.
    click_row(&mut harness, "small.png");
    until(&mut harness, "the small image", |h| has_label(h, "40 × 30"));
    assert_eq!(zoom(&harness), "Fit");
    click_row(&mut harness, "photo.png");
    until(&mut harness, "the photo again", |h| {
        has_label(h, "2000 × 1500")
    });
    assert_eq!(zoom(&harness), "Fit");
}

#[test]
fn svg_shows_its_own_dimensions() {
    let fixture = Fixture::start();
    std::fs::write(
        fixture.root().join(HOME).join("logo.svg"),
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="12"><rect width="24" height="12" fill="#c33"/></svg>"##,
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut harness = browsing(&fixture, &dir, AppSettings::default());
    click_row(&mut harness, "logo.svg");
    until(&mut harness, "the SVG", |h| has_label(h, "24 × 12"));
}

#[test]
fn an_image_over_the_size_cap_says_so_and_points_to_download() {
    let fixture = Fixture::start();
    std::fs::write(
        fixture.root().join(HOME).join("scan.png"),
        vec![0u8; 2 << 20],
    )
    .unwrap();
    std::fs::write(fixture.root().join(HOME).join("broken.png"), b"not a png").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut settings = AppSettings::default();
    settings.preview.size_cap = 1;
    let mut harness = browsing(&fixture, &dir, settings);
    click_row(&mut harness, "scan.png");
    until(&mut harness, "the verdict", |h| {
        has_label(h, "Too large to preview (2 MiB)")
    });
    assert!(
        harness
            .query_by_label_contains("Download it to open it locally")
            .is_some()
    );

    click_row(&mut harness, "broken.png");
    until(&mut harness, "the failure line", |h| {
        h.query_by_label_contains("Cannot preview:").is_some()
    });
}
