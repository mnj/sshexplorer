//! PDF Preview apart from the window: opening a document on its renderer thread, the
//! failure lines, rendering pages within the texture limit, and how the pages stack.

mod common;

use std::time::Duration;

use common::pdf;
use egui::{Rect, pos2, vec2};
use sshexplorer::kind::Kind;
use sshexplorer::pdf::{self as model, Event, Layout, Renderer, Want};
use sshexplorer::preview::{Body, Focused, Preview};
use sshexplorer::scratch::Place;
use sshexplorer::ssh::Entry;

const WAIT: Duration = Duration::from_secs(10);

fn next(renderer: &Renderer) -> Event {
    renderer.recv_timeout(WAIT).expect("the renderer answers")
}

#[test]
fn a_pdf_opens_with_the_size_of_each_page() {
    let r = Renderer::start(pdf::pages(&[(612, 792), (842, 595)]), 8192, || {});
    match next(&r) {
        Event::Opened(sizes) => assert_eq!(sizes, vec![[612.0, 792.0], [842.0, 595.0]]),
        other => panic!("expected Opened, got {other:?}"),
    }
}

#[test]
fn an_encrypted_pdf_cannot_be_previewed() {
    let r = Renderer::start(pdf::encrypted(&[(612, 792)]), 8192, || {});
    match next(&r) {
        Event::Failed(line) => assert_eq!(line, "Cannot preview: encrypted PDF"),
        other => panic!("expected Failed, got {other:?}"),
    }
    assert_eq!(model::ENCRYPTED, "Cannot preview: encrypted PDF");
}

#[test]
fn a_broken_pdf_says_so() {
    let r = Renderer::start(b"%PDF-1.4\nnot really".to_vec(), 8192, || {});
    match next(&r) {
        Event::Failed(line) => assert_eq!(line, "Cannot preview: not a valid PDF"),
        other => panic!("expected Failed, got {other:?}"),
    }
}

fn opened(r: &Renderer) {
    assert!(matches!(next(r), Event::Opened(_)));
}

fn page(r: &Renderer) -> model::Page {
    match next(r) {
        Event::Page(p) => p,
        other => panic!("expected a page, got {other:?}"),
    }
}

#[test]
fn a_page_renders_at_the_scale_asked_for() {
    let r = Renderer::start(pdf::pages(&[(612, 792), (842, 595)]), 8192, || {});
    opened(&r);
    r.want(vec![Want {
        page: 1,
        scale: 2.0,
    }]);
    let p = page(&r);
    assert_eq!((p.index, p.scale), (1, 2.0));
    assert_eq!(p.image.size, [1684, 1190]);
    // The page is filled 50 % grey.
    let mid = p.image.pixels[p.image.size[0] * 595 + 842];
    assert!((120..=135).contains(&mid.r()), "{mid:?}");
}

#[test]
fn a_page_is_never_larger_than_the_texture_limit() {
    let r = Renderer::start(pdf::pages(&[(612, 792)]), 1000, || {});
    opened(&r);
    r.want(vec![Want {
        page: 0,
        scale: 4.0,
    }]);
    let p = page(&r);
    assert_eq!(p.scale, 4.0, "says the scale asked for");
    let [w, h] = p.image.size;
    assert_eq!(h, 1000);
    assert!((772..=774).contains(&w), "keeps the shape: {w}");
}

#[test]
fn a_newer_list_replaces_the_pages_not_yet_drawn() {
    let sizes = [(612, 792); 6];
    let r = Renderer::start(pdf::pages(&sizes), 8192, || {});
    opened(&r);
    let at = |page| Want { page, scale: 1.0 };
    r.want(vec![at(0), at(1), at(2)]);
    r.want(vec![at(5)]);
    let mut drawn = Vec::new();
    while let Some(Event::Page(p)) = r.recv_timeout(Duration::from_millis(500)) {
        drawn.push(p.index);
    }
    assert!(drawn.ends_with(&[5]), "{drawn:?}");
    assert!(!drawn.contains(&2), "page 2 was dropped: {drawn:?}");
    // An index past the last page is skipped.
    r.want(vec![at(9), at(4)]);
    assert_eq!(page(&r).index, 4);
}

const MIB: u64 = 1 << 20;

fn file(name: &str, size: u64) -> Focused {
    Focused {
        path: format!("/srv/{name}"),
        name: name.into(),
        entry: Some(Entry {
            name: name.into(),
            size: Some(size),
            modified: Some(1),
            ..Entry::default()
        }),
    }
}

#[test]
fn a_pdf_is_fetched_whole_and_arrives_as_a_pdf() {
    let bytes = pdf::pages(&[(612, 792)]);
    let mut p = Preview::default();
    p.focus(Some(file("report.pdf", bytes.len() as u64)), 0.0);
    assert!(matches!(p.shown().unwrap().body, Body::Loading));
    let fetch = p.due(1.0, 10 * MIB).expect("a PDF is fetched");
    assert_eq!(fetch.len, bytes.len() as u64);
    p.arrive(fetch.id, Ok(bytes.clone()), &Place::Memory);
    let Body::Pdf(doc) = &p.shown().unwrap().body else {
        panic!("a PDF body");
    };
    assert_eq!(doc.fetched.kept.bytes().unwrap().as_ref(), bytes.as_slice());
}

#[test]
fn a_pdf_over_the_cap_is_not_fetched_and_says_it_is_too_large() {
    let mut p = Preview::default();
    p.focus(Some(file("scan.pdf", 2_469_606_195)), 0.0);
    assert_eq!(p.due(1.0, 10 * MIB), None);
    assert!(
        matches!(&p.shown().unwrap().body, Body::TooLarge(s) if s == "Too large to preview (2.3 GiB)")
    );
}

#[test]
fn a_file_without_a_known_extension_that_is_a_pdf_shows_as_one() {
    let bytes = pdf::pages(&[(612, 792)]);
    let mut p = Preview::default();
    p.focus(Some(file("statement", bytes.len() as u64)), 0.0);
    let fetch = p.due(1.0, MIB).unwrap();
    p.arrive(fetch.id, Ok(bytes), &Place::Memory);
    let shown = p.shown().unwrap();
    assert_eq!(shown.kind, Kind::Pdf);
    assert!(matches!(shown.body, Body::Pdf(_)));

    // Sniffed only after a capped fetch: too large.
    p.focus(Some(file("archive-scan", 40 * MIB)), 2.0);
    let fetch = p.due(3.0, MIB).unwrap();
    let mut head = b"%PDF-1.7\n".to_vec();
    head.resize(MIB as usize, b' ');
    p.arrive(fetch.id, Ok(head), &Place::Memory);
    assert!(
        matches!(&p.shown().unwrap().body, Body::TooLarge(s) if s == "Too large to preview (40 MiB)")
    );
}

fn three() -> Layout {
    Layout::new(&[[612.0, 792.0], [842.0, 595.0], [612.0, 792.0]], 8.0)
}

fn view(top: f32, height: f32) -> Rect {
    Rect::from_min_size(pos2(0.0, top), vec2(400.0, height))
}

#[test]
fn pages_stack_top_to_bottom_centred_with_a_gap() {
    let l = three();
    assert_eq!(l.len(), 3);
    assert_eq!(l.size(), vec2(842.0, 792.0 + 8.0 + 595.0 + 8.0 + 792.0));
    assert_eq!(
        l.page(0),
        Rect::from_min_size(pos2(115.0, 0.0), vec2(612.0, 792.0))
    );
    assert_eq!(
        l.page(1),
        Rect::from_min_size(pos2(0.0, 800.0), vec2(842.0, 595.0))
    );
    assert_eq!(l.page(2).min, pos2(115.0, 1403.0));
}

#[test]
fn the_pages_in_view_are_those_it_overlaps() {
    let l = three();
    assert_eq!(l.visible(view(0.0, 300.0)), 0..1);
    assert_eq!(l.visible(view(700.0, 300.0)), 0..2);
    // Only the gap between pages 2 and 3.
    assert_eq!(l.visible(view(1396.0, 6.0)), 2..2);
    assert_eq!(l.visible(view(1000.0, 5000.0)), 1..3);
    assert_eq!(l.visible(view(0.0, 10_000.0)), 0..3);
}

#[test]
fn the_page_counter_names_the_page_at_the_middle_of_the_view() {
    let l = three();
    assert_eq!(l.counter(view(0.0, 300.0)), "1 / 3");
    // The middle (1000) is on page 2.
    assert_eq!(l.counter(view(850.0, 300.0)), "2 / 3");
    // In the gap after page 2, the next page.
    assert_eq!(l.counter(view(1249.0, 300.0)), "3 / 3");
    // Scrolled to the end: the last page, wherever the middle is.
    let end = l.size().y;
    assert_eq!(l.counter(view(end - 2000.0, 2000.0)), "3 / 3");
    // Everything in view: the first.
    assert_eq!(l.counter(view(0.0, end + 100.0)), "1 / 3");
}
