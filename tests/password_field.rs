//! A password text field never paints what was typed: it shows one dot per character.

use egui::epaint::Shape;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use sshexplorer::{app, widgets};

/// Every piece of text painted in the last frame.
fn painted(harness: &Harness<'_, String>) -> Vec<String> {
    fn walk(shape: &Shape, out: &mut Vec<String>) {
        match shape {
            Shape::Text(text) => out.push(text.galley.text().to_owned()),
            Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in &harness.output().shapes {
        walk(&clipped.shape, &mut out);
    }
    out
}

#[test]
fn a_password_field_paints_dots_not_the_secret() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(300.0, 80.0))
        .with_context_setup(app::setup)
        .build_ui_state(
            |ui, text: &mut String| {
                ui.add(widgets::text_field(text).password())
                    .accessible_name("Password");
            },
            String::new(),
        );
    harness.get_by_label("Password").focus();
    harness.run();
    harness.get_by_label("Password").type_text("hunter2");
    harness.run();
    assert_eq!(harness.state(), "hunter2");
    let painted = painted(&harness);
    assert!(
        painted.iter().all(|t| !t.contains("hunter2")),
        "the secret was painted: {painted:?}"
    );
    assert!(
        painted.iter().any(|t| t == "•••••••"),
        "no dots were painted: {painted:?}"
    );
}
