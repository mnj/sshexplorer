//! An open Session: its connection, its tab label and its listing.
//!
//! This is the tracer's plain Folder view (issue #22): the names in the folder the Session
//! opened in, with `.` and `..` on top and directories first. Hidden entries are left
//! out, as the "Show hidden entries" setting is off by default.
// TODO(#27): columns, sort, natural order, focus and Selection, navigation, Tree view.

use std::path::PathBuf;

use crate::ssh::{self, Entry, Listing, Target};
use crate::widgets;

/// What a Session takes from the Host it was opened from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Details {
    /// The tab's label: a saved Host's display name, else [`label`].
    pub label: String,
    /// The tab's tooltip: for a saved Host, where it connects, port and all.
    pub tooltip: Option<String>,
    pub readonly: bool,
    pub local_folder: PathBuf,
    /// Why the OpenSSH config was not applied, naming the file and line.
    pub note: Option<String>,
}

pub struct Session {
    pub details: Details,
    listing: Listing,
    /// Kept for its connection, which closes when the Session is dropped.
    _connection: ssh::Session,
}

impl Session {
    pub fn new(details: Details, opened: ssh::Opened) -> Self {
        Session {
            details,
            listing: opened.home,
            _connection: opened.session,
        }
    }

    pub fn label(&self) -> &str {
        &self.details.label
    }

    /// The Folder view of the listed directory.
    pub fn ui(&self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                widgets::rows(ui, |ui| {
                    for name in folder_names(&self.listing.entries) {
                        ui.add(widgets::list_row(name));
                    }
                });
            });
    }
}

/// An unsaved Session's tab label: `account@address`, and `:port` off port 22.
pub fn label(target: &Target) -> String {
    let mut label = format!("{}@{}", target.account, target.address);
    if target.port != 22 {
        label.push_str(&format!(":{}", target.port));
    }
    label
}

/// What Folder view lists: `.` and `..`, then directories, then the rest, each by name
/// ignoring case, without hidden entries.
fn folder_names(entries: &[Entry]) -> Vec<&str> {
    let mut shown: Vec<&Entry> = entries
        .iter()
        .filter(|e| !e.name.starts_with('.'))
        .collect();
    shown.sort_by_cached_key(|e| (!e.is_dir, e.name.to_lowercase()));
    [".", ".."]
        .into_iter()
        .chain(shown.into_iter().map(|e| e.name.as_str()))
        .collect()
}
