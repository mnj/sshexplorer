//! sshexplorer's widgets: the recipes of the chrome's controls (ADR-0001, ported from
//! REDA's `widgets`).
//!
//! Each widget is a thin builder on stable egui that picks its colours, paddings, radii
//! and text styles from `ui.tokens()` and nothing else, so it follows light and dark,
//! Density and the fonts without any code of its own. Use one with `ui.add`:
//!
//! ```ignore
//! if ui.add(widgets::button("Save").primary()).clicked() { .. }
//! ui.add(widgets::text_field(&mut name).hint("Display name"));
//! ui.add(widgets::checkbox(&mut remember, "Remember"));
//! ```
//!
//! The data widgets work the same way: `widgets::list_row`, `tree_row`, `table`,
//! `section_header` and `section`, `form` with `form_row`, `status_mark`, `swatch`,
//! `badge` and `count_pill`. So do the shell's: `tab`, `status_item`, `paint_card` with
//! `mask_card_corners`, and the `splitter`.
//!
//! Every control draws rest, hover, press, keyboard focus and disabled, and selected and
//! invalid where they apply. Disabled comes from the `Ui` (`ui.add_enabled(false, ..)`).
//! [`State`] forces one state, for a test or gallery that shows them side by side.
//!
//! `text_view` shows a long text to read and copy, such as the third-party notices.
//!
//! Dimensions that no setting changes (the 28 × 16 switch track, the 16 px mark) are
//! named constants beside their recipe, not `Tokens` entries.

mod badge;
mod bar;
mod button;
mod check;
mod code;
mod dialog;
mod dropdown;
mod field;
mod file_properties;
mod form;
pub mod glyph;
mod key_list;
mod kind;
mod mark;
mod menu;
mod pages;
mod paint;
mod range;
mod row;
mod section;
mod segmented;
mod splitter;
mod status;
mod stepper;
mod surface;
mod tab;
mod table;
mod text_view;
mod tooltip;
mod zoom;

pub use badge::{BADGE_PAD_X, BADGE_PAD_Y, Badge, BadgeTone, badge, count_pill};
pub use bar::{
    BAR_GAP, BAR_SEPARATOR, bar_button_side, bar_frame, bar_frame_inset, bar_row, bar_separator,
    bar_separator_sized, toolbar,
};
pub use button::{
    Button, ICON_GAP, IconButton, Link, button, glyph_button, icon_button, icon_button_image, link,
};
pub use check::{
    Checkbox, MARK, Radio, SWITCH_KNOB, SWITCH_TRACK, Switch, checkbox, radio, switch,
};
pub use code::{
    CodeLine, CodeView, DetailValue, LoadingLine, Muted, Spinner, code_view, detail_value,
    loading_line, muted, spinner,
};
pub use dialog::{Shown, Window, button_row, dialog_frame, scrim, title_row, window, window_frame};
pub use dropdown::{
    Choice, ChoiceRow, Dropdown, FilterDropdown, Picked, choice, choice_value, dropdown,
    filter_dropdown,
};
pub use field::{
    InvalidMessage, TextField, WarningMessage, invalid_message, number_field, retry_save,
    text_field, warning_message,
};
pub(crate) use file_properties::file_properties;
pub use form::{Form, FormName, FormText, form, form_group, form_hint, form_name, form_row};
pub use glyph::{Glyph, GlyphMark, glyph};
pub use key_list::{KeyChipButton, key_chip_button, key_list};
pub use kind::{EntryName, entry_name, kind_color};
pub use mark::{SWATCH, Status, StatusMark, Swatch, status_mark, swatch, swatch_none, swatch_rgb};
pub use menu::{
    Menu, MenuButton, MenuItem, context_menu, menu, menu_bar, menu_below, menu_button, menu_frame,
    menu_is_open, menu_item, menu_separator, menu_titles_width, row_context_menu,
};
pub use pages::{PAGE_HEIGHT, PAGE_LIST_WIDTH, PAGE_WIDTH, PagesDialog, PagesShown, pages_dialog};
pub use paint::{State, drew_focus};
pub use range::{PROGRESS_RAIL, Progress, SLIDER_KNOB, SLIDER_RAIL, Slider, progress, slider};
pub use row::{
    Highlight, Row, RowResponse, TREE_INDENT, TreeLead, list_row, rows, tree_lead, tree_row,
};
pub use section::{Section, SectionHeader, SectionResponse, section, section_header};
pub use segmented::{Segmented, segmented};
pub use splitter::splitter;
pub use status::{
    STATUS_ITEM_MIN, STATUS_ITEM_PAD, StatusItem, status_bar, status_ink, status_item,
};
pub use stepper::{STEPPER_WIDTH, StepperField, stepper_field};
pub use surface::{
    CARD_GAP, card_frame, floating_frame, flyout_frame, flyout_title, mask_card_corners, paint_card,
};
pub use tab::{
    TAB_INSET, Tab, TabKind, TabResponse, accent_line, dock_strip, join_tab, tab, tab_strip,
};
pub use table::{COLUMN_GAP, Column, RowLook, Sorted, Table, TableBody, TableRow, table};
pub use text_view::{TextView, text_view};
pub use tooltip::{
    KeyChip, TOOLTIP_WIDTH, Tooltip, TooltipText, key_chip, tooltip_contents, tooltip_frame,
};
pub use zoom::{ZoomShown, ZoomView, paint_page, paint_texture, zoom_bar, zoom_menu, zoom_view};
