//! The theme: `Tokens` built from light/dark, Density and fonts, read through
//! `ui.tokens()`; egui's `Style` generated from them; and following the OS by default.

use egui::{Color32, FontFamily, TextStyle, Theme, ThemePreference};
use sshexplorer::theme::{self, Density, Families, Settings, Tokens, TokensExt as _};

fn tokens(theme: Theme, density: Density) -> Tokens {
    let settings = Settings {
        density,
        ..Settings::default()
    };
    Tokens::new(theme, &settings)
}

#[test]
fn a_row_is_30_px_comfortable_and_26_px_compact() {
    assert_eq!(tokens(Theme::Dark, Density::Comfortable).row_height(), 30.0);
    assert_eq!(tokens(Theme::Dark, Density::Compact).row_height(), 26.0);
}

#[test]
fn colours_and_shadows_follow_light_and_dark_and_nothing_else() {
    let dark = tokens(Theme::Dark, Density::Comfortable);
    let light = tokens(Theme::Light, Density::Comfortable);
    // The neutral window in each.
    assert_eq!(dark.color.surface.base, Color32::from_rgb(0x11, 0x11, 0x13));
    assert_eq!(
        light.color.surface.base,
        Color32::from_rgb(0xf3, 0xf3, 0xf1)
    );
    assert_ne!(dark.elevation, light.elevation);
    assert_eq!(dark.space, light.space);
    assert_eq!(dark.ty, light.ty);
    assert_eq!(dark.radius, light.radius);
    assert_eq!(dark.motion, light.motion);
    // Density moves no colour.
    let compact = tokens(Theme::Dark, Density::Compact);
    assert_eq!(compact.color, dark.color);
    assert_eq!(compact.elevation, dark.elevation);
}

#[test]
fn density_changes_paddings_and_through_them_the_heights_and_nothing_else() {
    let a = tokens(Theme::Dark, Density::Comfortable);
    let b = tokens(Theme::Dark, Density::Compact);
    let heights = |t: &Tokens| [t.control_height(), t.row_height(), t.tab_height()];
    assert_eq!(heights(&a), [30.0, 30.0, 34.0]);
    assert_eq!(heights(&b), [26.0, 26.0, 30.0]);
    // The status bar ignores Density.
    assert_eq!(a.status_bar_height(), 24.0);
    assert_eq!(b.status_bar_height(), 24.0);
    assert_eq!(a.ty, b.ty);
    assert_eq!(a.radius, b.radius);
    assert_eq!(a.color, b.color);
}

#[test]
fn the_body_is_13_px_and_the_type_scale_hangs_off_it() {
    let size_line = |t: &theme::Text| (t.size, t.line);
    let t = Tokens::new(Theme::Dark, &Settings::default()).ty;
    assert_eq!(size_line(&t.small), (11.0, 16.0));
    assert_eq!(size_line(&t.body), (13.0, 20.0));
    assert_eq!(size_line(&t.body_strong), (13.0, 20.0));
    assert_eq!(size_line(&t.title), (13.0, 20.0));
    assert_eq!(size_line(&t.heading), (16.0, 24.0));
    assert_eq!(size_line(&t.mono), (12.0, 20.0));
    let format = t.body.format(Color32::WHITE);
    assert_eq!(format.line_height, Some(20.0));
}

#[test]
fn each_weight_is_a_family_of_its_own() {
    let name = |n: &str| FontFamily::Name(n.into());
    let t = Tokens::new(Theme::Dark, &Settings::default()).ty;
    assert_eq!(t.body.family, name("sshexplorer-ui-regular"));
    assert_eq!(t.body_strong.family, name("sshexplorer-ui-medium"));
    assert_eq!(t.title.family, name("sshexplorer-ui-semibold"));
    assert_eq!(t.mono.family, name("sshexplorer-mono"));
    assert_eq!(t.numeric.family, name("sshexplorer-ui-numeric"));
    let t = Tokens::with_families(Theme::Dark, &Settings::default(), &Families::egui()).ty;
    assert_eq!(t.title.family, FontFamily::Proportional);
    assert_eq!(t.mono.family, FontFamily::Monospace);
}

#[test]
fn hover_and_press_fills_go_over_anything_with_one_function() {
    let t = tokens(Theme::Light, Density::Comfortable);
    let c = &t.color;
    assert_eq!(theme::over(c.accent.solid, c.surface.base), c.accent.solid);
    let half = Color32::from_black_alpha(128);
    assert_eq!(theme::over(half, Color32::WHITE), Color32::from_gray(127));
    let hovered = theme::over(c.fill.hover, c.surface.card);
    assert!(hovered.is_opaque());
}

#[test]
fn the_egui_style_is_generated_from_the_tokens() {
    for theme in [Theme::Dark, Theme::Light] {
        for density in [Density::Comfortable, Density::Compact] {
            let t = tokens(theme, density);
            let style = theme::style(&t);
            let v = &style.visuals;
            assert_eq!(v.dark_mode, theme == Theme::Dark);
            assert_eq!(v.panel_fill, t.color.surface.base);
            assert_eq!(v.window_fill, t.color.surface.floating);
            assert_eq!(v.text_color(), t.color.text.primary);
            assert_eq!(v.weak_text_color(), t.color.text.muted);
            assert_eq!(v.selection.bg_fill, t.color.accent.subtle);
            assert_eq!(v.window_shadow, t.elevation.window);
            assert_eq!(v.popup_shadow, t.elevation.popup);
            assert_eq!(v.widgets.inactive.weak_bg_fill, t.color.fill.control);
            assert_eq!(v.widgets.active.bg_stroke.color, t.color.stroke.focus_ring);
            let s = &style.spacing;
            assert_eq!(s.interact_size.y, t.control_height());
            assert_eq!(s.item_spacing.x, t.space.gap);
            assert_eq!(TextStyle::Body.resolve(&style), t.ty.body.font_id());
            assert_eq!(TextStyle::Monospace.resolve(&style), t.ty.mono.font_id());
            assert_eq!(style.animation_time, t.motion.menu.enter);
        }
    }
}

#[test]
fn installing_the_theme_sets_both_variants_and_follows_the_system() {
    let ctx = egui::Context::default();
    theme::install(&ctx, ThemePreference::System);
    assert_eq!(ctx.options(|o| o.theme_preference), ThemePreference::System);
    for theme in [Theme::Light, Theme::Dark] {
        ctx.set_theme(theme);
        let t = ctx.tokens();
        assert_eq!(t.theme, theme);
        assert_eq!(ctx.global_style().visuals, theme::style(&t).visuals);
    }
}

#[test]
fn installing_the_theme_installs_inter_and_jetbrains_mono() {
    let ctx = egui::Context::default();
    theme::install(&ctx, ThemePreference::Dark);
    let report = theme::fonts::report(&ctx).expect("fonts are installed with the theme");
    assert_eq!(
        (report.ui, report.mono),
        (
            theme::fonts::Outcome::Bundled,
            theme::fonts::Outcome::Bundled
        )
    );
    assert_eq!(theme::fonts::BUNDLED_UI, "Inter");
    assert_eq!(theme::fonts::BUNDLED_MONO, "JetBrains Mono");
}

#[test]
fn the_accent_colours_the_accent_and_the_focus_ring_and_nothing_else() {
    use theme::Accent;
    for theme in [Theme::Light, Theme::Dark] {
        let with = |accent| {
            let settings = Settings {
                accent,
                ..Settings::default()
            };
            Tokens::new(theme, &settings).color
        };
        let blue = with(Accent::Blue);
        for accent in Accent::ALL {
            let c = with(accent);
            // Error, warning and success are fixed hues; surfaces, text and fills too.
            assert_eq!(c.error, blue.error);
            assert_eq!(c.warning, blue.warning);
            assert_eq!(c.success, blue.success);
            assert_eq!(c.surface, blue.surface);
            assert_eq!(c.text, blue.text);
            assert_eq!(c.fill, blue.fill);
            assert_eq!(c.stroke.hairline, blue.stroke.hairline);
            if accent != Accent::Blue {
                assert_ne!(c.accent, blue.accent, "{accent:?} in {theme:?}");
                assert_ne!(c.stroke.focus_ring, blue.stroke.focus_ring);
            }
        }
        // Each preset is its own colour.
        let solids: std::collections::BTreeSet<_> = Accent::ALL
            .iter()
            .map(|a| with(*a).accent.solid.to_array())
            .collect();
        assert_eq!(solids.len(), Accent::ALL.len());
    }
}

#[test]
fn blue_is_the_default_accent_and_keeps_papers_blue() {
    let light = Tokens::new(Theme::Light, &Settings::default()).color;
    assert_eq!(light.accent.solid, Color32::from_rgb(0x1f, 0x5f, 0xbf));
    assert_eq!(light.stroke.focus_ring, Color32::from_rgb(0x1f, 0x5f, 0xbf));
    let dark = Tokens::new(Theme::Dark, &Settings::default()).color;
    assert_eq!(dark.accent.text, Color32::from_rgb(0x8f, 0xba, 0xf8));
    assert_eq!(dark.stroke.focus_ring, Color32::from_rgb(0x8f, 0xba, 0xf8));
}

#[test]
fn the_accent_reaches_selection_links_and_the_text_cursor_in_the_egui_style() {
    let settings = Settings {
        accent: theme::Accent::Pink,
        ..Settings::default()
    };
    let t = Tokens::new(Theme::Light, &settings);
    let v = theme::style(&t).visuals;
    assert_eq!(v.selection.bg_fill, t.color.accent.subtle);
    assert_eq!(v.hyperlink_color, t.color.accent.text);
    assert_eq!(v.text_cursor.stroke.color, t.color.accent.text);
    assert_eq!(v.widgets.active.bg_stroke.color, t.color.stroke.focus_ring);
}

#[test]
fn the_installed_font_index_finds_a_listed_family_by_name_in_any_case() {
    use theme::fonts::{Installed, Source as _};
    let installed = Installed::load();
    let families = installed.families();
    let Some(first) = families.first() else {
        return; // A machine with no fonts has nothing to look up.
    };
    let found = installed
        .family(&first.name.to_uppercase())
        .expect("a listed family is found");
    assert_eq!(found.name, first.name);
    assert_eq!(found.faces.len(), first.faces.len());
    assert!(installed.family("No Such Family 0xdead").is_none());
}
