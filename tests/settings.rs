//! App settings persist in a file and load at launch; unknown or missing fields fall back
//! to their defaults.

use egui::{Key, Modifiers};
use sshexplorer::keymap::{Action, Binding};
use sshexplorer::settings::{Animate, AppSettings, Edge, Store, ThemeChoice};
use sshexplorer::theme::{Accent, Density};

fn store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let store = Store::at(dir.path().join("config").join("settings.toml"));
    (dir, store)
}

#[test]
fn the_appearance_defaults_are_the_specs() {
    let a = AppSettings::default().appearance;
    assert_eq!(a.theme, ThemeChoice::System);
    assert_eq!(a.accent, Accent::Blue);
    assert_eq!(a.ui_scale, 100);
    assert_eq!(a.density, Density::Comfortable);
    assert_eq!(a.ui_font, None);
    assert_eq!(a.mono_font, None);
    assert!(a.show_icons);
    assert_eq!(a.animate, Animate::System);
    assert_eq!(a.tab_bar_edge, Edge::Top);
}

#[test]
fn hidden_entries_are_not_shown_by_default_and_the_choice_is_saved() {
    assert!(!AppSettings::default().browsing.show_hidden);
    let (_dir, store) = store();
    let mut settings = AppSettings::default();
    settings.browsing.show_hidden = true;
    store.save(&settings).expect("saved");
    assert!(store.load().browsing.show_hidden);
}

#[test]
fn the_preview_defaults_are_the_specs_and_are_saved() {
    use sshexplorer::settings::ScratchChoice;

    let p = AppSettings::default().preview;
    assert_eq!(p.size_cap, 10);
    assert_eq!(p.size_cap_bytes(), 10 << 20);
    assert!(!p.autoplay);
    assert_eq!(p.scratch, ScratchChoice::Memory);
    assert_eq!(p.scratch_dir, None);

    let (_dir, store) = store();
    let mut settings = AppSettings::default();
    settings.preview.size_cap = 64;
    settings.preview.scratch = ScratchChoice::Chosen;
    settings.preview.scratch_dir = Some("/var/tmp/peek".into());
    store.save(&settings).expect("saved");
    assert_eq!(store.load().preview, settings.preview);

    // A cap that is not a whole number of MiB from 1 falls back to 10, alone.
    std::fs::write(
        store.path(),
        "[preview]\nsize_cap = 0\nscratch = \"temp\"\n",
    )
    .unwrap();
    let loaded = store.load().preview;
    assert_eq!(loaded.size_cap, 10);
    assert_eq!(loaded.scratch, ScratchChoice::Temp);
}

#[test]
fn no_file_loads_the_defaults() {
    let (_dir, store) = store();
    assert_eq!(store.load(), AppSettings::default());
}

#[test]
fn saved_settings_load_again() {
    let (_dir, store) = store();
    let mut settings = AppSettings::default();
    let a = &mut settings.appearance;
    a.theme = ThemeChoice::Dark;
    a.accent = Accent::Teal;
    a.ui_scale = 150;
    a.density = Density::Compact;
    a.ui_font = Some("Noto Sans".into());
    a.mono_font = Some("Fira Code".into());
    a.show_icons = false;
    a.animate = Animate::Off;
    a.tab_bar_edge = Edge::Left;
    store.save(&settings).expect("saved");
    assert_eq!(store.load(), settings);
}

#[test]
fn missing_and_unknown_fields_fall_back_to_their_defaults() {
    let (_dir, store) = store();
    std::fs::create_dir_all(store.path().parent().unwrap()).unwrap();
    std::fs::write(
        store.path(),
        r#"
        future_section = { x = 1 }
        [appearance]
        accent = "violet"
        density = "chartreuse"
        ui_scale = "big"
        show_icons = false
        sparkles = true
        "#,
    )
    .unwrap();
    let a = store.load().appearance;
    assert_eq!(a.accent, Accent::Violet);
    assert!(!a.show_icons);
    // Wrong values fall back one field at a time.
    assert_eq!(a.density, Density::Comfortable);
    assert_eq!(a.ui_scale, 100);
    // Missing ones are the defaults.
    assert_eq!(a.theme, ThemeChoice::System);
    assert_eq!(a.tab_bar_edge, Edge::Top);
}

#[test]
fn rebound_keys_load_again() {
    let (_dir, store) = store();
    let mut settings = AppSettings::default();
    let shift_ctrl = Modifiers::COMMAND | Modifiers::SHIFT;
    let control_alt = Modifiers::CTRL | Modifiers::ALT;
    settings
        .keys
        .set(Action::Settings, Binding::new(shift_ctrl, Key::Plus));
    settings
        .keys
        .set(Action::CloseSession, Binding::new(control_alt, Key::F4));
    store.save(&settings).expect("saved");
    assert_eq!(store.load(), settings);
    let text = std::fs::read_to_string(store.path()).unwrap();
    assert!(text.contains(r#"Settings = "Ctrl+Shift+Plus""#), "{text}");
}

#[test]
fn a_key_entry_that_cannot_be_read_keeps_its_default() {
    let (_dir, store) = store();
    std::fs::create_dir_all(store.path().parent().unwrap()).unwrap();
    std::fs::write(
        store.path(),
        r#"
        [keys]
        Settings = "Ctrl+Escape"
        CloseSession = "Hyper+Q"
        UiScaleUp = 7
        UiScaleDown = ""
        UiScaleReset = "Alt+F9"
        NoSuchAction = "F1"
        "#,
    )
    .unwrap();
    let keys = store.load().keys;
    for action in [Action::Settings, Action::CloseSession, Action::UiScaleUp] {
        assert!(!keys.is_changed(action), "{action:?}");
    }
    assert_eq!(keys.binding(Action::UiScaleDown), None, "\"\" is unbound");
    assert_eq!(
        keys.binding(Action::UiScaleReset),
        Some(Binding::new(Modifiers::ALT, Key::F9))
    );
}

#[test]
fn a_ui_scale_off_the_steps_loads_as_the_nearest_step() {
    let (_dir, store) = store();
    std::fs::create_dir_all(store.path().parent().unwrap()).unwrap();
    for (written, loaded) in [(110, 100), (115, 125), (10, 75), (900, 200)] {
        std::fs::write(
            store.path(),
            format!("[appearance]\nui_scale = {written}\n"),
        )
        .unwrap();
        assert_eq!(store.load().appearance.ui_scale, loaded, "from {written}");
    }
}

#[test]
fn an_unreadable_file_loads_the_defaults_and_is_kept_aside_on_save() {
    let (_dir, store) = store();
    std::fs::create_dir_all(store.path().parent().unwrap()).unwrap();
    std::fs::write(store.path(), "this is [not toml").unwrap();
    assert_eq!(store.load(), AppSettings::default());
    store.save(&AppSettings::default()).expect("saved");
    let aside = store.path().with_extension("toml.bad");
    assert_eq!(std::fs::read_to_string(aside).unwrap(), "this is [not toml");
    assert_eq!(store.load(), AppSettings::default());
}

#[test]
fn the_platform_store_is_in_the_config_directory() {
    let store = Store::platform().expect("a config directory");
    assert!(store.path().ends_with("settings.toml"));
    assert!(
        store
            .path()
            .components()
            .any(|c| c.as_os_str() == "sshexplorer")
    );
}

#[test]
fn download_keeps_time_and_executable_bit_and_follows_no_links_by_default() {
    let s = AppSettings::default();
    assert!(s.download.keep_modified);
    assert!(s.download.keep_executable);
    assert!(!s.search.follow_links);
    let (_dir, store) = store();
    let mut settings = AppSettings::default();
    settings.download.keep_modified = false;
    settings.download.keep_executable = false;
    settings.search.follow_links = true;
    store.save(&settings).expect("saved");
    assert_eq!(store.load(), settings);
}
