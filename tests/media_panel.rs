//! Audio and video Preview in the window, against the test server, with the sound going
//! nowhere: the first frame or the placeholder, the toolbar, Space, focus leaving,
//! Autoplay and a format that cannot be decoded.

mod common;

use std::time::{Duration, Instant};

use common::fixture::{self, Fixture};
use egui::accesskit::{Role, Toggled};
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use sshexplorer::app::{self, App};
use sshexplorer::media::AudioOut;
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

/// A window with a Session open in Folder view on the fixture's home, which holds the
/// test clips.
fn browsing(
    fixture: &Fixture,
    dir: &tempfile::TempDir,
    settings: AppSettings,
) -> Harness<'static, App> {
    let home = fixture.root().join(HOME);
    std::fs::copy("tests/media/clip.webm", home.join("clip.webm")).unwrap();
    std::fs::copy("tests/media/tone.ogg", home.join("tone.ogg")).unwrap();
    std::fs::write(home.join("broken.mp4"), b"no movie in here".repeat(64)).unwrap();
    let known_hosts = dir.path().join("known_hosts");
    std::fs::write(&known_hosts, format!("{}\n", fixture.known_hosts_line())).unwrap();
    let app = App::with_settings(settings)
        .known_hosts_at(known_hosts)
        .agent_at(None)
        .audio_out(AudioOut::Silent);
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
        h.query_by_role_and_label(Role::Row, "clip.webm").is_some()
    });
    harness
}

fn click_row(harness: &mut Harness<'static, App>, name: &str) {
    harness.get_by_role_and_label(Role::Row, name).click();
    harness.run_ok();
}

fn has_button(harness: &Harness<'static, App>, label: &str) -> bool {
    harness
        .query_by_role_and_label(Role::Button, label)
        .is_some()
}

fn has_label(harness: &Harness<'static, App>, label: &str) -> bool {
    harness.query_by_label(label).is_some()
}

fn preview_focused(harness: &Harness<'static, App>) -> bool {
    harness.get_by_label("Preview").is_focused()
}

#[test]
fn a_video_shows_its_first_frame_under_the_media_and_zoom_toolbars() {
    let fixture = Fixture::start();
    let dir = tempfile::tempdir().unwrap();
    let mut harness = browsing(&fixture, &dir, AppSettings::default());
    click_row(&mut harness, "clip.webm");
    until(&mut harness, "the player", |h| has_button(h, "Play"));
    assert!(has_label(&harness, "0:00 / 0:02"));
    assert!(
        harness
            .query_by_role_and_label(Role::Slider, "Seek")
            .is_some()
    );
    assert!(
        harness
            .query_by_role_and_label(Role::Slider, "Volume")
            .is_some()
    );
    assert!(has_button(&harness, "Mute"));
    // The zoom toolbar, at Fit, with the frame's size; the frame is the view's content.
    let fit = harness.get_by_role_and_label(Role::Button, "Fit");
    assert_eq!(fit.accesskit_node().toggled(), Some(Toggled::True));
    assert!(has_label(&harness, "64 × 48"));
    harness.get_by_role_and_label(Role::Button, "200%").click();
    harness.run_ok();
    let doubled = harness.get_by_role_and_label(Role::Button, "200%");
    assert_eq!(doubled.accesskit_node().toggled(), Some(Toggled::True));

    harness.get_by_role_and_label(Role::Button, "Play").click();
    harness.run_ok();
    until(&mut harness, "playback", |h| has_button(h, "Pause"));
    until(&mut harness, "the clock to move", |h| {
        has_label(h, "0:01 / 0:02")
    });
    harness.get_by_role_and_label(Role::Button, "Pause").click();
    harness.run_ok();
    assert!(has_button(&harness, "Play"));

    harness.get_by_role_and_label(Role::Button, "Mute").click();
    harness.run_ok();
    let mute = harness.get_by_role_and_label(Role::Button, "Mute");
    assert_eq!(mute.accesskit_node().toggled(), Some(Toggled::True));
}

#[test]
fn audio_shows_a_placeholder_and_no_zoom_toolbar() {
    let fixture = Fixture::start();
    let dir = tempfile::tempdir().unwrap();
    let mut harness = browsing(&fixture, &dir, AppSettings::default());
    click_row(&mut harness, "tone.ogg");
    until(&mut harness, "the player", |h| has_button(h, "Play"));
    assert!(has_label(
        &harness,
        sshexplorer::media_view::AUDIO_PLACEHOLDER
    ));
    assert!(!has_button(&harness, "Fit"));
}

#[test]
fn space_plays_while_preview_has_focus_and_leaving_preview_stops() {
    let fixture = Fixture::start();
    let dir = tempfile::tempdir().unwrap();
    let mut harness = browsing(&fixture, &dir, AppSettings::default());
    click_row(&mut harness, "tone.ogg");
    until(&mut harness, "the player", |h| has_button(h, "Play"));
    // Space in the listing does nothing to it.
    harness.key_press(Key::Space);
    harness.run_ok();
    assert!(has_button(&harness, "Play"));

    harness.key_press(Key::Enter);
    harness.run_ok();
    assert!(preview_focused(&harness));
    harness.key_press(Key::Space);
    harness.run_ok();
    assert!(has_button(&harness, "Pause"));
    harness.key_press(Key::Space);
    harness.run_ok();
    assert!(has_button(&harness, "Play"));

    // Clicking a control keeps it playing; Escape back to the listing stops it.
    harness.key_press(Key::Space);
    harness.run_ok();
    harness.get_by_role_and_label(Role::Button, "Mute").click();
    harness.run_ok();
    assert!(has_button(&harness, "Pause"));
    harness.get_by_label("Preview").focus();
    harness.run_ok();
    harness.key_press(Key::Escape);
    harness.run_ok();
    assert!(!preview_focused(&harness));
    assert!(has_button(&harness, "Play"));
}

#[test]
fn autoplay_starts_playback_once_loaded() {
    let fixture = Fixture::start();
    let dir = tempfile::tempdir().unwrap();
    let mut settings = AppSettings::default();
    settings.preview.autoplay = true;
    let mut harness = browsing(&fixture, &dir, settings);
    click_row(&mut harness, "tone.ogg");
    until(&mut harness, "playback", |h| has_button(h, "Pause"));
}

#[test]
fn a_format_it_cannot_decode_shows_the_failure_line() {
    let fixture = Fixture::start();
    let dir = tempfile::tempdir().unwrap();
    let mut harness = browsing(&fixture, &dir, AppSettings::default());
    click_row(&mut harness, "broken.mp4");
    until(&mut harness, "the failure line", |h| {
        has_label(h, "Cannot preview: unsupported format")
    });
    assert!(!has_button(&harness, "Play"));
}
