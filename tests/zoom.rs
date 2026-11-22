//! The zoom toolbar's model, apart from egui's drawing: Fit and the presets, stepping,
//! Ctrl+wheel, and where the content sits in the view as it is dragged.

use egui::{Rect, pos2, vec2};
use sshexplorer::zoom::{Level, Zoom};

const VIEW: Rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(400.0, 300.0));

#[test]
fn fit_shrinks_large_content_into_the_view_and_never_enlarges_small_content() {
    let zoom = Zoom::default();
    assert_eq!(zoom.level, Level::Fit);
    assert_eq!(zoom.scale(vec2(800.0, 300.0), VIEW.size()), 0.5);
    assert_eq!(zoom.scale(vec2(400.0, 600.0), VIEW.size()), 0.5);
    assert_eq!(zoom.scale(vec2(40.0, 30.0), VIEW.size()), 1.0);
    assert_eq!(zoom.label(), "Fit");
}

#[test]
fn the_keys_step_through_fit_and_the_presets_in_order() {
    let mut zoom = Zoom::default();
    // Fit is 25 % here, below every preset.
    let fit = 0.25;
    zoom.zoom_in(fit);
    assert_eq!(zoom.level, Level::Scale(0.5));
    assert_eq!(zoom.label(), "50%");
    zoom.zoom_in(fit);
    zoom.zoom_in(fit);
    zoom.zoom_in(fit);
    assert_eq!(zoom.level, Level::Scale(4.0));
    // 400 % is as far as the keys go.
    zoom.zoom_in(fit);
    assert_eq!(zoom.level, Level::Scale(4.0));
    zoom.zoom_out(fit);
    assert_eq!(zoom.level, Level::Scale(2.0));
    zoom.zoom_out(fit);
    zoom.zoom_out(fit);
    zoom.zoom_out(fit);
    assert_eq!(zoom.level, Level::Fit);
    zoom.zoom_out(fit);
    assert_eq!(zoom.level, Level::Fit);

    // Fit at 100 % (small content): the step in from Fit skips the 100 % it already is.
    let mut zoom = Zoom::default();
    zoom.zoom_in(1.0);
    assert_eq!(zoom.level, Level::Scale(2.0));
    zoom.zoom_out(1.0);
    assert_eq!(zoom.level, Level::Scale(1.0));
    zoom.fit();
    assert_eq!(zoom.level, Level::Fit);
}

#[test]
fn ctrl_wheel_zooms_by_a_factor_within_bounds() {
    let mut zoom = Zoom::default();
    zoom.wheel(1.5, 0.5, VIEW, VIEW.center(), vec2(800.0, 600.0));
    assert_eq!(zoom.level, Level::Scale(0.75));
    assert_eq!(zoom.label(), "75%");
    for _ in 0..40 {
        zoom.wheel(2.0, 0.5, VIEW, VIEW.center(), vec2(800.0, 600.0));
    }
    assert_eq!(zoom.level, Level::Scale(sshexplorer::zoom::MAX));
    for _ in 0..80 {
        zoom.wheel(0.5, 0.5, VIEW, VIEW.center(), vec2(800.0, 600.0));
    }
    assert_eq!(zoom.level, Level::Scale(sshexplorer::zoom::MIN));
}

#[test]
fn content_smaller_than_the_view_is_centred_and_does_not_pan() {
    let mut zoom = Zoom::default();
    let content = vec2(100.0, 50.0);
    assert_eq!(
        zoom.layout(content, VIEW),
        Rect::from_min_size(pos2(150.0, 125.0), content)
    );
    assert!(!zoom.pans(content, VIEW.size()));
    zoom.drag(vec2(30.0, 30.0), content, VIEW);
    assert_eq!(zoom.layout(content, VIEW).min, pos2(150.0, 125.0));
}

#[test]
fn dragging_pans_larger_content_and_stops_at_its_edges() {
    let mut zoom = Zoom::default();
    zoom.set(Level::Scale(1.0));
    let content = vec2(800.0, 600.0);
    assert!(zoom.pans(content, VIEW.size()));
    // Centred to begin with.
    assert_eq!(zoom.layout(content, VIEW).min, pos2(-200.0, -150.0));
    // Dragging right and down shows more of the left and top.
    zoom.drag(vec2(50.0, 20.0), content, VIEW);
    assert_eq!(zoom.layout(content, VIEW).min, pos2(-150.0, -130.0));
    // Not past the edge.
    zoom.drag(vec2(1000.0, 1000.0), content, VIEW);
    assert_eq!(zoom.layout(content, VIEW).min, pos2(0.0, 0.0));
    zoom.drag(vec2(-5000.0, -5000.0), content, VIEW);
    assert_eq!(zoom.layout(content, VIEW).max, pos2(400.0, 300.0));
}

#[test]
fn ctrl_wheel_keeps_the_point_under_the_pointer_still() {
    let mut zoom = Zoom::default();
    zoom.set(Level::Scale(1.0));
    let content = vec2(800.0, 600.0);
    let pointer = pos2(100.0, 100.0);
    let before = zoom.layout(content, VIEW);
    let under = (pointer - before.min) / 1.0;
    zoom.wheel(2.0, 0.5, VIEW, pointer, content);
    let after = zoom.layout(content, VIEW);
    assert_eq!(after.min + under * 2.0, pointer);
}

#[test]
fn a_preset_or_fit_starts_again_from_the_centre() {
    let mut zoom = Zoom::default();
    zoom.set(Level::Scale(2.0));
    let content = vec2(800.0, 600.0);
    zoom.drag(vec2(100.0, 0.0), content, VIEW);
    zoom.set(Level::Scale(1.0));
    assert_eq!(zoom.layout(content, VIEW).min, pos2(-200.0, -150.0));
}

#[test]
fn the_zoom_keys_are_in_the_key_map_under_preview() {
    use egui::{Key, Modifiers};
    use sshexplorer::keymap::{Action, Area, Binding, KeyMap};
    let map = KeyMap::default();
    for (action, name, key) in [
        (Action::ZoomIn, "Zoom in (or Ctrl+wheel)", Key::Equals),
        (Action::ZoomOut, "Zoom out (or Ctrl+wheel)", Key::Minus),
        (Action::ZoomFit, "Zoom to Fit", Key::Num0),
    ] {
        assert!(Action::ALL.contains(&action));
        assert_eq!(action.area(), Area::Preview);
        assert_eq!(action.name(), name);
        assert_eq!(
            map.binding(action),
            Some(Binding::new(Modifiers::COMMAND, key))
        );
    }
}
