//! An image's Preview: decoded on a thread of its own, uploaded as textures within the
//! GPU's limit, played when animated, and shown under the zoom toolbar.
//!
//! One [`ImageView`] belongs to one preview; a new one starts at Fit, which is how each
//! newly focused file starts.

use crate::preview::cannot_preview;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use egui::{TextureHandle, TextureOptions};

use crate::keymap::KeyMap;
use crate::picture::{self, Picture};
use crate::preview::Image;
use crate::widgets;
use crate::zoom::Zoom;

/// The shortest a frame shows; shorter delays (often 0) play at [`SLOW_FRAME`], as
/// browsers do.
const FAST_FRAME: Duration = Duration::from_millis(20);
const SLOW_FRAME: Duration = Duration::from_millis(100);

#[derive(Default)]
pub struct ImageView {
    zoom: Zoom,
    state: State,
}

#[derive(Default)]
enum State {
    #[default]
    Idle,
    Decoding(Receiver<Result<Picture, String>>),
    Ready(Ready),
    Done,
}

struct Ready {
    size: [u32; 2],
    dimensions: String,
    frames: Vec<(TextureHandle, f64)>,
    /// When the animation started, and one loop's length, in seconds.
    start: f64,
    cycle: f64,
}

impl ImageView {
    /// Draws `image`, the file `name`, taking the zoom keys from `keymap` when given.
    /// Returns the failure line when it cannot be decoded.
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        name: &str,
        image: &Image,
        keymap: Option<&KeyMap>,
    ) -> Option<String> {
        if let State::Idle = self.state {
            self.state = match image.fetched.kept.bytes() {
                Ok(bytes) => State::Decoding(decode(ui.ctx(), name, bytes.into_owned())),
                Err(e) => return Some(format!("Cannot keep the preview: {e}")),
            };
        }
        if let State::Decoding(rx) = &self.state {
            match rx.try_recv() {
                Ok(Ok(picture)) => self.state = State::Ready(upload(ui, name, picture)),
                Ok(Err(reason)) => {
                    self.state = State::Done;
                    return Some(cannot_preview(&reason));
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.state = State::Done;
                    return Some(cannot_preview("the decoder stopped"));
                }
            }
        }
        let State::Ready(ready) = &self.state else {
            if let State::Decoding(_) = self.state {
                ui.add(widgets::loading_line(crate::preview::LOADING));
            }
            return None;
        };
        widgets::zoom_bar(ui, &mut self.zoom, Some(&ready.dimensions));
        let content = egui::vec2(ready.size[0] as f32, ready.size[1] as f32);
        let shown = widgets::zoom_view(
            ui.scope_id().with("image"),
            "Image",
            &mut self.zoom,
            content,
        )
        .keys(keymap)
        .show(ui);
        let texture = ready.frame(ui);
        widgets::paint_texture(ui, &shown, texture.id());
        None
    }
}

impl Ready {
    /// The frame to show now; asks for a repaint when the next one is due.
    fn frame(&self, ui: &egui::Ui) -> &TextureHandle {
        if self.frames.len() < 2 || self.cycle <= 0.0 {
            return &self.frames[0].0;
        }
        let now = ui.input(|i| i.time);
        let mut at = (now - self.start).rem_euclid(self.cycle);
        for (texture, delay) in &self.frames {
            if at < *delay {
                ui.ctx()
                    .request_repaint_after(Duration::from_secs_f64(delay - at));
                return texture;
            }
            at -= delay;
        }
        ui.ctx().request_repaint();
        &self.frames[0].0
    }
}

fn decode(ctx: &egui::Context, name: &str, bytes: Vec<u8>) -> Receiver<Result<Picture, String>> {
    let max_side = ctx.input(|i| i.max_texture_side) as u32;
    let (tx, rx) = mpsc::channel();
    let ctx = ctx.clone();
    let name = name.to_owned();
    std::thread::Builder::new()
        .name("preview-image".into())
        .spawn(move || {
            let _ = tx.send(picture::decode(&name, &bytes, max_side));
            ctx.request_repaint();
        })
        .expect("spawn the image decoder");
    rx
}

fn upload(ui: &egui::Ui, name: &str, picture: Picture) -> Ready {
    let options = TextureOptions::LINEAR.with_mipmap_mode(Some(egui::TextureFilter::Linear));
    let dimensions = picture.dimensions();
    let size = picture.size;
    let frames: Vec<(TextureHandle, f64)> = picture
        .frames
        .into_iter()
        .enumerate()
        .map(|(i, frame)| {
            let delay = if frame.delay < FAST_FRAME {
                SLOW_FRAME
            } else {
                frame.delay
            };
            let texture =
                ui.ctx()
                    .load_texture(format!("preview/{name}/{i}"), frame.image, options);
            (texture, delay.as_secs_f64())
        })
        .collect();
    let cycle = frames.iter().map(|(_, d)| d).sum();
    Ready {
        size,
        dimensions,
        frames,
        start: ui.input(|i| i.time),
        cycle,
    }
}
