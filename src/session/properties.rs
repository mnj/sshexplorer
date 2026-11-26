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
        let t = ctx.tokens();
        let margin = 2.0 * (t.space.dialog + t.space.panel);
        let available = ctx.content_rect().size();
        let width = (available.x - margin).clamp(1.0, 456.0);
        let height = (available.y - margin - t.ty.title.line - t.space.stack).max(t.row_height());
        widgets::window(self.id.with("properties"), "Properties")
            .open(&mut open)
            .centred()
            .size(egui::vec2(width, height + t.ty.title.line + t.space.stack))
            .show(ctx, |ui| {
                ui.set_width(width);
                egui::ScrollArea::vertical()
                    .id_salt(&props.entry.path)
                    .max_height(height)
                    .show(ui, |ui| {
                        widgets::file_properties(ui, &props.entry, props.kind, &self.zone);
                        if props.listing.is_some() {
                            ui.add(widgets::loading_line("Loading file details…"));
                        }
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
