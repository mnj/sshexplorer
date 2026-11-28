//! The Settings dialog: opened by the gear or Ctrl+, (also at Launch), pages on the left
//! and fields on the right, every change applied on the next frame and kept across a
//! restart.

use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use sshexplorer::app::{self, App};
use sshexplorer::settings::{AppSettings, ScratchChoice, Store, ThemeChoice};
use sshexplorer::theme::{Accent, Density, Settings, Tokens, TokensExt as _, fonts};

const PAGES: [&str; 7] = [
    "Appearance",
    "Browsing",
    "Search",
    "Preview",
    "Download",
    "Connections",
    "Keys",
];

struct Launched {
    harness: Harness<'static, App>,
    store: Store,
    _dir: tempfile::TempDir,
}

/// Launches the app on a settings file holding `settings` (none when `None`).
fn launch_with(settings: Option<&AppSettings>) -> Launched {
    let dir = tempfile::tempdir().expect("a temp dir");
    let store = Store::at(dir.path().join("settings.toml"));
    if let Some(settings) = settings {
        store.save(settings).expect("saved");
    }
    let app = App::with_store(store.clone());
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 760.0))
        .with_context_setup(app::setup)
        .build_ui_state(|ui, app: &mut App| app.ui(ui), app);
    harness.run();
    Launched {
        harness,
        store,
        _dir: dir,
    }
}

fn launch() -> Launched {
    launch_with(None)
}

fn open_with_gear(h: &mut Harness<'static, App>) {
    h.get_by_role_and_label(Role::Button, "Settings").click();
    h.run();
}

fn is_open(h: &Harness<'static, App>) -> bool {
    h.query_by_role_and_label(Role::Dialog, "Settings")
        .is_some()
}

const CTRL: Modifiers = Modifiers::COMMAND;
const CTRL_SHIFT: Modifiers = Modifiers {
    shift: true,
    ..Modifiers::COMMAND
};

#[test]
fn the_gear_opens_settings_with_its_seven_pages() {
    let mut l = launch();
    assert!(!is_open(&l.harness));
    open_with_gear(&mut l.harness);
    assert!(is_open(&l.harness));
    for page in PAGES {
        l.harness.get_by_role_and_label(Role::ListItem, page);
    }
    // The pages are on the left of the fields.
    let page = l
        .harness
        .get_by_role_and_label(Role::ListItem, "Appearance");
    let theme = l.harness.get_by_role_and_label(Role::Label, "Theme");
    assert!(page.rect().max.x <= theme.rect().min.x);
}

#[test]
fn the_dialog_has_a_close_x_reset_and_close_and_no_ok_apply_or_cancel() {
    let mut l = launch();
    open_with_gear(&mut l.harness);
    let h = &l.harness;
    // The ✕ in the header and Close in the footer.
    assert_eq!(
        h.get_all_by_role_and_label(Role::Button, "Close").count(),
        2
    );
    h.get_by_role_and_label(Role::Button, "Reset Appearance to defaults");
    for absent in ["OK", "Apply", "Cancel"] {
        assert!(h.query_by_role_and_label(Role::Button, absent).is_none());
    }
}

#[test]
fn ctrl_comma_opens_settings_at_launch() {
    let mut l = launch();
    l.harness.key_press_modifiers(CTRL, Key::Comma);
    l.harness.run();
    assert!(is_open(&l.harness));
}

#[test]
fn close_the_x_and_escape_each_close_the_dialog() {
    let mut l = launch();
    let h = &mut l.harness;
    open_with_gear(h);
    let footer = h
        .get_all_by_role_and_label(Role::Button, "Close")
        .max_by(|a, b| a.rect().min.y.total_cmp(&b.rect().min.y))
        .unwrap();
    footer.click();
    h.run();
    assert!(!is_open(h));

    open_with_gear(h);
    let x = h
        .get_all_by_role_and_label(Role::Button, "Close")
        .min_by(|a, b| a.rect().min.y.total_cmp(&b.rect().min.y))
        .unwrap();
    x.click();
    h.run();
    assert!(!is_open(h));

    open_with_gear(h);
    h.key_press(Key::Escape);
    h.run();
    assert!(!is_open(h));
}

#[test]
fn another_page_shows_its_own_reset() {
    let mut l = launch();
    let h = &mut l.harness;
    open_with_gear(h);
    h.get_by_role_and_label(Role::ListItem, "Browsing").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Reset Browsing to defaults");
    h.get_by_role_and_label(Role::ListItem, "Keys").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Reset all keys");
}

#[test]
fn show_hidden_entries_is_on_the_browsing_page_and_ctrl_h_toggles_it() {
    let mut l = launch();
    let h = &mut l.harness;
    open_with_gear(h);
    h.get_by_role_and_label(Role::ListItem, "Browsing").click();
    h.run();
    h.get_by_role_and_label(Role::Switch, "Show hidden entries")
        .click();
    h.run();
    assert!(l.store.load().browsing.show_hidden);
    h.get_by_role_and_label(Role::Button, "Reset Browsing to defaults")
        .click();
    h.run();
    assert!(!l.store.load().browsing.show_hidden);
    h.key_press(Key::Escape);
    h.run();

    h.key_press_modifiers(Modifiers::COMMAND, Key::H);
    h.run();
    assert!(l.store.load().browsing.show_hidden);
    h.key_press_modifiers(Modifiers::COMMAND, Key::H);
    h.run();
    assert!(!l.store.load().browsing.show_hidden);
}

#[test]
fn density_applies_on_the_next_frame_and_survives_a_restart() {
    let mut l = launch();
    open_with_gear(&mut l.harness);
    l.harness
        .get_by_role_and_label(Role::RadioButton, "Compact")
        .click();
    l.harness.run();
    assert_eq!(l.harness.ctx.tokens().row_height(), 26.0);
    assert_eq!(l.store.load().appearance.density, Density::Compact);

    let again = launch_with(Some(&l.store.load()));
    assert_eq!(again.harness.ctx.tokens().density, Density::Compact);
}

#[test]
fn the_theme_follows_the_choice() {
    let mut l = launch();
    open_with_gear(&mut l.harness);
    l.harness
        .get_by_role_and_label(Role::RadioButton, "Dark")
        .click();
    l.harness.run();
    let ctx = &l.harness.ctx;
    assert_eq!(
        ctx.options(|o| o.theme_preference),
        egui::ThemePreference::Dark
    );
    assert_eq!(ctx.tokens().theme, egui::Theme::Dark);
    assert_eq!(l.store.load().appearance.theme, ThemeChoice::Dark);
}

#[test]
fn the_accent_picked_colours_the_chrome() {
    let mut l = launch();
    open_with_gear(&mut l.harness);
    l.harness.get_by_value("Blue").click();
    l.harness.run();
    l.harness.get_by_label("Teal").click();
    l.harness.run();
    let ctx = &l.harness.ctx;
    let t = ctx.tokens();
    let teal = Settings {
        accent: Accent::Teal,
        ..Settings::default()
    };
    assert_eq!(t.color.accent, Tokens::new(t.theme, &teal).color.accent);
    assert_eq!(l.store.load().appearance.accent, Accent::Teal);
}

#[test]
fn the_appearance_fields_show_their_defaults() {
    let mut l = launch();
    open_with_gear(&mut l.harness);
    let h = &l.harness;
    for name in [
        "Theme",
        "Accent",
        "UI scale",
        "Density",
        "UI font",
        "Monospace font",
        "Show icons",
        "Animate",
        "Tab bar edge",
    ] {
        h.get_by_role_and_label(Role::Label, name);
    }
    h.get_by_value("100 %");
    h.get_by_value("Top");
    h.get_by_value("Default (Inter)");
    h.get_by_value("Default (JetBrains Mono)");
    assert_eq!(
        h.get_all_by_role_and_label(Role::RadioButton, "Follow system")
            .count(),
        2
    );
}

#[test]
fn reset_appearance_puts_every_appearance_setting_back() {
    let mut saved = AppSettings::default();
    saved.appearance.density = Density::Compact;
    saved.appearance.accent = Accent::Pink;
    saved.appearance.ui_scale = 150;
    let mut l = launch_with(Some(&saved));
    open_with_gear(&mut l.harness);
    l.harness
        .get_by_role_and_label(Role::Button, "Reset Appearance to defaults")
        .click();
    // The scale change lays the window out again over a few frames.
    l.harness.run_steps(8);
    assert_eq!(l.store.load(), AppSettings::default());
    assert_eq!(l.harness.ctx.tokens().density, Density::Comfortable);
    assert_eq!(l.harness.ctx.zoom_factor(), 1.0);
}

#[test]
fn saved_settings_apply_at_launch() {
    let mut saved = AppSettings::default();
    saved.appearance.theme = ThemeChoice::Light;
    saved.appearance.density = Density::Compact;
    saved.appearance.ui_scale = 125;
    let l = launch_with(Some(&saved));
    let ctx = &l.harness.ctx;
    assert_eq!(ctx.tokens().theme, egui::Theme::Light);
    assert_eq!(ctx.tokens().density, Density::Compact);
    assert_eq!(ctx.zoom_factor(), 1.25);
}

#[test]
fn ctrl_shift_plus_minus_and_zero_step_the_ui_scale() {
    let mut l = launch();
    let h = &mut l.harness;
    let press = |h: &mut Harness<'static, App>, key| {
        h.key_press_modifiers(CTRL_SHIFT, key);
        h.run();
        h.ctx.zoom_factor()
    };
    assert_eq!(press(h, Key::Equals), 1.25);
    assert_eq!(l.store.load().appearance.ui_scale, 125);
    let h = &mut l.harness;
    assert_eq!(press(h, Key::Minus), 1.0);
    assert_eq!(press(h, Key::Minus), 0.75);
    assert_eq!(press(h, Key::Minus), 0.75, "75 % is the smallest");
    assert_eq!(press(h, Key::Num0), 1.0);
    for _ in 0..6 {
        press(h, Key::Equals);
    }
    assert_eq!(h.ctx.zoom_factor(), 2.0, "200 % is the largest");
}

#[test]
fn ctrl_plus_and_minus_alone_leave_the_ui_scale_to_preview_zoom() {
    let mut l = launch();
    let h = &mut l.harness;
    for key in [Key::Equals, Key::Plus, Key::Minus] {
        h.key_press_modifiers(CTRL, key);
        h.run();
        assert_eq!(h.ctx.zoom_factor(), 1.0, "{key:?}");
    }
}

#[test]
fn a_font_that_is_not_installed_warns_and_the_bundled_face_is_used() {
    let mut saved = AppSettings::default();
    saved.appearance.ui_font = Some("Nowhere Sans".into());
    saved.appearance.mono_font = Some("Nowhere Mono".into());
    let mut l = launch_with(Some(&saved));
    let report = fonts::report(&l.harness.ctx).expect("fonts installed");
    assert_eq!(report.ui, fonts::Outcome::NotInstalled);
    assert_eq!(report.mono, fonts::Outcome::NotInstalled);
    open_with_gear(&mut l.harness);
    let h = &l.harness;
    h.get_by_label("“Nowhere Sans” is not installed. Using the bundled Inter.");
    h.get_by_label("“Nowhere Mono” is not installed. Using the bundled JetBrains Mono.");
    h.get_by_value("Nowhere Sans");
}

#[test]
fn the_settings_shortcut_also_closes_the_dialog() {
    let mut l = launch();
    let h = &mut l.harness;
    h.key_press_modifiers(CTRL, Key::Comma);
    h.run();
    h.key_press_modifiers(CTRL, Key::Comma);
    h.run();
    assert!(!is_open(h));
}

#[test]
fn the_download_page_has_its_two_switches_and_the_follow_setting_shared_with_search() {
    let mut l = launch();
    let h = &mut l.harness;
    open_with_gear(h);
    h.get_by_role_and_label(Role::ListItem, "Download").click();
    h.run();
    h.get_by_label("No effect on Windows");
    h.get_by_label("Shared with Search");
    h.get_by_role_and_label(Role::Switch, "Keep modification time")
        .click();
    h.run();
    h.get_by_role_and_label(Role::Switch, "Keep executable bit")
        .click();
    h.run();
    h.get_by_role_and_label(Role::Switch, "Follow symlinked directories")
        .click();
    h.run();
    let saved = l.store.load();
    assert!(!saved.download.keep_modified);
    assert!(!saved.download.keep_executable);
    assert!(saved.search.follow_links);

    h.get_by_role_and_label(Role::Button, "Reset Download to defaults")
        .click();
    h.run();
    let saved = l.store.load();
    assert_eq!(saved.download, AppSettings::default().download);
    assert_eq!(saved.search, AppSettings::default().search);
}

#[test]
fn the_preview_page_picks_the_scratch_folder() {
    let mut l = launch();
    let h = &mut l.harness;
    open_with_gear(h);
    h.get_by_role_and_label(Role::ListItem, "Preview").click();
    h.run();
    h.get_by_label("Scratch folder");
    h.get_by_label("In memory");
    h.get_by_label("System temp").click();
    h.run();
    assert_eq!(l.store.load().preview.scratch, ScratchChoice::Temp);
    l.harness.get_by_label("Chosen").click();
    l.harness.run();
    l.harness
        .get_by_role_and_label(Role::TextInput, "Scratch folder path");
    l.harness.get_by_role_and_label(Role::Button, "Browse…");
    assert_eq!(l.store.load().preview.scratch, ScratchChoice::Chosen);
}

#[test]
fn retry_saves_settings_edited_before_a_write_failure() {
    let mut l = launch();
    std::fs::create_dir(l.store.path()).unwrap();
    let h = &mut l.harness;
    open_with_gear(h);
    h.get_by_role_and_label(Role::ListItem, "Download").click();
    h.run();
    h.get_by_role_and_label(Role::Switch, "Keep modification time")
        .click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Retry");
    std::fs::remove_dir(l.store.path()).unwrap();
    h.get_by_role_and_label(Role::Button, "Retry").click();
    h.run();
    assert!(h.query_by_role_and_label(Role::Button, "Retry").is_none());
    assert!(!l.store.load().download.keep_modified);
}
