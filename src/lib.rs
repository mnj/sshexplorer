//! sshexplorer: browse a remote filesystem over SSH, preview files and download them.
//!
//! - [`theme`] and [`widgets`] hold all styling (ADR-0001). Nothing else reads egui's
//!   visuals or spacing, names a colour or a size, or calls a stock widget that has an
//!   sshexplorer one; `tests/source_scan` enforces it.
//! - [`app`] is the window: the toolbar and the Sessions as tabs. [`connect`] holds the
//!   Host dialog and the prompts while a Session opens; [`session`] an open Session.
//! - [`download`] copies a Selection into the Local folder in the background, and
//!   [`download_ui`] is its strip and prompts in the Session.
//! - [`ssh`] is the SSH stack (russh, ADR-0003) on its own runtime, off the frame
//!   thread; [`known_hosts`] checks host keys against the operator's `known_hosts`, and
//!   [`openssh_config`] reads and applies the operator's `~/.ssh/config` (ADR-0002).
//! - [`settings`] holds the app settings saved between runs, and [`keymap`] every key
//!   binding; [`binding_editor`] is Settings → Keys, where the operator rebinds them.
//! - [`folder`] and [`tree`] are the two views apart from egui, and [`refresh`] times
//!   Auto-refresh.

pub mod app;
pub mod archive;
pub mod binding_editor;
pub mod connect;
pub mod credentials;
pub mod download;
pub mod download_ui;
pub mod folder;
pub mod hosts;
pub mod icons;
pub mod image_view;
pub mod keymap;
pub mod kind;
pub mod known_hosts;
pub mod markdown;
pub mod media;
pub mod media_view;
pub mod openssh_config;
pub mod picture;
pub mod preview;
pub mod preview_panel;
pub mod refresh;
pub mod scratch;
pub mod session;
pub mod settings;
pub mod source;
pub mod ssh;
pub mod theme;
pub mod tree;
pub mod widgets;
pub mod zoom;
