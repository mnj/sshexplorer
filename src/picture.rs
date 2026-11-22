//! Decoding a fetched image for Preview, off the frame thread: the `image` crate's
//! formats, animated GIF, WebP and PNG frame by frame, and SVG through resvg.
//!
//! egui panics on a texture wider or taller than the GPU's limit
//! (`InputState::max_texture_side`), so every frame is shrunk to fit it. The pixel
//! dimensions the toolbar shows stay the file's own ([`Picture::size`]). An SVG is
//! rasterised large enough that 400 % stays sharp, within the same limit.
//!
//! The `image` crate's default allocation limit (512 MiB) stands, so a small file that
//! claims a vast size fails instead of exhausting memory, and an animation keeps frames
//! only up to [`FRAME_BYTES`].

use std::io::Cursor;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use egui::ColorImage;
use image::codecs::gif::GifDecoder;
use image::codecs::png::PngDecoder;
use image::codecs::webp::WebPDecoder;
use image::{AnimationDecoder, DynamicImage, ImageFormat, ImageReader, RgbaImage};
use resvg::{tiny_skia, usvg};

/// How many bytes of decoded frames an animation keeps; later frames are left out.
pub const FRAME_BYTES: usize = 256 << 20;
/// The longest side an SVG is rasterised at, before the texture limit.
const SVG_SIDE: u32 = 4096;
/// The shortest longest side an SVG is rasterised at, so a small icon stays sharp at
/// 400 %.
const SVG_MIN_SIDE: u32 = 1024;

/// A decoded image.
#[derive(Debug)]
pub struct Picture {
    /// The image's own pixel dimensions, width and height.
    pub size: [u32; 2],
    /// One frame for a still; an animation's frames in order.
    pub frames: Vec<PictureFrame>,
}

/// One frame, within the texture limit, and how long it shows.
#[derive(Debug)]
pub struct PictureFrame {
    pub image: ColorImage,
    /// Zero for a still.
    pub delay: Duration,
}

impl Picture {
    /// The pixel dimensions as the toolbar shows them: "1920 × 1080".
    pub fn dimensions(&self) -> String {
        dimensions(self.size)
    }

    /// Whether it has more than one frame to play.
    pub fn animated(&self) -> bool {
        self.frames.len() > 1
    }
}

/// "1920 × 1080".
pub fn dimensions([w, h]: [u32; 2]) -> String {
    format!("{w} × {h}")
}

/// Decodes `bytes`, the file `name`, shrinking each frame to at most `max_side` pixels a
/// side. The contents decide the format; an SVG is known by its name or its start.
pub fn decode(name: &str, bytes: &[u8], max_side: u32) -> Result<Picture, String> {
    let max_side = max_side.max(1);
    if is_svg(name, bytes) {
        return svg(bytes, max_side);
    }
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let format = reader.format();
    let animated = match format {
        Some(ImageFormat::Gif) => frames(GifDecoder::new(Cursor::new(bytes)), max_side),
        Some(ImageFormat::WebP) => {
            let decoder = WebPDecoder::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
            if decoder.has_animation() {
                frames(Ok(decoder), max_side)
            } else {
                None
            }
        }
        Some(ImageFormat::Png) => {
            let decoder = PngDecoder::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
            if decoder.is_apng().unwrap_or(false) {
                frames(decoder.apng(), max_side)
            } else {
                None
            }
        }
        _ => None,
    };
    if let Some(picture) = animated {
        return picture;
    }
    let image = reader.decode().map_err(|e| e.to_string())?;
    let size = [image.width(), image.height()];
    Ok(Picture {
        size,
        frames: vec![PictureFrame {
            image: still(image, max_side),
            delay: Duration::ZERO,
        }],
    })
}

/// An animation's frames; `None` when the decoder could not start, so the file is read
/// as a still.
fn frames<'a, D: AnimationDecoder<'a>>(
    decoder: image::ImageResult<D>,
    max_side: u32,
) -> Option<Result<Picture, String>> {
    let decoder = decoder.ok()?;
    let mut size = None;
    let mut out = Vec::new();
    let mut bytes = 0;
    for frame in decoder.into_frames() {
        let frame = match frame {
            Ok(frame) => frame,
            // A frame that fails after some played: keep what decoded.
            Err(_) if !out.is_empty() => break,
            Err(e) => return Some(Err(e.to_string())),
        };
        let (numer, denom) = frame.delay().numer_denom_ms();
        let delay = Duration::from_micros(u64::from(numer) * 1000 / u64::from(denom.max(1)));
        let buffer = frame.into_buffer();
        size.get_or_insert([buffer.width(), buffer.height()]);
        let image = rgba(shrink(buffer, max_side));
        bytes += image.pixels.len() * 4;
        out.push(PictureFrame { image, delay });
        if bytes >= FRAME_BYTES {
            break;
        }
    }
    let size = size?;
    Some(Ok(Picture { size, frames: out }))
}

fn still(image: DynamicImage, max_side: u32) -> ColorImage {
    rgba(shrink(image.into_rgba8(), max_side))
}

/// `image` scaled down, in proportion, so neither side is over `max_side`.
fn shrink(image: RgbaImage, max_side: u32) -> RgbaImage {
    let (w, h) = image.dimensions();
    if w <= max_side && h <= max_side {
        return image;
    }
    let [nw, nh] = fit([w, h], max_side);
    image::imageops::resize(&image, nw, nh, image::imageops::FilterType::Triangle)
}

/// `[w, h]` scaled in proportion so the longer side is `side`, at least 1 × 1.
fn fit([w, h]: [u32; 2], side: u32) -> [u32; 2] {
    let scale = side as f64 / w.max(h) as f64;
    [
        ((w as f64 * scale).round() as u32).max(1),
        ((h as f64 * scale).round() as u32).max(1),
    ]
}

fn rgba(image: RgbaImage) -> ColorImage {
    let size = [image.width() as usize, image.height() as usize];
    ColorImage::from_rgba_unmultiplied(size, image.as_raw())
}

fn is_svg(name: &str, bytes: &[u8]) -> bool {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".svg") || lower.ends_with(".svgz") {
        return true;
    }
    let head = &bytes[..bytes.len().min(512)];
    let head = String::from_utf8_lossy(head);
    let head = head.trim_start_matches('\u{feff}').trim_start();
    head.starts_with("<svg") || (head.starts_with("<?xml") && head.contains("<svg"))
}

/// The system fonts, read once, for SVG `<text>`.
fn fonts() -> Arc<usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut db = usvg::fontdb::Database::new();
            db.load_system_fonts();
            Arc::new(db)
        })
        .clone()
}

fn svg(bytes: &[u8], max_side: u32) -> Result<Picture, String> {
    let options = usvg::Options {
        fontdb: fonts(),
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_data(bytes, &options).map_err(|e| e.to_string())?;
    let own = tree.size();
    let size = [
        (own.width().round() as u32).max(1),
        (own.height().round() as u32).max(1),
    ];
    let longest = size[0].max(size[1]);
    let side = (longest * 4)
        .clamp(SVG_MIN_SIDE, SVG_SIDE)
        .max(longest.min(SVG_SIDE))
        .min(max_side);
    let [w, h] = fit(size, side);
    let mut pixmap =
        tiny_skia::Pixmap::new(w, h).ok_or_else(|| "the image has no area".to_owned())?;
    let transform =
        tiny_skia::Transform::from_scale(w as f32 / own.width(), h as f32 / own.height());
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    let image = ColorImage::from_rgba_premultiplied([w as usize, h as usize], pixmap.data());
    Ok(Picture {
        size,
        frames: vec![PictureFrame {
            image,
            delay: Duration::ZERO,
        }],
    })
}
