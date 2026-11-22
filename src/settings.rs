//! App settings: what the Settings dialog changes, kept in `settings.toml` in the
//! platform config directory and loaded at launch.
//!
//! Each page of the dialog is one section of the file. A field that is missing, or whose
//! value cannot be read, takes its default and leaves the others as written; fields and
//! sections the app does not know are ignored. A file that is not TOML at all loads as
//! the defaults and is moved aside to `settings.toml.bad` before the next save, so it is
//! never lost.
//!
//! Hosts are not app settings and live in their own file.

use std::io;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};

use crate::keymap::KeyMap;
use crate::theme::{self, Accent, Density, fonts};

/// Every app setting, a section per Settings page. Later pages add their sections here.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    #[serde(deserialize_with = "lenient")]
    pub appearance: Appearance,
    /// The Keys page: the bindings that differ from the defaults.
    #[serde(deserialize_with = "lenient")]
    pub keys: KeyMap,
    #[serde(deserialize_with = "lenient")]
    pub browsing: Browsing,
    #[serde(deserialize_with = "lenient")]
    pub preview: PreviewSettings,
    #[serde(deserialize_with = "lenient")]
    pub connections: Connections,
    #[serde(deserialize_with = "lenient")]
    pub search: Search,
    #[serde(deserialize_with = "lenient")]
    pub download: Download,
}

/// The Search page. Its Follow symlinked directories is shared with Download.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Search {
    /// "Follow symlinked directories", for Search and Download alike. Off by default.
    #[serde(deserialize_with = "lenient")]
    pub follow_links: bool,
}

/// The Download page. It also shows [`Search::follow_links`].
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Download {
    #[serde(deserialize_with = "lenient_true")]
    pub keep_modified: bool,
    /// No effect on Windows.
    #[serde(deserialize_with = "lenient_true")]
    pub keep_executable: bool,
}

impl Default for Download {
    fn default() -> Self {
        Download {
            keep_modified: true,
            keep_executable: true,
        }
    }
}

/// The Preview page.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct PreviewSettings {
    /// The most of a file Preview fetches, in MiB: text and source show that much of a
    /// larger file. At least 1.
    #[serde(deserialize_with = "size_cap")]
    pub size_cap: u32,
    /// Whether audio and video start playing when focused. Stored only, until they play.
    #[serde(deserialize_with = "lenient")]
    pub autoplay: bool,
    #[serde(deserialize_with = "lenient")]
    pub scratch: ScratchChoice,
    /// The Chosen directory, kept when another choice is made.
    #[serde(deserialize_with = "lenient")]
    pub scratch_dir: Option<PathBuf>,
}

/// The size cap a new install and a reset have, in MiB.
pub const DEFAULT_SIZE_CAP: u32 = 10;

impl Default for PreviewSettings {
    fn default() -> Self {
        PreviewSettings {
            size_cap: DEFAULT_SIZE_CAP,
            autoplay: false,
            scratch: ScratchChoice::Memory,
            scratch_dir: None,
        }
    }
}

impl PreviewSettings {
    /// The size cap in bytes.
    pub fn size_cap_bytes(&self) -> u64 {
        u64::from(self.size_cap.max(1)) << 20
    }
}

/// Where Preview keeps fetched bytes (the Scratch folder).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScratchChoice {
    #[default]
    Memory,
    /// The system temp directory.
    Temp,
    /// The directory in `scratch_dir`.
    Chosen,
}

/// The Browsing page.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Browsing {
    /// Whether dot-names are listed, in every Session. Ctrl+H toggles it.
    #[serde(deserialize_with = "lenient")]
    pub show_hidden: bool,
    #[serde(deserialize_with = "lenient")]
    pub auto_refresh: AutoRefresh,
}

/// How often each Session refreshes by itself: off, or one of four intervals.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum AutoRefresh {
    #[default]
    #[serde(rename = "off")]
    Off,
    #[serde(rename = "10s")]
    Seconds10,
    #[serde(rename = "30s")]
    Seconds30,
    #[serde(rename = "1min")]
    Minute1,
    #[serde(rename = "5min")]
    Minutes5,
}

impl AutoRefresh {
    /// In the order the Browsing page lists them.
    pub const ALL: [AutoRefresh; 5] = [
        AutoRefresh::Off,
        AutoRefresh::Seconds10,
        AutoRefresh::Seconds30,
        AutoRefresh::Minute1,
        AutoRefresh::Minutes5,
    ];

    /// The interval; `None` when off.
    pub fn seconds(self) -> Option<f64> {
        match self {
            AutoRefresh::Off => None,
            AutoRefresh::Seconds10 => Some(10.0),
            AutoRefresh::Seconds30 => Some(30.0),
            AutoRefresh::Minute1 => Some(60.0),
            AutoRefresh::Minutes5 => Some(300.0),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            AutoRefresh::Off => "Off",
            AutoRefresh::Seconds10 => "10 s",
            AutoRefresh::Seconds30 => "30 s",
            AutoRefresh::Minute1 => "1 min",
            AutoRefresh::Minutes5 => "5 min",
        }
    }
}

/// The Connections page.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Connections {
    /// "Use ~/.ssh/config": Sessions and the Host dialog take values from the
    /// operator's OpenSSH config.
    #[serde(deserialize_with = "lenient_true")]
    pub use_ssh_config: bool,
    /// How often a Disconnected Session tries to reconnect by itself.
    #[serde(deserialize_with = "lenient")]
    pub reconnect_attempts: ReconnectAttempts,
}

impl Default for Connections {
    fn default() -> Self {
        Connections {
            use_ssh_config: true,
            reconnect_attempts: ReconnectAttempts::default(),
        }
    }
}

/// How many times a Disconnected Session tries to reconnect before it waits for the
/// operator.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum ReconnectAttempts {
    #[serde(rename = "off")]
    Off,
    #[serde(rename = "3")]
    Three,
    #[default]
    #[serde(rename = "5")]
    Five,
    #[serde(rename = "10")]
    Ten,
    #[serde(rename = "unlimited")]
    Unlimited,
}

impl ReconnectAttempts {
    /// In the order the Connections page lists them.
    pub const ALL: [ReconnectAttempts; 5] = [
        ReconnectAttempts::Off,
        ReconnectAttempts::Three,
        ReconnectAttempts::Five,
        ReconnectAttempts::Ten,
        ReconnectAttempts::Unlimited,
    ];

    /// The number of attempts; `None` for no limit.
    pub fn limit(self) -> Option<u32> {
        match self {
            ReconnectAttempts::Off => Some(0),
            ReconnectAttempts::Three => Some(3),
            ReconnectAttempts::Five => Some(5),
            ReconnectAttempts::Ten => Some(10),
            ReconnectAttempts::Unlimited => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            ReconnectAttempts::Off => "Off",
            ReconnectAttempts::Three => "3",
            ReconnectAttempts::Five => "5",
            ReconnectAttempts::Ten => "10",
            ReconnectAttempts::Unlimited => "Unlimited",
        }
    }
}

/// Light, dark, or whatever the OS uses.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}

/// Whether the chrome animates: as the OS's reduced-motion setting says, or on, or off.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Animate {
    #[default]
    System,
    On,
    Off,
}

/// The window edge the Session tabs run along.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Edge {
    #[default]
    Top,
    Left,
    Right,
    Bottom,
}

/// The UI scale steps, in percent.
pub const UI_SCALES: [u16; 6] = [75, 100, 125, 150, 175, 200];
/// The UI scale a new install and a reset have.
pub const DEFAULT_UI_SCALE: u16 = 100;

/// The step of [`UI_SCALES`] nearest `percent`; the larger of two equally near.
pub fn nearest_ui_scale(percent: u16) -> u16 {
    UI_SCALES
        .into_iter()
        .min_by_key(|step| (step.abs_diff(percent), u16::MAX - step))
        .unwrap_or(DEFAULT_UI_SCALE)
}

/// The Appearance page.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    #[serde(deserialize_with = "lenient")]
    pub theme: ThemeChoice,
    #[serde(deserialize_with = "lenient")]
    pub accent: Accent,
    /// In percent, one of [`UI_SCALES`].
    #[serde(deserialize_with = "ui_scale")]
    pub ui_scale: u16,
    #[serde(deserialize_with = "lenient")]
    pub density: Density,
    /// An installed family by name, or `None` for the bundled Inter.
    #[serde(deserialize_with = "lenient")]
    pub ui_font: Option<String>,
    /// An installed family by name, or `None` for the bundled JetBrains Mono.
    #[serde(deserialize_with = "lenient")]
    pub mono_font: Option<String>,
    #[serde(deserialize_with = "lenient_true")]
    pub show_icons: bool,
    #[serde(deserialize_with = "lenient")]
    pub animate: Animate,
    #[serde(deserialize_with = "lenient")]
    pub tab_bar_edge: Edge,
}

impl Default for Appearance {
    fn default() -> Self {
        Appearance {
            theme: ThemeChoice::System,
            accent: Accent::Blue,
            ui_scale: DEFAULT_UI_SCALE,
            density: Density::Comfortable,
            ui_font: None,
            mono_font: None,
            show_icons: true,
            animate: Animate::System,
            tab_bar_edge: Edge::Top,
        }
    }
}

impl Appearance {
    /// What the theme's tokens are built from.
    pub fn theme_settings(&self) -> theme::Settings {
        theme::Settings {
            density: self.density,
            accent: self.accent,
            fonts: fonts::Choice {
                ui: self.ui_font.clone(),
                mono: self.mono_font.clone(),
            },
            animate: match self.animate {
                Animate::System => None,
                Animate::On => Some(true),
                Animate::Off => Some(false),
            },
        }
    }

    /// The egui theme preference for [`Appearance::theme`].
    pub fn theme_preference(&self) -> egui::ThemePreference {
        match self.theme {
            ThemeChoice::System => egui::ThemePreference::System,
            ThemeChoice::Light => egui::ThemePreference::Light,
            ThemeChoice::Dark => egui::ThemePreference::Dark,
        }
    }

    /// The UI scale as egui's zoom factor.
    pub fn zoom_factor(&self) -> f32 {
        f32::from(nearest_ui_scale(self.ui_scale)) / 100.0
    }

    /// One step larger, up to the largest.
    pub fn scale_up(&mut self) {
        let now = nearest_ui_scale(self.ui_scale);
        self.ui_scale = UI_SCALES.into_iter().find(|s| *s > now).unwrap_or(now);
    }

    /// One step smaller, down to the smallest.
    pub fn scale_down(&mut self) {
        let now = nearest_ui_scale(self.ui_scale);
        self.ui_scale = UI_SCALES
            .into_iter()
            .rev()
            .find(|s| *s < now)
            .unwrap_or(now);
    }
}

/// A value that cannot be read as `T` is `T`'s default.
fn lenient<'de, D: Deserializer<'de>, T: DeserializeOwned + Default>(d: D) -> Result<T, D::Error> {
    let value = toml::Value::deserialize(d)?;
    Ok(T::deserialize(value).unwrap_or_default())
}

/// A flag that defaults to on.
fn lenient_true<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    let value = toml::Value::deserialize(d)?;
    Ok(value.as_bool().unwrap_or(true))
}

/// A size cap: a whole number of MiB from 1, or the default.
fn size_cap<'de, D: Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    let value = toml::Value::deserialize(d)?;
    Ok(value
        .as_integer()
        .filter(|n| (1..=i64::from(u32::MAX)).contains(n))
        .map_or(DEFAULT_SIZE_CAP, |n| n as u32))
}

/// A UI scale: the nearest step, or the default when it is not a number.
fn ui_scale<'de, D: Deserializer<'de>>(d: D) -> Result<u16, D::Error> {
    let value = toml::Value::deserialize(d)?;
    Ok(match value.as_integer() {
        Some(n) => nearest_ui_scale(n.clamp(0, i64::from(u16::MAX)) as u16),
        None => DEFAULT_UI_SCALE,
    })
}

/// Where app settings are kept: one TOML file.
#[derive(Clone, Debug)]
pub struct Store {
    path: PathBuf,
}

impl Store {
    /// The settings file at `path`.
    pub fn at(path: impl Into<PathBuf>) -> Store {
        Store { path: path.into() }
    }

    /// `settings.toml` in the platform config directory: `~/.config/sshexplorer` on
    /// Linux, `~/Library/Application Support/sshexplorer` on macOS,
    /// `%APPDATA%\sshexplorer\config` on Windows. `None` when the OS gives no home.
    pub fn platform() -> Option<Store> {
        let dirs = directories::ProjectDirs::from("", "", "sshexplorer")?;
        Some(Store::at(dirs.config_dir().join("settings.toml")))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The settings in the file; the defaults when it is missing or unreadable.
    pub fn load(&self) -> AppSettings {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// Writes `settings` to the file, creating its directory. A file there that is not
    /// TOML is first moved aside to `<name>.bad`.
    pub fn save(&self, settings: &AppSettings) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        if let Ok(old) = std::fs::read_to_string(&self.path)
            && toml::from_str::<toml::Table>(&old).is_err()
        {
            std::fs::rename(&self.path, self.aside())?;
        }
        let text = toml::to_string(settings).map_err(io::Error::other)?;
        let partial = self.path.with_extension("toml.partial");
        std::fs::write(&partial, text)?;
        std::fs::rename(&partial, &self.path)
    }

    fn aside(&self) -> PathBuf {
        let mut name = self.path.file_name().unwrap_or_default().to_owned();
        name.push(".bad");
        self.path.with_file_name(name)
    }
}
