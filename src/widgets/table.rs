//! The table: a header of small muted text over one hairline, and rows of cells in
//! columns (REDA docs/spec/v0.1/design-language.md#lists-trees-and-tables).
//!
//! There are no rules between rows or columns and no stripes. A number column is
//! right-aligned in the numeric style, whose digits are tabular, so figures line up.
//!
//! ```ignore
//! widgets::table("net-lengths")
//!     .columns([Column::fill(), Column::auto().number()])
//!     .show(ui, |table| {
//!         table.header(|h| {
//!             h.text("Net");
//!             h.text("Length");
//!         });
//!         for net in nets {
//!             let look = RowLook::selected(ui_selection);
//!             if table.row_with(look, |row| {
//!                 row.text(&net.name);
//!                 row.text(&net.length);
//!             }).clicked() { .. }
//!         }
//!     });
//! ```

use std::sync::Arc;

use egui::{
    Align, Align2, Color32, Id, InnerResponse, Label, Layout, Rect, Response, Role, Sense, Shape,
    Stroke, Ui, UiBuilder, WidgetInfo, pos2, vec2,
};

use super::Tooltip as _;
use super::glyph::{self, Glyph};
use super::mark::{Status, status_mark};
use super::paint::{self, State};
use super::row::{Highlight, ground};
use crate::theme::{Text, Tokens, TokensExt as _};

/// Between two columns of a table.
pub const COLUMN_GAP: f32 = 12.0;
const MIN_COLUMN_WIDTH: f32 = 36.0;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Width {
    Auto,
    Fixed(f32),
    Fill,
}

/// One column of a table: how wide it is, and whether it holds numbers.
#[derive(Clone, Copy, Debug)]
pub struct Column {
    width: Width,
    number: bool,
}

impl Column {
    /// As wide as its widest cell.
    pub fn auto() -> Column {
        Column {
            width: Width::Auto,
            number: false,
        }
    }

    /// `width` points wide. Text that does not fit is cut with an ellipsis.
    pub fn fixed(width: f32) -> Column {
        Column {
            width: Width::Fixed(width),
            number: false,
        }
    }

    /// The width the other columns leave. Text that does not fit is cut with an ellipsis.
    pub fn fill() -> Column {
        Column {
            width: Width::Fill,
            number: false,
        }
    }

    /// A number column: right-aligned, its text in the numeric style.
    pub fn number(mut self) -> Column {
        self.number = true;
        self
    }
}

/// How a column is sorted, for the header cell that sorts by it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Sorted {
    pub ascending: bool,
    /// This column's place among the sort keys, from 1, shown when the table is sorted by
    /// more than one column.
    pub rank: Option<usize>,
}

/// How one row of a table is drawn.
#[derive(Clone, Copy, Default, Debug)]
pub struct RowLook {
    pub selection: Highlight,
    /// Something dragged is over this row and would drop on it.
    pub drop_target: bool,
    /// Forces one state, for the gallery.
    pub state: State,
}

impl RowLook {
    /// A row that shows `selection`.
    pub fn selected(selection: Highlight) -> RowLook {
        RowLook {
            selection,
            ..RowLook::default()
        }
    }

    /// A row of a list that can hold the keyboard: highlighted when `selected`, live
    /// when the list holds the keyboard and the window is in front (`live`), and with
    /// the focus ring when it is the list's focused row while the list holds the
    /// keyboard (`ringed`).
    pub fn of(selected: bool, live: bool, ringed: bool) -> RowLook {
        let highlight = match (selected, live) {
            (false, _) => Highlight::None,
            (true, true) => Highlight::Live,
            (true, false) => Highlight::Unfocused,
        };
        let look = RowLook::selected(highlight);
        if ringed {
            look.forced(State::Focus)
        } else {
            look
        }
    }

    pub fn drop_target(mut self, drop_target: bool) -> RowLook {
        self.drop_target = drop_target;
        self
    }

    pub fn forced(mut self, state: State) -> RowLook {
        self.state = state;
        self
    }
}

/// A table. Give it its columns, then add the header and the rows in `show`.
pub fn table(id_salt: impl egui::AsIdSalt) -> Table {
    Table {
        id_salt: egui::IdSalt::new(id_salt),
        columns: Vec::new(),
        width: None,
        resizable: false,
    }
}

#[must_use = "call show"]
pub struct Table {
    id_salt: egui::IdSalt,
    columns: Vec<Column>,
    width: Option<f32>,
    resizable: bool,
}

impl Table {
    pub fn columns(mut self, columns: impl IntoIterator<Item = Column>) -> Self {
        self.columns = columns.into_iter().collect();
        self
    }

    /// Give the header draggable column boundaries. Double-click to restore automatic sizing.
    pub fn resizable(mut self) -> Self {
        self.resizable = true;
        self
    }

    /// The width a [`Column::fill`] column fills, instead of all that is available.
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    pub fn show<R>(
        self,
        ui: &mut Ui,
        add: impl FnOnce(&mut TableBody<'_>) -> R,
    ) -> InnerResponse<R> {
        let t = ui.tokens();
        let id = ui.make_persistent_id(self.id_salt);
        // Auto columns take the widths their cells had last frame. The first frame has
        // none, so it only measures and is drawn again at once.
        let known: Option<Arc<Vec<f32>>> = ui.data(|d| d.get_temp(id));
        let first = known.as_ref().is_none_or(|k| k.len() != self.columns.len());
        let has_auto = self.columns.iter().any(|c| c.width == Width::Auto);
        let manual: Option<Vec<f32>> = ui.data(|d| d.get_temp(id.with("widths")));
        let manual = manual.filter(|w| self.resizable && w.len() == self.columns.len());
        let measuring = first && has_auto && manual.is_none();
        let available = self.width.unwrap_or_else(|| ui.available_width());
        let mut columns = self.columns;
        if let Some(widths) = &manual {
            for (column, &width) in columns.iter_mut().zip(widths) {
                column.width = Width::Fixed(width);
            }
        }
        let mut widths = resolve(&t, &columns, known.as_deref(), available, measuring);
        if self.resizable && !measuring {
            for width in &mut widths {
                *width = width.max(MIN_COLUMN_WIDTH);
            }
        }
        let gaps = COLUMN_GAP * columns.len().saturating_sub(1) as f32;
        if manual.is_some() {
            let spare = available - gaps - 2.0 * t.space.row_x - widths.iter().sum::<f32>();
            if let Some(last) = widths.last_mut() {
                *last += spare.max(0.0);
            }
        }
        // A column still being measured has no width yet.
        let columns_width: f32 = widths.iter().filter(|w| w.is_finite()).sum();
        let width = columns_width + gaps + 2.0 * t.space.row_x;

        let mut builder = UiBuilder::new().layout(Layout::top_down(Align::Min));
        if measuring {
            builder = builder.sizing_pass().invisible();
            ui.ctx().request_discard("a new table measures its columns");
        }
        let out = ui.scope_builder(builder, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            ui.set_width(width);
            let mut body = TableBody {
                ui,
                t: t.clone(),
                id,
                columns: &columns,
                resizable: self.resizable && !measuring,
                widths: &widths,
                measured: vec![0.0; columns.len()],
                width,
                rows: 0,
            };
            let inner = add(&mut body);
            (inner, body.measured)
        });
        let (inner, measured) = out.inner;
        if manual.is_none() && known.as_deref() != Some(&measured) {
            ui.data_mut(|d| d.insert_temp(id, Arc::new(measured)));
            if !measuring {
                ui.ctx().request_repaint();
            }
        }
        InnerResponse::new(inner, out.response)
    }
}

/// The width of each column this frame.
fn resolve(
    t: &Tokens,
    columns: &[Column],
    known: Option<&Vec<f32>>,
    available: f32,
    measuring: bool,
) -> Vec<f32> {
    let mut widths: Vec<f32> = columns
        .iter()
        .enumerate()
        .map(|(i, column)| match column.width {
            Width::Fixed(width) => width,
            Width::Auto if measuring => f32::INFINITY,
            Width::Auto => known.and_then(|k| k.get(i)).copied().unwrap_or(0.0),
            Width::Fill => 0.0,
        })
        .collect();
    let fills = columns.iter().filter(|c| c.width == Width::Fill).count();
    if fills > 0 && !measuring {
        let gaps = COLUMN_GAP * columns.len().saturating_sub(1) as f32;
        let taken = widths.iter().sum::<f32>() + gaps + 2.0 * t.space.row_x;
        let each = ((available - taken) / fills as f32).max(0.0).floor();
        for (width, column) in widths.iter_mut().zip(columns) {
            if column.width == Width::Fill {
                *width = each;
            }
        }
    }
    widths
}

/// The table being shown: add its header, then its rows.
pub struct TableBody<'a> {
    resizable: bool,
    ui: &'a mut Ui,
    t: Arc<Tokens>,
    id: Id,
    columns: &'a [Column],
    widths: &'a [f32],
    measured: Vec<f32>,
    width: f32,
    rows: usize,
}

impl TableBody<'_> {
    /// The header: one cell per column in small muted text, over a hairline.
    pub fn header<R>(&mut self, add: impl FnOnce(&mut TableRow<'_>) -> R) -> R {
        let height = self.t.ty.small.line + 2.0 * self.t.space.row_y;
        let (rect, inner) = self.cells(true, height, add);
        let t = &self.t;
        let line = Stroke::new(t.stroke.hairline, t.color.stroke.hairline);
        // The hairline is the last pixel row of the header.
        let y = rect.bottom() - t.stroke.hairline / 2.0;
        self.ui.painter().hline(rect.x_range(), y, line);
        if self.resizable {
            self.dividers(rect);
        }
        inner
    }

    fn dividers(&mut self, header: Rect) {
        let mut x = header.left() + self.t.space.row_x;
        for (i, &width) in self
            .widths
            .iter()
            .take(self.widths.len().saturating_sub(1))
            .enumerate()
        {
            x += width + COLUMN_GAP / 2.0;
            let hit = Rect::from_center_size(
                pos2(x, header.center().y),
                vec2(COLUMN_GAP, header.height()),
            );
            let response = self
                .ui
                .interact(hit, self.id.with(("divider", i)), Sense::click_and_drag())
                .on_hover_and_drag_cursor(egui::CursorIcon::ResizeHorizontal);
            response.widget_info(|| {
                WidgetInfo::labeled(
                    Role::Splitter,
                    self.ui.is_enabled(),
                    format!("Resize column {}", i + 1),
                )
            });
            let color = if response.hovered() || response.dragged() {
                self.t.color.accent.text
            } else {
                self.t.color.stroke.hairline
            };
            self.ui.painter().vline(
                x,
                header.shrink(self.t.space.row_y).y_range(),
                Stroke::new(self.t.stroke.hairline, color),
            );
            let key = self.id.with("widths");
            if response.double_clicked() {
                self.ui.data_mut(|d| d.remove::<Vec<f32>>(key));
                self.ui.ctx().request_repaint();
            } else if response.dragged_by(egui::PointerButton::Primary)
                && response.drag_delta().x != 0.0
            {
                let mut widths = self
                    .ui
                    .data(|d| d.get_temp::<Vec<f32>>(key))
                    .unwrap_or_else(|| self.widths.to_vec());
                widths[i] = (widths[i] + response.drag_delta().x).max(MIN_COLUMN_WIDTH);
                self.ui.data_mut(|d| d.insert_temp(key, widths));
                self.ui.ctx().request_repaint();
            }
            response.tooltip("Drag to resize. Double-click to reset column widths.");
            x += COLUMN_GAP / 2.0;
        }
    }

    /// A row that only shows its cells.
    pub fn row<R>(&mut self, add: impl FnOnce(&mut TableRow<'_>) -> R) -> R {
        let height = self.t.row_height();
        self.rows += 1;
        self.cells(false, height, add).1
    }

    /// A row that can be chosen: it draws hover, press, focus, its selection and a drop
    /// target, and its response says when it is clicked. Name it for AccessKit with
    /// `widget_info` when its cells do not say enough.
    pub fn row_with(&mut self, look: RowLook, add: impl FnOnce(&mut TableRow<'_>)) -> Response {
        self.row_with_key(self.rows, look, add)
    }

    /// A stable row identity keeps its context menu attached to the same entry across refreshes.
    pub fn row_with_key(
        &mut self,
        key: impl egui::AsIdSalt,
        look: RowLook,
        add: impl FnOnce(&mut TableRow<'_>),
    ) -> Response {
        let t = self.t.clone();
        self.rows += 1;
        let id = self.id.with(("row", key));
        let width = self.width;
        let builder = if look.state == State::Disabled {
            UiBuilder::new().disabled()
        } else {
            UiBuilder::new()
        };
        let (columns, widths) = (self.columns, self.widths);
        let measured = &mut self.measured;
        self.ui
            .scope_builder(builder, |ui| {
                // The row is sensed before its cells are added, so a control in a cell
                // stays on top of it.
                let at = Rect::from_min_size(ui.cursor().min, vec2(width, t.row_height()));
                let response = ui.interact(at, id, Sense::click());
                let enabled = ui.is_enabled();
                response.widget_info(|| {
                    let mut info = WidgetInfo::new(Role::Row);
                    info.enabled = enabled;
                    info.selected = Some(look.selection.is_selected());
                    info
                });
                ui.ctx().accesskit_node_builder(id, |node| {
                    node.set_selected(look.selection.is_selected());
                });
                let under = ui.painter().add(Shape::Noop);
                let mut sub = TableBody {
                    ui,
                    t: t.clone(),
                    id,
                    columns,
                    resizable: false,
                    widths,
                    measured: std::mem::take(measured),
                    width,
                    rows: 0,
                };
                let (rect, ()) = sub.cells(false, t.row_height(), add);
                *measured = sub.measured;
                let s = paint::seen(&response, look.state);
                ui.painter()
                    .set(under, ground(&t, rect, look.selection, look.drop_target, s));
                response
            })
            .inner
    }

    /// `count` rows of one row height each, of which only those in view are added: `add`
    /// is called with the body and each such row's index, and adds it with
    /// [`TableBody::row_with`] (or [`TableBody::row`]). The rows out of view take their
    /// space, so the scroll bar is right. `reveal` scrolls that row into view, drawn or
    /// not. Auto columns measure the rows in view.
    pub fn rows(
        &mut self,
        count: usize,
        reveal: Option<usize>,
        mut add: impl FnMut(&mut TableBody<'_>, usize),
    ) {
        let height = self.t.row_height();
        let top = self.ui.cursor().min.y;
        let clip = self.ui.clip_rect();
        let at = |y: f32| (((y - top) / height).max(0.0) as usize).min(count);
        let first = at(clip.top()).saturating_sub(1);
        let end = (at(clip.bottom()) + 1).min(count);
        let width = self.width;
        let left = self.ui.cursor().min.x;
        let space = |rows: usize| vec2(width, rows as f32 * height);
        if first > 0 {
            self.ui.allocate_space(space(first));
        }
        let rows = self.rows;
        for i in first..end {
            self.rows = rows + i;
            add(self, i);
        }
        self.rows = rows + count;
        if end < count {
            self.ui.allocate_space(space(count - end));
        }
        if let Some(i) = reveal.filter(|i| *i < first || *i >= end) {
            let row = Rect::from_min_size(pos2(left, top + i as f32 * height), space(1));
            self.ui.scroll_to_rect(row, None);
        }
    }

    /// Lays out one row of cells at least `height` high; returns the row's rectangle.
    fn cells<R>(
        &mut self,
        header: bool,
        height: f32,
        add: impl FnOnce(&mut TableRow<'_>) -> R,
    ) -> (Rect, R) {
        let top_left = self.ui.cursor().min;
        let mut row = TableRow {
            ui: self.ui,
            t: &self.t,
            columns: self.columns,
            widths: self.widths,
            measured: &mut self.measured,
            header,
            column: 0,
            x: top_left.x + self.t.space.row_x,
            top: top_left.y,
            height,
            tallest: height,
        };
        let inner = add(&mut row);
        let tallest = row.tallest;
        let rect = Rect::from_min_size(top_left, vec2(self.width, tallest));
        self.ui.allocate_rect(rect, Sense::hover());
        (rect, inner)
    }
}

/// One row of a table being filled: each call adds the next column's cell.
pub struct TableRow<'a> {
    ui: &'a mut Ui,
    t: &'a Tokens,
    columns: &'a [Column],
    widths: &'a [f32],
    measured: &'a mut Vec<f32>,
    header: bool,
    column: usize,
    x: f32,
    top: f32,
    height: f32,
    tallest: f32,
}

impl TableRow<'_> {
    /// A cell of text: body text in the primary colour, or the numeric style in a number
    /// column. In the header it is small and muted.
    pub fn text(&mut self, text: impl Into<String>) -> Response {
        let ink = if self.header {
            self.t.color.text.muted
        } else {
            self.t.color.text.primary
        };
        self.text_in(text.into(), ink)
    }

    /// A cell of muted text: a secondary column, a waived row.
    pub fn muted(&mut self, text: impl Into<String>) -> Response {
        self.text_in(text.into(), self.t.color.text.muted)
    }

    /// A cell of text in a colour of the caller's: a figure out of tolerance in a tone's
    /// text strength.
    pub fn colored(&mut self, text: impl Into<String>, ink: Color32) -> Response {
        self.text_in(text.into(), ink)
    }

    /// A cell holding a status mark.
    pub fn status(&mut self, status: Status) -> Response {
        self.ui(|ui| ui.add(status_mark(status)))
    }

    /// An empty cell.
    pub fn empty(&mut self) {
        self.ui(|_| ());
    }

    /// A cell holding whatever `add` adds: a control, a link, a swatch.
    pub fn ui<R>(&mut self, add: impl FnOnce(&mut Ui) -> R) -> R {
        let Some(column) = self.columns.get(self.column).copied() else {
            panic!("a table row has more cells than the table has columns");
        };
        let width = self.widths[self.column];
        let rect = Rect::from_min_size(pos2(self.x, self.top), vec2(width, self.height));
        let layout = if column.number {
            Layout::right_to_left(Align::Center)
        } else {
            Layout::left_to_right(Align::Center)
        };
        // An auto column that is still being measured has no width to align in yet.
        let layout = if width.is_finite() {
            layout
        } else {
            Layout::left_to_right(Align::Center)
        };
        let mut cell = self
            .ui
            .new_child(UiBuilder::new().max_rect(rect).layout(layout));
        if matches!(column.width, Width::Fill | Width::Fixed(_)) {
            let mut clip = cell.clip_rect();
            clip.min.x = clip.min.x.max(rect.left());
            clip.max.x = clip.max.x.min(rect.right());
            cell.set_clip_rect(clip);
        }
        let inner = add(&mut cell);
        let used = cell.min_rect();
        let measured = &mut self.measured[self.column];
        *measured = measured.max(used.width().ceil());
        self.tallest = self.tallest.max(used.height());
        if width.is_finite() {
            self.x += width + COLUMN_GAP;
        }
        self.column += 1;
        inner
    }

    /// A cell like [`TableRow::ui`] whose controls are disabled unless `enabled`.
    pub fn ui_enabled<R>(&mut self, enabled: bool, add: impl FnOnce(&mut Ui) -> R) -> R {
        self.ui(|ui| ui.add_enabled_ui(enabled, add).inner)
    }

    /// A header cell that sorts by its column when clicked: the title, and an arrow (with
    /// the column's rank when there are several sort keys) while the table is sorted by
    /// it. It is a button named `title`.
    pub fn sort(&mut self, title: impl Into<String>, sorted: Option<Sorted>) -> Response {
        let title = title.into();
        let t = self.t;
        let style = &t.ty.small;
        let rank = sorted
            .and_then(|s| s.rank)
            .map(|rank| rank.to_string())
            .unwrap_or_default();
        self.ui(|ui| {
            let mut width = paint::width(ui, style, &title);
            if sorted.is_some() {
                width += glyph::BOX;
            }
            if !rank.is_empty() {
                width += paint::width(ui, &t.ty.numeric_small, &rank);
            }
            let size = vec2(width, ui.available_height());
            let (rect, response) = ui.allocate_exact_size(size, Sense::click());
            let enabled = ui.is_enabled();
            response.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, &title));
            if !ui.is_rect_visible(rect) {
                return response;
            }
            let s = paint::seen(&response, State::Live);
            let ink = if s.hover || s.press || sorted.is_some() {
                t.color.text.primary
            } else {
                t.color.text.muted
            };
            // Drawn from the leading edge whichever way the column is aligned: the title,
            // the arrow, the rank.
            let drawn = paint::text_in(ui, rect, Align2::LEFT_CENTER, style, &title, ink);
            if let Some(sorted) = sorted {
                let arrow = if sorted.ascending {
                    Glyph::SortUp
                } else {
                    Glyph::SortDown
                };
                let at = pos2(drawn.right() + glyph::BOX / 2.0, rect.center().y);
                arrow.paint(ui.painter(), at, ink);
                let after = rect.with_min_x(drawn.right() + glyph::BOX);
                let style = &t.ty.numeric_small;
                paint::text_in(ui, after, Align2::LEFT_CENTER, style, &rank, ink);
            }
            if s.focus {
                paint::focus_ring(ui, t, rect, t.radius.mark);
            }
            response
        })
    }

    fn text_in(&mut self, text: String, ink: Color32) -> Response {
        let column = self.columns.get(self.column).copied();
        let number = column.is_some_and(|c| c.number);
        let fits = column.is_some_and(|c| c.width == Width::Auto);
        let style: &Text = match (self.header, number) {
            (true, _) => &self.t.ty.small,
            (false, true) => &self.t.ty.numeric,
            (false, false) => &self.t.ty.body,
        };
        let job = style.job(&text, ink);
        self.ui(|ui| {
            let label = Label::new(job).selectable(false);
            if fits {
                ui.add(label.extend())
            } else {
                ui.add(label.truncate())
            }
        })
    }
}
