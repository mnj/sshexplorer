//! A PDF's Preview: its pages stacked in a scrolling view under the zoom toolbar, with
//! the page counter ("3 / 12") in the toolbar.
//!
//! The document opens on its own [`Renderer`] thread. Each frame asks it for the pages in
//! view that have no texture at the scale in force, so pages are rasterised as they
//! scroll into view and again after the zoom changes; until then a page shows its last
//! texture stretched, or blank paper. Textures of pages far out of view are dropped.
//!
//! Fit fits the widest page to the view's width (it never enlarges). The presets, the
//! zoom keys, Ctrl+wheel (around the pointer) and the right-click menu work as for an
//! image; the wheel scrolls, and dragging pans. One [`PdfView`] belongs to one preview,
//! so each newly focused file starts at Fit and at its first page.

use egui::{Id, Rect, ScrollArea, Sense, TextureHandle, TextureOptions, Vec2, vec2};

use crate::keymap::{Action, KeyMap};
use crate::pdf::{Event, Layout, Renderer, Want};
use crate::preview::Pdf;
use crate::theme::TokensExt as _;
use crate::widgets;
use crate::zoom::{self, Level, Zoom};

/// How many pages either side of those in view keep their textures.
const KEEP: usize = 2;

#[derive(Default)]
pub struct PdfView {
    zoom: Zoom,
    state: State,
    /// The scroll position, in screen points from the stack's top left.
    offset: Vec2,
    /// The view's height last frame, for paging with the keys.
    height: f32,
    /// The height one move key scrolls, in screen points: a row's.
    line: f32,
}

#[derive(Default)]
enum State {
    #[default]
    Idle,
    Opening(Renderer),
    Ready(Ready),
    Done,
}

struct Ready {
    renderer: Renderer,
    layout: Layout,
    /// Each page's texture and the render scale it was drawn at.
    textures: Vec<Option<(TextureHandle, f32)>>,
    /// The list last asked of the renderer.
    asked: Vec<Want>,
    counter: String,
}

impl PdfView {
    /// Draws `pdf`, scrolled `scroll` view heights further, taking the zoom keys from
    /// `keymap` when given. A click on the pages gives the keyboard to `focus`. Returns
    /// the failure line when the PDF cannot be shown.
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        pdf: &Pdf,
        keymap: Option<&KeyMap>,
        scroll: f32,
        focus: Id,
    ) -> Option<String> {
        self.line = ui.tokens().row_height();
        if let State::Idle = self.state {
            let kept = pdf.fetched.kept.clone();
            let max_side = ui.ctx().input(|i| i.max_texture_side) as u32;
            let ctx = ui.ctx().clone();
            self.state = State::Opening(Renderer::load(
                move || kept.bytes().map(|bytes| bytes.into_owned()),
                max_side,
                move || ctx.request_repaint(),
            ));
        }
        if let State::Opening(renderer) = &self.state {
            match renderer.try_next() {
                Some(Event::Opened(sizes)) => {
                    let State::Opening(renderer) = std::mem::take(&mut self.state) else {
                        unreachable!()
                    };
                    let layout = Layout::new(&sizes, ui.tokens().size.page_gap);
                    self.state = State::Ready(Ready {
                        renderer,
                        textures: (0..layout.len()).map(|_| None).collect(),
                        counter: format!("1 / {}", layout.len()),
                        layout,
                        asked: Vec::new(),
                    });
                }
                Some(Event::Failed(line)) => {
                    self.state = State::Done;
                    return Some(line);
                }
                Some(Event::Page(_)) | None => {}
            }
        }
        let State::Ready(ready) = &mut self.state else {
            if let State::Opening(_) = self.state {
                ui.add(widgets::loading_line(crate::preview::LOADING));
            }
            return None;
        };
        while let Some(event) = ready.renderer.try_next() {
            match event {
                Event::Page(page) => {
                    let options = TextureOptions::LINEAR;
                    let name = format!("preview/pdf/{}", page.index);
                    let texture = ui.ctx().load_texture(name, page.image, options);
                    if let Some(slot) = ready.textures.get_mut(page.index) {
                        *slot = Some((texture, page.scale));
                    }
                }
                Event::Failed(line) => {
                    self.state = State::Done;
                    return Some(line);
                }
                Event::Opened(_) => {}
            }
        }

        widgets::zoom_bar(ui, &mut self.zoom, Some(&ready.counter));
        let content = ready.layout.size();
        let view = ui
            .available_size()
            .max(Vec2::splat(ui.tokens().control_height() * 2.0));
        let fit = if content.x > 0.0 {
            (view.x / content.x).min(1.0)
        } else {
            1.0
        };
        let before = scale_of(&self.zoom, fit);

        if let Some(keymap) = keymap {
            ui.input_mut(|i| {
                if keymap.take(i, Action::ZoomIn) {
                    self.zoom.zoom_in(fit);
                }
                if keymap.take(i, Action::ZoomOut) {
                    self.zoom.zoom_out(fit);
                }
                if keymap.take(i, Action::ZoomFit) {
                    self.zoom.fit();
                }
            });
        }
        let top = ui.cursor().min;
        let (factor, pointer) = ui.input(|i| (i.zoom_delta(), i.pointer.hover_pos()));
        let hovered = pointer.filter(|p| Rect::from_min_size(top, view).contains(*p));
        if factor != 1.0
            && let Some(pointer) = hovered
        {
            let scale = (before * factor).clamp(zoom::MIN.min(fit), zoom::MAX);
            self.zoom.level = Level::Scale(scale);
            // Keep the point under the pointer where it is.
            let at = pointer - top;
            self.offset = (self.offset + at) * (scale / before) - at;
        } else {
            let after = scale_of(&self.zoom, fit);
            if after != before {
                // A step or a preset: keep the view's middle where it is.
                let middle = view / 2.0;
                self.offset = (self.offset + middle) * (after / before) - middle;
            }
        }
        let scale = scale_of(&self.zoom, fit);
        self.offset.y += scroll * self.height.max(self.line);
        let scaled = content * scale;
        // Within the stack, so that the keys' ends land on its ends.
        self.offset = self
            .offset
            .min((scaled - view).max(Vec2::ZERO))
            .max(Vec2::ZERO);

        let output = ScrollArea::both()
            .id_salt("pdf")
            .auto_shrink(false)
            .scroll_offset(self.offset)
            .show_viewport(ui, |ui, viewport| {
                let size = scaled.max(viewport.size());
                let (whole, _) = ui.allocate_exact_size(size, Sense::hover());
                let left = (size.x - scaled.x) / 2.0;
                let points = Rect::from_min_max(
                    (viewport.min - vec2(left, 0.0)) / scale,
                    (viewport.max - vec2(left, 0.0)) / scale,
                );
                let visible = ready.layout.visible(points);
                ready.counter = ready.layout.counter(points);
                let clip = ui.clip_rect();
                let ppp = ui.ctx().pixels_per_point();
                let render = scale * ppp;
                let mut wants = Vec::new();
                for index in visible.clone() {
                    let page = ready.layout.page(index);
                    let rect = Rect::from_min_size(
                        whole.min + vec2(left, 0.0) + page.min.to_vec2() * scale,
                        page.size() * scale,
                    );
                    let texture = ready.textures[index].as_ref();
                    if texture.is_none_or(|(_, at)| (at - render).abs() > 1e-3) {
                        wants.push(Want {
                            page: index,
                            scale: render,
                        });
                    }
                    widgets::paint_page(ui, clip, rect, texture.map(|(t, _)| t.id()));
                }
                // Far from the view: let the GPU memory go.
                for (index, slot) in ready.textures.iter_mut().enumerate() {
                    if index + KEEP < visible.start || index >= visible.end + KEEP {
                        *slot = None;
                    }
                }
                if wants != ready.asked {
                    ready.renderer.want(wants.clone());
                    ready.asked = wants;
                }
            });
        self.offset = output.state.clamped_offset();
        self.height = output.inner_rect.height();
        let response = ui.interact(
            output.inner_rect,
            ui.scope_id().with("pdf-view"),
            Sense::click(),
        );
        response
            .ctx
            .accesskit_node_builder(response.id, |node| node.set_label("PDF"));
        if response.clicked() {
            ui.memory_mut(|m| m.request_focus(focus));
        }
        widgets::zoom_menu(&response, &mut self.zoom);
        None
    }

    /// How many move-key steps make one view height, for paging.
    pub fn rows(&self) -> usize {
        (self.height / self.line.max(1.0)).max(1.0) as usize
    }
}

fn scale_of(zoom: &Zoom, fit: f32) -> f32 {
    match zoom.level {
        Level::Fit => fit,
        Level::Scale(s) => s,
    }
}
