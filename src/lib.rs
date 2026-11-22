//! sshexplorer: browse a remote filesystem over SSH, preview files and download them.
//!
//! - [`theme`] and [`widgets`] hold all styling (ADR-0001). Nothing else reads egui's
//!   visuals or spacing, names a colour or a size, or calls a stock widget that has an
//!   sshexplorer one; `tests/source_scan` enforces it.
//! - [`app`] is the window: the toolbar and the Sessions as tabs. [`connect`] holds the
//!   Host dialog and the prompts while a Session opens; [`session`] an open Session.
//! - [`ssh`] is the SSH stack (russh, ADR-0003) on its own runtime, off the frame
//!   thread; [`known_hosts`] checks host keys against the operator's `known_hosts`.
//! - [`preview`] is the Preview panel's model; [`source`] highlights source for it, and
//!   [`scratch`] keeps the bytes it fetched.
//! - [`settings`] holds the app settings saved between runs, and [`keymap`] every key
//!   binding; [`binding_editor`] is Settings → Keys, where the operator rebinds them.

pub mod app;
pub mod binding_editor;
pub mod connect;
pub mod folder;
pub mod hosts;
pub mod icons;
pub mod keymap;
pub mod kind;
pub mod known_hosts;
pub mod preview;
pub mod preview_panel;
pub mod scratch;
pub mod session;
pub mod settings;
pub mod source;
pub mod ssh;
pub mod theme;
pub mod widgets;
