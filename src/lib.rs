//! sshexplorer: browse a remote filesystem over SSH, preview files and download them.
//!
//! - [`theme`] and [`widgets`] hold all styling (ADR-0001). Nothing else reads egui's
//!   visuals or spacing, names a colour or a size, or calls a stock widget that has an
//!   sshexplorer one; `tests/source_scan` enforces it.
//! - [`app`] is the window: the toolbar and, later, the Sessions.

pub mod app;
pub mod icons;
pub mod keymap;
pub mod settings;
pub mod theme;
pub mod widgets;
