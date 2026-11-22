//! Audio and video in Preview: a [`Player`] fed by ranged reads over the Session's
//! connection, its toolbar, and the video frame under the zoom toolbar.
//!
//! One [`MediaView`] belongs to one preview; dropping it stops playback and the reads.
//! It opens paused on the first frame (or the audio placeholder) unless Autoplay is on.
//! The toolbar has Play/Pause, a seek bar, elapsed/total time, Mute and a volume slider.
//! Playback pauses when the keyboard moves from Preview to anything else; a click that
//! leaves nothing focused is not leaving.

use std::time::Duration;

use egui::{TextureHandle, TextureOptions};

use crate::keymap::KeyMap;
use crate::media::{AudioOut, Options, Phase, Player, RangedReader};
use crate::preview::Stream;
use crate::ssh;
use crate::theme::TokensExt as _;
use crate::widgets;
use crate::zoom::Zoom;

/// What shows in place of a picture for audio.
pub const AUDIO_PLACEHOLDER: &str = "Audio only: there is no picture.";
/// How often the clock is redrawn while playing.
const TICK: Duration = Duration::from_millis(200);

#[derive(Default)]
pub struct MediaView {
    player: Option<Player>,
    zoom: Zoom,
    frame: Option<TextureHandle>,
    /// Whether Autoplay has had its turn.
    started: bool,
    /// Whether the keyboard was in Preview last frame.
    inside: bool,
}

/// What the view needs from the panel to draw a frame.
pub struct MediaFrame<'a> {
    pub connection: &'a ssh::Session,
    /// The zoom keys, when the panel may take keys.
    pub keymap: Option<&'a KeyMap>,
    pub autoplay: bool,
    pub audio: AudioOut,
    /// Whether the keyboard is in Preview; `None` when nothing has it, which is not
    /// leaving.
    pub inside: Option<bool>,
    /// Whether Play/Pause was pressed this frame.
    pub toggle: bool,
}

impl MediaView {
    /// Draws `stream`. Returns the failure line when it cannot be played.
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        stream: &Stream,
        frame: MediaFrame<'_>,
    ) -> Option<String> {
        let ctx = ui.ctx().clone();
        let player = self.player.get_or_insert_with(|| {
            let reader = RangedReader::new(frame.connection, stream.path.clone());
            let wake = ctx.clone();
            let options = Options {
                audio: frame.audio,
                max_side: ctx.input(|i| i.max_texture_side),
                wake: std::sync::Arc::new(move || wake.request_repaint()),
            };
            Player::open(reader, stream.size, options)
        });
        player.poll();
        match player.phase() {
            Phase::Failed(line) => return Some(line.clone()),
            Phase::Opening => {
                let t = ui.tokens();
                ui.horizontal(|ui| {
                    ui.add(widgets::spinner());
                    ui.label(t.ty.body.rich("Loading preview…").color(t.color.text.muted));
                });
                return None;
            }
            Phase::Ready => {}
        }
        if !self.started {
            self.started = true;
            if frame.autoplay {
                player.play();
            }
        }
        if let Some(inside) = frame.inside {
            if self.inside && !inside {
                player.pause();
            }
            self.inside = inside;
        }
        if frame.toggle {
            player.toggle();
        }
        if let Some(image) = player.take_frame() {
            let options = TextureOptions::LINEAR;
            match &mut self.frame {
                Some(texture) => texture.set(image, options),
                None => self.frame = Some(ctx.load_texture("preview/media", image, options)),
            }
        }

        controls(ui, player);
        if player.is_playing() {
            ctx.request_repaint_after(TICK);
        }
        let t = ui.tokens();
        ui.add_space(t.space.stack);
        match (&self.frame, player.has_video()) {
            (Some(texture), true) => {
                let [w, h] = texture.size();
                let dimensions = format!("{w} × {h}");
                widgets::zoom_bar(ui, &mut self.zoom, Some(&dimensions));
                let content = egui::vec2(w as f32, h as f32);
                let shown = widgets::zoom_view(
                    ui.scope_id().with("video"),
                    "Video",
                    &mut self.zoom,
                    content,
                )
                .keys(frame.keymap)
                .show(ui);
                widgets::paint_texture(ui, &shown, texture.id());
            }
            (_, true) => {}
            (_, false) => {
                ui.label(t.ty.body.rich(AUDIO_PLACEHOLDER).color(t.color.text.muted));
            }
        }
        None
    }
}

/// Play/Pause, the seek bar, elapsed/total, Mute and the volume.
fn controls(ui: &mut egui::Ui, player: &mut Player) {
    let t = ui.tokens();
    ui.horizontal_wrapped(|ui| {
        let label = if player.is_playing() { "Pause" } else { "Play" };
        if ui.add(widgets::button(label).flat()).clicked() {
            player.toggle();
        }
        let total = player.duration().unwrap_or_default();
        let at = player.position().unwrap_or_default().min(total);
        let mut seconds = at.as_secs_f64();
        let seek = widgets::slider(&mut seconds, 0.0..=total.as_secs_f64().max(0.001))
            .show_value(false)
            .name("Seek");
        if ui.add_enabled(!total.is_zero(), seek).changed() {
            player.seek(Duration::from_secs_f64(seconds));
        }
        let time = format!("{} / {}", clock(at), clock(total));
        ui.label(t.ty.numeric.rich(time).color(t.color.text.muted));
        let muted = player.is_muted();
        if ui.add(widgets::button("Mute").flat().on(muted)).clicked() {
            player.set_muted(!muted);
        }
        let mut volume = player.volume() * 100.0;
        let slider = widgets::slider(&mut volume, 0.0..=100.0)
            .step(1.0)
            .show_value(false)
            .text("Volume");
        if ui.add(slider).changed() {
            player.set_volume(volume / 100.0);
        }
    });
}

/// "1:05", or "1:02:03" from an hour.
pub fn clock(at: Duration) -> String {
    let s = at.as_secs();
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}
