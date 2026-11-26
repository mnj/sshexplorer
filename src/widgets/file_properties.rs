//! Read-only inspection of a remote entry. The permission grid decodes the mode
//! supplied by the host, including execute and special bits in the Unix notation.
use super::kind::{kind_color, paint_kind, paint_link_arrow};
use super::{Glyph, Tooltip as _};
use crate::folder::{modified_text, permissions_text, size_text};
use crate::kind::Kind;
use crate::preview::{Focused, kind_name};
use crate::ssh::Entry;
use crate::theme::{Text, TokensExt as _};
use egui::{Align, Color32, Frame, Layout, Response, Ui, vec2};

/// A selectable, wrapping value with a stable field name for assistive technology.
fn value(ui: &mut Ui, name: &str, text: &str, style: &Text, ink: Color32) -> Response {
    let r = ui.add(
        egui::Label::new(style.job(text, ink))
            .selectable(true)
            .wrap(),
    );
    r.ctx.accesskit_node_builder(r.id, |node| {
        node.set_label(name);
        node.set_value(text);
    });
    r
}

pub(crate) fn file_properties(
    ui: &mut Ui,
    focused: &Focused,
    kind: Kind,
    zone: &jiff::tz::TimeZone,
) {
    let t = ui.tokens();
    let entry = focused.entry.as_ref();
    let linked = entry.is_some_and(|e| e.link.is_some());
    let ink = kind_color(&t, kind);
    ui.add_space(t.space.stack);
    ui.horizontal(|ui| {
        let side = t.icon.bar * 2.5;
        let (tile, _) = ui.allocate_exact_size(vec2(side, side), egui::Sense::hover());
        ui.painter()
            .rect_filled(tile, t.radius.popup, ink.gamma_multiply(0.12));
        paint_kind(ui, &t, tile.center(), t.icon.bar * 1.55, kind, ink);
        if linked {
            paint_link_arrow(ui, &t, tile.center(), t.icon.bar * 1.55);
        }
        ui.add_space(t.space.gap);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = t.space.control_y;
            let mut heading = t.ty.heading.clone();
            heading.size *= 1.25;
            heading.line *= 1.25;
            value(ui, "Name", &focused.name, &heading, t.color.text.primary);
            ui.horizontal_wrapped(|ui| {
                value(ui, "Kind", kind_name(kind), &t.ty.body, t.color.text.muted);
                if let Some(size) = entry.and_then(|e| e.size.filter(|_| !e.is_dir)) {
                    ui.label(t.ty.small.rich("·").color(t.color.text.muted));
                    value(ui, "Size", &size_text(size), &t.ty.body, t.color.text.muted)
                        .tooltip(format!("{size} bytes"));
                }
            });
        });
    });

    ui.add_space(t.space.section);
    ui.horizontal(|ui| {
        ui.add(super::form_group("Remote path"));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let id = ui.scope_id().with(("copied-path", &focused.path));
            let now = ui.input(|i| i.time);
            let copied = ui
                .data(|d| d.get_temp::<f64>(id))
                .is_some_and(|until| now < until);
            let (glyph, tip) = if copied {
                (Glyph::Tick, "Path copied")
            } else {
                (Glyph::Copy, "Copy full remote path")
            };
            if ui
                .add(
                    super::glyph_button(glyph)
                        .side(t.control_height())
                        .name("Copy path"),
                )
                .tooltip(tip)
                .clicked()
            {
                ui.ctx().copy_text(focused.path.clone());
                ui.data_mut(|d| d.insert_temp(id, now + 1.5));
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(1500));
            }
        });
    });
    Frame::new()
        .fill(t.color.surface.well)
        .corner_radius(t.radius.card)
        .inner_margin(t.space.panel)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            value(ui, "Path", &focused.path, &t.ty.mono, t.color.text.primary);
        });
    let Some(e) = entry else { return };
    if let Some(target) = &e.link {
        ui.add_space(t.space.gap);
        ui.add(super::form_hint("Link to"));
        value(ui, "Link to", target, &t.ty.mono, t.color.text.primary);
    }
    if let Some(at) = e.modified {
        ui.add_space(t.space.gap);
        ui.horizontal_wrapped(|ui| {
            ui.label(t.ty.body.rich("Modified").color(t.color.text.muted));
            value(
                ui,
                "Modified",
                &modified_text(at, zone),
                &t.ty.body,
                t.color.text.primary,
            );
        });
    }
    if e.permissions.is_some()
        || e.owner.is_some()
        || e.uid.is_some()
        || e.group.is_some()
        || e.gid.is_some()
    {
        ui.add_space(t.space.section);
        access(ui, e);
    }
}

fn access(ui: &mut Ui, e: &Entry) {
    let t = ui.tokens();
    ui.horizontal_wrapped(|ui| {
        ui.add(super::form_group("Access"));
        if let Some(mode) = e.permissions {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add(super::badge(format!("{:04o}", mode & 0o7777)));
                value(
                    ui,
                    "Permissions",
                    &permissions_text(mode),
                    &t.ty.mono,
                    t.color.text.muted,
                );
            });
        }
    });
    let owner = e.owner.clone().or_else(|| e.uid.map(|id| id.to_string()));
    let group = e.group.clone().or_else(|| e.gid.map(|id| id.to_string()));
    let frame = super::card_frame(&t).shadow(Default::default());
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        let width = ui.available_width();
        let bit_width = (width / 6.0).min(t.ty.body.line * 2.8);
        let identity = (width - 3.0 * bit_width).max(1.0);
        let row = |ui: &mut Ui, class: &str, name: Option<&str>, shift: u32| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.allocate_ui_with_layout(
                    vec2(identity, t.row_height()),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        ui.set_width(identity);
                        ui.spacing_mut().item_spacing.x = t.space.gap;
                        ui.label(t.ty.body.rich(class).color(t.color.text.muted));
                        if let Some(name) = name {
                            value(ui, class, name, &t.ty.body_strong, t.color.text.primary);
                        }
                    },
                );
                if let Some(mode) = e.permissions {
                    for (bit, permission) in [(4, "Read"), (2, "Write"), (1, "Execute")] {
                        let allowed = (mode >> shift) & bit != 0;
                        let (rect, r) = ui.allocate_exact_size(
                            vec2(bit_width, t.row_height()),
                            egui::Sense::hover(),
                        );
                        let state = if allowed { "yes" } else { "no" };
                        let label = format!("{class} · {permission}: {state}");
                        r.widget_info(|| {
                            egui::WidgetInfo::labeled(egui::Role::Label, true, &label)
                        });
                        r.tooltip(&label);
                        let (glyph, color) = if allowed {
                            (Glyph::Tick, t.color.accent.text)
                        } else {
                            (Glyph::Minus, t.color.stroke.strong)
                        };
                        glyph.paint(ui.painter(), rect.center(), color);
                    }
                }
            });
        };
        if e.permissions.is_some() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.allocate_ui_with_layout(
                    vec2(identity, t.ty.small.line),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        ui.set_width(identity);
                        ui.add(super::form_hint("Identity"));
                    },
                );
                for title in ["Read", "Write", "Exec"] {
                    ui.allocate_ui_with_layout(
                        vec2(bit_width, t.ty.small.line),
                        Layout::top_down(Align::Center),
                        |ui| {
                            ui.set_width(bit_width);
                            ui.add(super::form_hint(title));
                        },
                    );
                }
            });
        }
        if owner.is_some() || e.permissions.is_some() {
            row(ui, "Owner", owner.as_deref(), 6);
        }
        if group.is_some() || e.permissions.is_some() {
            row(ui, "Group", group.as_deref(), 3);
        }
        if e.permissions.is_some() {
            row(ui, "Others", None, 0);
        }
    });
}
