//! Decoding a fetched image for Preview: stills, animations and SVG, each kept within
//! the largest texture the GPU takes while the pixel dimensions stay the file's own.

use std::io::Cursor;
use std::time::Duration;

use image::codecs::gif::{GifEncoder, Repeat};
use image::{Delay, Frame, ImageFormat, Rgba, RgbaImage};
use sshexplorer::picture;

fn png(w: u32, h: u32) -> Vec<u8> {
    let img = RgbaImage::from_pixel(w, h, Rgba([200, 30, 30, 255]));
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, ImageFormat::Png).unwrap();
    out.into_inner()
}

fn gif(frames: &[(u8, u32)]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = GifEncoder::new(&mut out);
        enc.set_repeat(Repeat::Infinite).unwrap();
        for &(shade, ms) in frames {
            let img = RgbaImage::from_pixel(6, 4, Rgba([shade, shade, shade, 255]));
            enc.encode_frame(Frame::from_parts(
                img,
                0,
                0,
                Delay::from_numer_denom_ms(ms, 1),
            ))
            .unwrap();
        }
    }
    out
}

#[test]
fn a_still_image_decodes_to_one_frame_at_its_own_size() {
    let pic = picture::decode("red.png", &png(30, 20), 2048).unwrap();
    assert_eq!(pic.size, [30, 20]);
    assert_eq!(pic.frames.len(), 1);
    assert_eq!(pic.frames[0].image.size, [30, 20]);
    assert_eq!(
        pic.frames[0].image.pixels[0],
        egui::Color32::from_rgb(200, 30, 30)
    );
    assert_eq!(pic.dimensions(), "30 × 20");
}

#[test]
fn an_image_larger_than_the_texture_limit_is_shrunk_but_keeps_its_dimensions() {
    let pic = picture::decode("wide.png", &png(5000, 100), 1000).unwrap();
    assert_eq!(pic.size, [5000, 100]);
    assert_eq!(pic.frames[0].image.size, [1000, 20]);
    assert_eq!(pic.dimensions(), "5000 × 100");
}

#[test]
fn an_animated_gif_keeps_every_frame_and_its_delay() {
    let pic = picture::decode("spin.gif", &gif(&[(0, 100), (128, 250), (255, 50)]), 2048).unwrap();
    assert_eq!(pic.size, [6, 4]);
    assert_eq!(pic.frames.len(), 3);
    let delays: Vec<_> = pic.frames.iter().map(|f| f.delay).collect();
    assert_eq!(
        delays,
        [
            Duration::from_millis(100),
            Duration::from_millis(250),
            Duration::from_millis(50)
        ]
    );
    assert_eq!(pic.frames[1].image.pixels[0], egui::Color32::from_gray(128));
}

#[test]
fn the_contents_decide_the_format_not_the_name() {
    let pic = picture::decode("misnamed.jpg", &png(3, 3), 2048).unwrap();
    assert_eq!(pic.size, [3, 3]);
}

#[test]
fn svg_is_rasterised_sharp_enough_to_zoom_and_reports_its_own_size() {
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="12">
        <rect width="24" height="12" fill="#00ff00"/></svg>"##;
    let pic = picture::decode("icon.svg", svg, 2048).unwrap();
    assert_eq!(pic.size, [24, 12]);
    assert_eq!(pic.frames.len(), 1);
    let [w, h] = pic.frames[0].image.size;
    // Rasterised larger than it is, so 400 % stays sharp, and in proportion.
    assert!(w >= 24 * 4, "{w}");
    assert_eq!(w, h * 2);
    assert!(w <= 2048);
    assert_eq!(
        pic.frames[0].image.pixels[0],
        egui::Color32::from_rgb(0, 255, 0)
    );
}

#[test]
fn a_file_that_does_not_decode_gives_a_reason() {
    let err = picture::decode("broken.png", b"\x89PNG\r\n\x1a\nnot really", 2048).unwrap_err();
    assert!(!err.is_empty());
    let err = picture::decode("broken.svg", b"<svg", 2048).unwrap_err();
    assert!(!err.is_empty());
}
