//! PDF Preview apart from the window (issue #36): a document opened and rasterised on a
//! thread of its own, and the layout of its pages stacked top to bottom.
//!
//! hayro reads and renders the PDF. It is pure Rust, so nothing is installed or shipped
//! beside the app. Its render cache is not `Send`, so each open PDF has one
//! [`Renderer`] thread that owns the document and its cache and takes render requests.
//! Only the newest list of requests counts: pages that scrolled away before their turn
//! are never drawn.
//!
//! A PDF that needs a password shows [`ENCRYPTED`]; there is no password prompt and no
//! text selection. One that only restricts permissions opens with the empty password.

use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use std::ops::Range;

use egui::{ColorImage, Rect, Vec2, pos2, vec2};
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::{LoadPdfError, Pdf};
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{PixmapSettings, RenderCache, RenderSettings};

/// The line for a PDF that needs a password.
pub const ENCRYPTED: &str = "Cannot preview: encrypted PDF";
/// The line for bytes hayro cannot read as a PDF.
pub const INVALID: &str = "Cannot preview: not a valid PDF";

/// The pages stacked top to bottom with a gap between them, each centred across the
/// widest, in points. Times the zoom scale, that is where they sit on screen.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pages: Vec<Rect>,
    size: Vec2,
}

impl Layout {
    /// The layout of pages of `sizes` points, `gap` points apart.
    pub fn new(sizes: &[[f32; 2]], gap: f32) -> Layout {
        let width = sizes.iter().map(|s| s[0]).fold(0.0, f32::max);
        let mut top = 0.0;
        let pages = sizes
            .iter()
            .map(|&[w, h]| {
                let rect = Rect::from_min_size(pos2((width - w) / 2.0, top), vec2(w, h));
                top += h + gap;
                rect
            })
            .collect();
        let height = if sizes.is_empty() { 0.0 } else { top - gap };
        Layout {
            pages,
            size: vec2(width, height),
        }
    }

    pub fn len(&self) -> usize {
        self.pages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pages.is_empty()
    }

    /// The whole stack's size.
    pub fn size(&self) -> Vec2 {
        self.size
    }

    /// Where page `index` sits.
    pub fn page(&self, index: usize) -> Rect {
        self.pages[index]
    }

    /// The pages `view` overlaps, which are the ones to rasterise.
    pub fn visible(&self, view: Rect) -> Range<usize> {
        let start = self.pages.partition_point(|p| p.max.y <= view.min.y);
        let end = self.pages.partition_point(|p| p.min.y < view.max.y);
        start..end.max(start)
    }

    /// The page the counter names for `view`: the one at its middle, or in a gap the
    /// next one. At the top it is the first, and scrolled to the end the last.
    pub fn current(&self, view: Rect) -> usize {
        if self.pages.is_empty() || view.min.y <= 0.0 {
            return 0;
        }
        if view.max.y >= self.size.y - 0.5 {
            return self.pages.len() - 1;
        }
        let middle = view.center().y;
        self.pages
            .partition_point(|p| p.max.y <= middle)
            .min(self.pages.len() - 1)
    }

    /// The page counter for `view`: "3 / 12".
    pub fn counter(&self, view: Rect) -> String {
        format!("{} / {}", self.current(view) + 1, self.pages.len())
    }
}

/// What the renderer thread says.
pub enum Event {
    /// The document opened: each page's size in points, in order.
    Opened(Vec<[f32; 2]>),
    /// It did not: the failure line.
    Failed(String),
    /// A page asked for with [`Renderer::want`].
    Page(Page),
}

impl std::fmt::Debug for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Event::Opened(sizes) => f.debug_tuple("Opened").field(sizes).finish(),
            Event::Failed(line) => f.debug_tuple("Failed").field(line).finish(),
            Event::Page(p) => f
                .debug_struct("Page")
                .field("index", &p.index)
                .field("scale", &p.scale)
                .field("size", &p.image.size)
                .finish(),
        }
    }
}

/// One rendered page.
pub struct Page {
    pub index: usize,
    /// The scale it was asked for; the image may be smaller, within the texture limit.
    pub scale: f32,
    pub image: ColorImage,
}

/// A page to render, at `scale` pixels per point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Want {
    pub page: usize,
    pub scale: f32,
}

/// One open PDF's renderer thread.
pub struct Renderer {
    wants: Sender<Vec<Want>>,
    events: Receiver<Event>,
}

impl Renderer {
    /// Opens `bytes` on a new thread, which renders no side longer than `max_side`
    /// pixels and calls `wake` after each event.
    pub fn start(bytes: Vec<u8>, max_side: u32, wake: impl Fn() + Send + 'static) -> Renderer {
        let (wants, wanted) = mpsc::channel();
        let (send, events) = mpsc::channel();
        std::thread::Builder::new()
            .name("preview-pdf".into())
            .spawn(move || {
                // hayro is young: a file that makes it panic fails that preview, not the
                // app, and not with a spinner forever.
                let failed = send.clone();
                let wake = std::sync::Arc::new(wake);
                let woken = wake.clone();
                let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run(bytes, max_side, wanted, send, move || woken())
                }));
                if ran.is_err() {
                    let _ = failed.send(Event::Failed(INVALID.to_owned()));
                    wake();
                }
            })
            .expect("spawn the PDF renderer");
        Renderer { wants, events }
    }

    /// The pages to render now, in order. It replaces the list before: a page asked
    /// for earlier and not yet drawn is dropped unless it is asked for again.
    pub fn want(&self, wants: Vec<Want>) {
        let _ = self.wants.send(wants);
    }

    /// The next event, if one is waiting.
    pub fn try_next(&self) -> Option<Event> {
        self.events.try_recv().ok()
    }

    /// The next event, waiting up to `timeout`.
    pub fn recv_timeout(&self, timeout: Duration) -> Option<Event> {
        self.events.recv_timeout(timeout).ok()
    }
}

fn run(
    bytes: Vec<u8>,
    max_side: u32,
    wanted: Receiver<Vec<Want>>,
    events: Sender<Event>,
    wake: impl Fn(),
) {
    let send = |event: Event| {
        let sent = events.send(event).is_ok();
        wake();
        sent
    };
    let pdf = match Pdf::new(bytes) {
        Ok(pdf) => pdf,
        Err(LoadPdfError::Decryption(_)) => {
            send(Event::Failed(ENCRYPTED.to_owned()));
            return;
        }
        Err(LoadPdfError::Invalid) => {
            send(Event::Failed(INVALID.to_owned()));
            return;
        }
    };
    let pages = pdf.pages();
    let sizes = pages
        .iter()
        .map(|p| {
            let (w, h) = p.render_dimensions();
            [w, h]
        })
        .collect();
    if !send(Event::Opened(sizes)) {
        return;
    }
    let cache = RenderCache::new();
    let interpreter = InterpreterSettings::default();
    let render_settings = RenderSettings::default();
    let mut queue: Vec<Want> = Vec::new();
    loop {
        if queue.is_empty() {
            match wanted.recv() {
                Ok(wants) => queue = wants,
                Err(_) => return,
            }
        }
        // Only the newest list counts.
        while let Ok(wants) = wanted.try_recv() {
            queue = wants;
        }
        if queue.is_empty() {
            continue;
        }
        let want = queue.remove(0);
        let Some(page) = pages.get(want.page) else {
            continue;
        };
        let (w, h) = page.render_dimensions();
        let scale = fitted_scale([w, h], want.scale, max_side);
        let pixmap = hayro::render(
            page,
            &cache,
            &interpreter,
            &render_settings,
            &PixmapSettings {
                x_scale: scale,
                y_scale: scale,
                bg_color: WHITE,
            },
        );
        let size = [pixmap.width() as usize, pixmap.height() as usize];
        let image = ColorImage::from_rgba_premultiplied(size, pixmap.data_as_u8_slice());
        let page = Page {
            index: want.page,
            scale: want.scale,
            image,
        };
        if !send(Event::Page(page)) {
            return;
        }
    }
}

/// `scale`, lowered as far as needed for a page of `size` points to fit `max_side`
/// pixels on each side (and hayro's 16-bit sizes).
fn fitted_scale(size: [f32; 2], scale: f32, max_side: u32) -> f32 {
    let limit = max_side.min(u16::MAX as u32) as f32;
    let longest = size[0].max(size[1]).max(1.0);
    scale.min(limit / longest).max(0.0)
}
