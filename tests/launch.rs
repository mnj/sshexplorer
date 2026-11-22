//! Launch: the window opens in the Paper look with the toolbar and no Session.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use sshexplorer::app::{self, App};
use sshexplorer::settings::{AppSettings, ThemeChoice};
use sshexplorer::theme::{TokensExt as _, fonts};

fn launch() -> Harness<'static, App> {
    Harness::builder()
        .with_size(egui::vec2(1000.0, 700.0))
        .with_context_setup(app::setup)
        .build_ui_state(|ui, app: &mut App| app.ui(ui), App::default())
}

#[test]
fn launch_shows_the_toolbar_with_hosts_and_the_settings_gear() {
    let harness = launch();
    let hosts = harness.get_by_role_and_label(Role::Button, "Hosts");
    let gear = harness.get_by_role_and_label(Role::Button, "Settings");
    // The gear sits at the right end, past Hosts.
    assert!(gear.rect().min.x > hosts.rect().max.x);
    assert!(gear.rect().max.x > 900.0);
}

#[test]
fn launch_opens_no_session() {
    let harness = launch();
    assert!(harness.query_by_role(Role::Tab).is_none());
    assert!(harness.query_by_role(Role::TabList).is_none());
}

#[test]
fn launch_follows_the_system_theme_with_the_bundled_fonts() {
    // The harness picks its own theme after setup, so setup is checked on a bare context.
    let ctx = egui::Context::default();
    app::setup(&ctx);
    assert_eq!(
        ctx.options(|o| o.theme_preference),
        egui::ThemePreference::System
    );
    assert!(fonts::report(&ctx).is_some());
}

#[test]
fn launch_draws_in_the_paper_look_in_light_and_dark() {
    for (theme, choice) in [
        (egui::Theme::Light, ThemeChoice::Light),
        (egui::Theme::Dark, ThemeChoice::Dark),
    ] {
        let mut settings = AppSettings::default();
        settings.appearance.theme = choice;
        let harness = Harness::builder()
            .with_context_setup(app::setup)
            .build_ui_state(|ui, app: &mut App| app.ui(ui), App::with_settings(settings));
        let ctx = &harness.ctx;
        let t = ctx.tokens();
        assert_eq!(t.theme, theme);
        assert_eq!(ctx.global_style().visuals.panel_fill, t.color.surface.base);
    }
}
