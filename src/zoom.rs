//! The zoom toolbar's model, shared by image, PDF and video Preview: Fit or a scale,
//! the presets the toolbar and the right-click menu offer, stepping with the keys,
//! Ctrl+wheel, and the pan that dragging changes.
//!
//! Sizes are the content's own pixels; [`Zoom::layout`] says where the scaled content
//! sits in the view on screen. Content no larger than the view is centred and does not
//! pan. Fit shrinks large content to the view and never enlarges small content. A new
//! [`Zoom`] is at Fit, which is how each newly focused file starts.

use egui::{Pos2, Rect, Vec2, pos2};

/// The presets after Fit, as scales.
pub const PRESETS: [f32; 4] = [0.5, 1.0, 2.0, 4.0];
/// How far Ctrl+wheel goes out (unless Fit is smaller still) and in.
// theme-exempt: a zoom scale, not a size
pub const MIN: f32 = 0.1;
// theme-exempt: a zoom scale, not a size
pub const MAX: f32 = 8.0;

/// The zoom level chosen.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum Level {
    #[default]
    Fit,
    /// Screen pixels per content pixel: 1.0 is 100 %.
    Scale(f32),
}

impl Level {
    /// The toolbar's and the menu's choices, in order: Fit · 50% · 100% · 200% · 400%.
    pub fn choices() -> [Level; 5] {
        [
            Level::Fit,
            Level::Scale(PRESETS[0]),
            Level::Scale(PRESETS[1]),
            Level::Scale(PRESETS[2]),
            Level::Scale(PRESETS[3]),
        ]
    }

    /// "Fit", or the scale as a percentage: "50%", "150%".
    pub fn label(self) -> String {
        match self {
            Level::Fit => "Fit".to_owned(),
            Level::Scale(s) => format!("{}%", (s * 100.0).round()),
        }
    }
}

/// The zoom and pan of one preview.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Zoom {
    pub level: Level,
    /// The content point at the view's centre, in content pixels; `None` is the
    /// content's centre.
    centre: Option<Pos2>,
}

impl Zoom {
    /// The Fit scale of `content` in a view of `view`: the largest that shows all of it,
    /// at most 1.0.
    pub fn fit_scale(content: Vec2, view: Vec2) -> f32 {
        if content.x <= 0.0 || content.y <= 0.0 {
            return 1.0;
        }
        (view.x / content.x).min(view.y / content.y).clamp(0.0, 1.0)
    }

    /// The scale in force for `content` in a view of `view`.
    pub fn scale(&self, content: Vec2, view: Vec2) -> f32 {
        match self.level {
            Level::Fit => Zoom::fit_scale(content, view),
            Level::Scale(s) => s,
        }
    }

    /// "Fit", or the scale as a percentage.
    pub fn label(&self) -> String {
        self.level.label()
    }

    /// Chooses `level`, from the content's centre.
    pub fn set(&mut self, level: Level) {
        self.level = level;
        self.centre = None;
    }

    /// Back to Fit.
    pub fn fit(&mut self) {
        self.set(Level::Fit);
    }

    /// The next step in: the smallest preset larger than the scale in force, where Fit's
    /// scale is `fit`. 400 % stays.
    pub fn zoom_in(&mut self, fit: f32) {
        let now = self.now(fit);
        if let Some(&next) = PRESETS.iter().find(|&&p| p > now + 1e-4) {
            self.level = Level::Scale(next);
        }
    }

    /// The next step out: the largest preset, or Fit, smaller than the scale in force.
    /// A preset that equals Fit's scale is taken before Fit.
    pub fn zoom_out(&mut self, fit: f32) {
        if self.level == Level::Fit {
            return;
        }
        let now = self.now(fit);
        let preset = PRESETS.iter().rev().find(|&&p| p < now - 1e-4).copied();
        self.level = match preset {
            Some(p) if p >= fit - 1e-4 => Level::Scale(p),
            _ if fit < now - 1e-4 => Level::Fit,
            Some(p) => Level::Scale(p),
            None => Level::Fit,
        };
    }

    /// Ctrl+wheel: the scale times `factor`, between [`MIN`] (or Fit, if smaller) and
    /// [`MAX`], keeping the content point under `pointer` where it is.
    pub fn wheel(&mut self, factor: f32, fit: f32, view: Rect, pointer: Pos2, content: Vec2) {
        let before = self.layout(content, view);
        let now = self.now(fit);
        let scale = (now * factor).clamp(MIN.min(fit), MAX);
        let under = pos2(0.0, 0.0) + (pointer - before.min) / now;
        self.level = Level::Scale(scale);
        let centre = under - (pointer - view.center()) / scale;
        self.centre = Some(self.clamped(centre, content, view.size()));
    }

    /// Dragging by `delta` screen pixels moves the content with the pointer, as far as
    /// its edges.
    pub fn drag(&mut self, delta: Vec2, content: Vec2, view: Rect) {
        let scale = self.scale(content, view.size());
        let centre = self.centre(content, view.size()) - delta / scale;
        self.centre = Some(self.clamped(centre, content, view.size()));
    }

    /// Whether `content` is larger than a view of `view` at the scale in force, so that
    /// dragging pans it.
    pub fn pans(&self, content: Vec2, view: Vec2) -> bool {
        let scaled = content * self.scale(content, view);
        scaled.x > view.x + 0.5 || scaled.y > view.y + 0.5
    }

    /// Where the scaled `content` sits on screen in `view`.
    pub fn layout(&self, content: Vec2, view: Rect) -> Rect {
        let scale = self.scale(content, view.size());
        let centre = self.centre(content, view.size());
        Rect::from_min_size(view.center() - centre.to_vec2() * scale, content * scale)
    }

    fn now(&self, fit: f32) -> f32 {
        match self.level {
            Level::Fit => fit,
            Level::Scale(s) => s,
        }
    }

    fn centre(&self, content: Vec2, view: Vec2) -> Pos2 {
        let centre = self.centre.unwrap_or((content / 2.0).to_pos2());
        self.clamped(centre, content, view)
    }

    /// `centre` kept where the view stays on the content; an axis on which the content
    /// is smaller than the view is centred.
    fn clamped(&self, centre: Pos2, content: Vec2, view: Vec2) -> Pos2 {
        let scale = self.scale(content, view);
        let axis = |c: f32, len: f32, view: f32| {
            let half = view / 2.0 / scale;
            if len * scale <= view {
                len / 2.0
            } else {
                c.clamp(half, len - half)
            }
        };
        pos2(
            axis(centre.x, content.x, view.x),
            axis(centre.y, content.y, view.y),
        )
    }
}
