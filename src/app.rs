//! The main window: the toolbar along the top and, once one is open, the Sessions as tabs;
//! and the Settings dialog over it.
//!
//! Launch opens no Session and lists no Hosts. The toolbar's Hosts control does nothing
//! yet: the Host dialog hangs off it. The Settings gear, and Ctrl+, from anywhere, opens
//! the Settings dialog. Every change there applies on the next frame and is saved at once.

use crate::keymap::{Action, KeyMap};
use crate::settings::{
    Animate, AppSettings, Appearance, DEFAULT_UI_SCALE, Edge, Store, ThemeChoice, UI_SCALES,
};
use crate::theme::{self, Accent, Density, fonts};
use crate::widgets::{self, Glyph, Tooltip as _};

/// The Settings dialog's pages, in order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Appearance,
    Browsing,
    Search,
    Preview,
    Download,
    Connections,
    Keys,
}

impl Page {
    pub const ALL: [Page; 7] = [
        Page::Appearance,
        Page::Browsing,
        Page::Search,
        Page::Preview,
        Page::Download,
        Page::Connections,
        Page::Keys,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Page::Appearance => "Appearance",
            Page::Browsing => "Browsing",
            Page::Search => "Search",
            Page::Preview => "Preview",
            Page::Download => "Download",
            Page::Connections => "Connections",
            Page::Keys => "Keys",
        }
    }

    /// The footer's reset button on this page.
    fn reset_label(self) -> String {
        match self {
            Page::Keys => "Reset all keys".to_owned(),
            page => format!("Reset {} to defaults", page.name()),
        }
    }
}

const PAGE_NAMES: [&str; 7] = [
    "Appearance",
    "Browsing",
    "Search",
    "Preview",
    "Download",
    "Connections",
    "Keys",
];

/// The Settings dialog while it is open.
struct SettingsDialog {
    page: usize,
    /// The installed families the font fields list, read when the dialog opened.
    ui_fonts: Vec<String>,
    mono_fonts: Vec<String>,
}

impl SettingsDialog {
    fn open(ctx: &egui::Context) -> SettingsDialog {
        let families = fonts::source(ctx).families();
        let names = |keep: fn(&fonts::Family) -> bool| {
            families
                .iter()
                .filter(|f| keep(f))
                .map(|f| f.name.clone())
                .collect()
        };
        SettingsDialog {
            page: 0,
            ui_fonts: names(fonts::Family::has_latin_regular),
            mono_fonts: names(fonts::Family::is_fixed_pitch),
        }
    }
}

/// The app's state across frames.
#[derive(Default)]
pub struct App {
    settings: AppSettings,
    /// Where settings are saved; `None` keeps them for this run only.
    store: Option<Store>,
    keymap: KeyMap,
    dialog: Option<SettingsDialog>,
    /// The Appearance last put on the context, so a change is applied once.
    applied: Option<Appearance>,
}

/// Readies `ctx` for the app, before its first frame: the bundled fonts, the tokens and
/// both themes' styles, following the system's light or dark. egui's own zoom keys are
/// turned off: Ctrl+= and Ctrl+- belong to Preview, and UI scale has its own keys.
pub fn setup(ctx: &egui::Context) {
    theme::install(ctx, egui::ThemePreference::System);
    ctx.options_mut(|o| o.zoom_with_keyboard = false);
}

impl App {
    /// The app with the settings `store` holds, saving every change back to it.
    pub fn with_store(store: Store) -> App {
        App {
            settings: store.load(),
            store: Some(store),
            ..App::default()
        }
    }

    /// The app with `settings`, kept for this run only.
    pub fn with_settings(settings: AppSettings) -> App {
        App {
            settings,
            ..App::default()
        }
    }

    pub fn settings(&self) -> &AppSettings {
        &self.settings
    }

    pub fn keymap(&self) -> &KeyMap {
        &self.keymap
    }

    /// Puts the settings on `ctx` if they are not there yet. The first frame does this
    /// itself; call it before then so the first frame is already drawn with them.
    pub fn apply(&mut self, ctx: &egui::Context) {
        let a = &self.settings.appearance;
        if self.applied.as_ref() == Some(a) {
            return;
        }
        theme::set(ctx, &a.theme_settings());
        ctx.set_theme(a.theme_preference());
        ctx.set_zoom_factor(a.zoom_factor());
        self.applied = Some(a.clone());
    }

    fn save(&self) {
        if let Some(store) = &self.store
            && let Err(err) = store.save(&self.settings)
        {
            eprintln!(
                "sshexplorer: cannot save settings to {}: {err}",
                store.path().display()
            );
        }
    }

    /// Draws one frame in `ui`, which covers the window.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.apply(&ctx);
        let before = self.settings.clone();
        self.keys(ui);
        let mut open_settings = false;
        widgets::toolbar(
            ui,
            |ui| {
                ui.add(widgets::button("Hosts").flat());
            },
            |ui| {
                open_settings = ui
                    .add(widgets::glyph_button(Glyph::Gear).name("Settings"))
                    .tooltip("Settings")
                    .clicked();
            },
        );
        if open_settings && self.dialog.is_none() {
            self.dialog = Some(SettingsDialog::open(&ctx));
        }
        self.settings_dialog(&ctx);
        if self.settings != before {
            self.save();
            self.apply(&ctx);
        }
    }

    /// The app-wide keys: Settings, and the UI scale.
    fn keys(&mut self, ui: &mut egui::Ui) {
        let map = &self.keymap;
        let [settings, up, down, reset] = ui.input_mut(|i| {
            [
                Action::Settings,
                Action::UiScaleUp,
                Action::UiScaleDown,
                Action::UiScaleReset,
            ]
            .map(|action| map.take(i, action))
        });
        if settings {
            self.dialog = match self.dialog.take() {
                Some(_) => None,
                None => Some(SettingsDialog::open(ui.ctx())),
            };
        }
        let a = &mut self.settings.appearance;
        if up {
            a.scale_up();
        }
        if down {
            a.scale_down();
        }
        if reset {
            a.ui_scale = DEFAULT_UI_SCALE;
        }
    }

    fn settings_dialog(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.dialog else {
            return;
        };
        let page = Page::ALL[dialog.page.min(Page::ALL.len() - 1)];
        let id = egui::Id::NULL.with("sshexplorer-settings");
        let report = fonts::report(ctx);
        let settings = &mut self.settings;
        let keymap = &mut self.keymap;
        let (ui_fonts, mono_fonts) = (&dialog.ui_fonts, &dialog.mono_fonts);
        let shown = widgets::pages_dialog(id, "Changes apply at once", &PAGE_NAMES)
            .reset_label(page.reset_label())
            .show(ctx, &mut dialog.page, |ui, page| match Page::ALL[page] {
                Page::Appearance => {
                    appearance(ui, &mut settings.appearance, report, ui_fonts, mono_fonts)
                }
                Page::Keys => keys_page(ui, keymap),
                _ => {
                    ui.add(widgets::form_name("Nothing to set here yet."));
                }
            });
        if shown.reset {
            match page {
                Page::Appearance => settings.appearance = Appearance::default(),
                Page::Keys => keymap.reset_all(),
                _ => {}
            }
        }
        if shown.close {
            self.dialog = None;
        }
    }
}

const THEMES: [(ThemeChoice, &str); 3] = [
    (ThemeChoice::System, "Follow system"),
    (ThemeChoice::Light, "Light"),
    (ThemeChoice::Dark, "Dark"),
];

const DENSITIES: [(Density, &str); 2] = [
    (Density::Comfortable, "Comfortable"),
    (Density::Compact, "Compact"),
];

const ANIMATES: [(Animate, &str); 3] = [
    (Animate::System, "Follow system"),
    (Animate::On, "On"),
    (Animate::Off, "Off"),
];

const EDGES: [(Edge, &str); 4] = [
    (Edge::Top, "Top"),
    (Edge::Left, "Left"),
    (Edge::Right, "Right"),
    (Edge::Bottom, "Bottom"),
];

fn percent(scale: u16) -> String {
    format!("{scale} %")
}

/// The Appearance page.
fn appearance(
    ui: &mut egui::Ui,
    a: &mut Appearance,
    report: Option<fonts::Report>,
    ui_fonts: &[String],
    mono_fonts: &[String],
) {
    widgets::form("appearance").show(ui, |ui| {
        widgets::form_row(ui, "Theme", |ui| {
            ui.add(widgets::segmented(&mut a.theme, &THEMES));
        });
        widgets::form_row(ui, "Accent", |ui| {
            widgets::dropdown("accent", a.accent.name())
                .name("Accent")
                .show(ui, |ui| {
                    for accent in Accent::ALL {
                        widgets::choice_value(ui, &mut a.accent, accent, accent.name());
                    }
                });
        });
        widgets::form_row(ui, "UI scale", |ui| {
            widgets::dropdown("ui-scale", percent(a.ui_scale))
                .name("UI scale")
                .show(ui, |ui| {
                    for scale in UI_SCALES {
                        widgets::choice_value(ui, &mut a.ui_scale, scale, percent(scale));
                    }
                });
        });
        widgets::form_row(ui, "Density", |ui| {
            ui.add(widgets::segmented(&mut a.density, &DENSITIES));
        });
        let ui_outcome = report.map(|r| r.ui);
        widgets::form_row(ui, "UI font", |ui| {
            font_field(
                ui,
                "ui-font",
                "UI font",
                &mut a.ui_font,
                ui_fonts,
                fonts::BUNDLED_UI,
                ui_outcome,
            );
        });
        let mono_outcome = report.map(|r| r.mono);
        widgets::form_row(ui, "Monospace font", |ui| {
            font_field(
                ui,
                "mono-font",
                "Monospace font",
                &mut a.mono_font,
                mono_fonts,
                fonts::BUNDLED_MONO,
                mono_outcome,
            );
        });
        widgets::form_row(ui, "Show icons", |ui| {
            ui.add(widgets::switch(&mut a.show_icons, "").name("Show icons"));
        });
        widgets::form_row(ui, "Animate", |ui| {
            ui.add(widgets::segmented(&mut a.animate, &ANIMATES));
        });
        widgets::form_row(ui, "Tab bar edge", |ui| {
            let edge = EDGES.iter().find(|(e, _)| *e == a.tab_bar_edge);
            let text = edge.map_or("Top", |(_, name)| name);
            widgets::dropdown("tab-bar-edge", text)
                .name("Tab bar edge")
                .show(ui, |ui| {
                    for (edge, name) in EDGES {
                        widgets::choice_value(ui, &mut a.tab_bar_edge, edge, name);
                    }
                });
        });
    });
}

/// A font setting: the bundled face or an installed family, and under it what became of
/// a family that cannot be used.
fn font_field(
    ui: &mut egui::Ui,
    id: &str,
    name: &str,
    chosen: &mut Option<String>,
    families: &[String],
    bundled: &str,
    outcome: Option<fonts::Outcome>,
) {
    let default = format!("Default ({bundled})");
    let mut choices = vec![widgets::Choice::new(&default).rule_below()];
    choices.extend(families.iter().map(|f| widgets::Choice::new(f)));
    let selected = match chosen.as_deref() {
        None => Some(0),
        Some(name) => families.iter().position(|f| f == name).map(|i| i + 1),
    };
    let text = chosen.clone().unwrap_or_else(|| default.clone());
    ui.vertical(|ui| {
        let picked = widgets::filter_dropdown(id, text)
            .name(name)
            .hint("Filter")
            .none_text("No installed font matches")
            .show(ui, &choices, selected);
        match picked.index {
            Some(0) => *chosen = None,
            Some(i) => *chosen = families.get(i - 1).cloned(),
            None => {}
        }
        if let (Some(family), Some(outcome)) = (chosen.as_deref(), outcome) {
            let why = match outcome {
                fonts::Outcome::NotInstalled => Some("is not installed"),
                fonts::Outcome::Rejected(_) => Some("cannot be used"),
                _ => None,
            };
            if let Some(why) = why {
                let message = format!("“{family}” {why}. Using the bundled {bundled}.");
                ui.add(widgets::warning_message(message));
            }
        }
    });
}

/// The Keys page: every binding by area, read-only until the binding editor.
fn keys_page(ui: &mut egui::Ui, keymap: &KeyMap) {
    let mac = ui.ctx().os() == egui::os::OperatingSystem::Mac;
    for area in crate::keymap::Area::ALL {
        let rows: Vec<(String, String)> = Action::ALL
            .into_iter()
            .filter(|a| a.area() == area)
            .map(|a| {
                let key = keymap
                    .binding(a)
                    .map_or("unbound".to_owned(), |b| b.label(mac));
                (key, a.name().to_owned())
            })
            .collect();
        if rows.is_empty() {
            continue;
        }
        ui.add(widgets::form_name(area.name()));
        widgets::key_list(ui, area.name(), &rows);
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        App::ui(self, ui);
    }
}
