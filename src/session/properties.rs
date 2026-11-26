//! File actions shared by Tree view, Folder view and Search matches.
use super::*;
use crate::widgets::Glyph;

pub(super) enum RowAction {
    Open,
    Download,
    CopyPath,
    Properties,
}

impl Session {
    /// Zero when a context download would first select this row.
    fn row_selection_size(&self, i: usize) -> usize {
        let (selected, count) = match &self.search {
            Some(searching) => (
                searching.matches.is_selected(i),
                searching.matches.selected_count(),
            ),
            None => (self.rows().is_selected(i), self.rows().selected_count()),
        };
        if selected { count } else { 0 }
    }

    pub(super) fn row_menu(
        &self,
        i: usize,
        response: &egui::Response,
        downloadable: bool,
        expanded: bool,
        frame: &Frame<'_>,
    ) -> Option<RowAction> {
        widgets::row_context_menu(response, |ui| {
            let keys = frame.keymap;
            let online = !self.is_disconnected();
            let count = self.row_selection_size(i);
            let mut picked = None;
            let (open, glyph) = if expanded {
                ("Collapse", Glyph::ChevronUp)
            } else {
                ("Open", Glyph::Open)
            };
            let download = if count > 1 {
                format!("Download {count} items")
            } else {
                "Download".into()
            };
            for (label, glyph, action, shortcut, enabled) in [
                (open, glyph, RowAction::Open, Some(Action::Open), online),
                (
                    download.as_str(),
                    Glyph::Download,
                    RowAction::Download,
                    Some(Action::Download),
                    online && downloadable,
                ),
                ("Copy path", Glyph::Copy, RowAction::CopyPath, None, true),
                ("Properties", Glyph::Info, RowAction::Properties, None, true),
            ] {
                if matches!(action, RowAction::Properties) {
                    widgets::menu_separator(ui);
                }
                let mut item = widgets::menu_item(label).glyph(glyph);
                if let Some(binding) = shortcut.and_then(|action| keys.binding(action)) {
                    item = item.shortcut(binding.label(cfg!(target_os = "macos")));
                }
                if ui.add_enabled(enabled, item).clicked() {
                    picked = Some(action);
                }
            }
            // The popup owns the shortcut target, even if another row was selected.
            // Enter on a focused menu item is handled by that button.
            if frame.keys && ui.is_enabled() && picked.is_none() {
                let shortcut = ui.input_mut(|input| {
                    if keys.take(input, Action::Download) {
                        (online && downloadable).then_some(RowAction::Download)
                    } else if keys.take(input, Action::Open) {
                        online.then_some(RowAction::Open)
                    } else {
                        None
                    }
                });
                if shortcut.is_some() {
                    ui.close();
                    picked = shortcut;
                }
            }
            picked
        })
        .and_then(|menu| menu.inner)
    }
}

pub(super) struct Properties {
    entry: Focused,
    kind: Kind,
    listing: Option<Request<Listing>>,
    error: Option<String>,
}

impl Session {
    pub(super) fn run_row_action(
        &mut self,
        action: RowAction,
        i: usize,
        ui: &egui::Ui,
        settings: &AppSettings,
    ) {
        let now = ui.input(|i| i.time);
        match action {
            RowAction::Open => self.apply(Command::DoubleClick(i), now),
            RowAction::Download => {
                if self.row_selection_size(i) == 0 {
                    self.apply(Command::Click(i), now);
                }
                self.download(self.download_options(settings));
            }
            RowAction::CopyPath => {
                if let Some(entry) = self.entry_at(i) {
                    ui.ctx().copy_text(entry.path);
                }
            }
            RowAction::Properties => {
                if let Some(entry) = self.entry_at(i) {
                    let kind = self.preview.kind_for(&entry);
                    // Search may have returned only a name and kind. Consult its parent
                    // listing for the same ownership and link metadata as browsing.
                    let listing = if self.search.is_some()
                        && self.tree.entry_of(&entry.path).is_none()
                        && !self.is_disconnected()
                    {
                        folder::parent(&entry.path).map(|(parent, _)| self.connection.list(parent))
                    } else {
                        None
                    };
                    self.properties = Some(Properties {
                        entry,
                        kind,
                        listing,
                        error: None,
                    });
                }
            }
        }
    }

    pub(super) fn properties_window(&mut self, ctx: &egui::Context) {
        let Some(props) = &mut self.properties else {
            return;
        };
        if let Some(result) = props.listing.as_ref().and_then(Request::try_take) {
            props.listing = None;
            match result {
                Ok(list) => {
                    if let Some(entry) = list
                        .entries
                        .into_iter()
                        .find(|e| e.name == props.entry.name)
                    {
                        if entry.is_dir || props.kind == Kind::Other {
                            props.kind = Kind::of(&entry.name, entry.is_dir);
                        }
                        props.entry.entry = Some(entry);
                    } else {
                        props.error = Some("Entry no longer listed".into());
                    }
                }
                Err(reason) => props.error = Some(reason),
            }
        }
        let mut open = true;
        widgets::window(self.id.with("properties"), "Properties")
            .open(&mut open)
            .centred()
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    details(ui, &props.entry, props.kind, &self.zone);
                    if let Some(reason) = &props.error {
                        ui.add(widgets::warning_message(reason));
                    }
                });
            });
        if !open {
            self.properties = None;
        }
    }
}

/// The metadata available for this entry.
fn details(
    ui: &mut egui::Ui,
    focused: &Focused,
    kind: crate::kind::Kind,
    zone: &jiff::tz::TimeZone,
) {
    let mut rows = vec![
        ("Name", focused.name.clone()),
        ("Path", focused.path.clone()),
        ("Kind", crate::preview::kind_name(kind).to_owned()),
    ];
    if let Some(e) = &focused.entry {
        if let Some(size) = e.size.filter(|_| !e.is_dir) {
            rows.push(("Size", size_text(size)));
        }
        if let Some(at) = e.modified {
            rows.push(("Modified", modified_text(at, zone)));
        }
        if let Some(mode) = e.permissions {
            rows.push(("Permissions", permissions_text(mode)));
        }
        // The names when the host gave them, else the ids.
        if let Some(owner) = e.owner.clone().or_else(|| e.uid.map(|id| id.to_string())) {
            rows.push(("Owner", owner));
        }
        if let Some(group) = e.group.clone().or_else(|| e.gid.map(|id| id.to_string())) {
            rows.push(("Group", group));
        }
        if let Some(target) = &e.link {
            rows.push(("Link to", target.clone()));
        }
    }
    widgets::form(ui.scope_id().with("details")).show(ui, |ui| {
        for (name, value) in rows {
            widgets::form_row(ui, name, |ui| ui.add(widgets::detail_value(name, value)));
        }
    });
}
