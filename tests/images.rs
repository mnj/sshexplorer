//! Image Preview in the model: an image is fetched whole, or not at all when it is over
//! the size cap.

use sshexplorer::kind::Kind;
use sshexplorer::preview::{self, Body, Focused, Preview};
use sshexplorer::scratch::Place;
use sshexplorer::ssh::Entry;

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
fn an_image_is_fetched_whole_and_arrives_as_an_image() {
    let mut p = Preview::default();
    p.focus(Some(file("photo.png", 1234)), 0.0);
    assert!(matches!(p.shown().unwrap().body, Body::Loading));
    let fetch = p.due(1.0, 10 * MIB).expect("an image is fetched");
    assert_eq!(fetch.len, 1234);
    p.arrive(fetch.id, Ok(vec![7; 1234]), &Place::Memory);
    let Body::Image(image) = &p.shown().unwrap().body else {
        panic!("an image body");
    };
    assert_eq!(image.fetched.kept.bytes().unwrap().len(), 1234);
    assert!(!image.fetched.capped);
}

#[test]
fn an_image_over_the_cap_is_not_fetched_and_says_it_is_too_large() {
    let mut p = Preview::default();
    let size = 2_469_606_195; // 2.3 GiB
    p.focus(Some(file("huge.tiff", size)), 0.0);
    assert_eq!(p.due(1.0, 10 * MIB), None);
    assert!(matches!(p.shown().unwrap().body, Body::TooLarge(s) if s == size));
    assert_eq!(p.wait(2.0), None);
    assert_eq!(
        preview::too_large_line(size),
        "Too large to preview (2.3 GiB)"
    );
    // Exactly at the cap is not over it.
    p.focus(Some(file("edge.png", 10 * MIB)), 3.0);
    assert!(p.due(4.0, 10 * MIB).is_some());
}

#[test]
fn a_file_without_a_known_extension_that_is_an_image_shows_as_one() {
    let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    let mut p = Preview::default();
    p.focus(Some(file("avatar", png.len() as u64)), 0.0);
    let fetch = p.due(1.0, MIB).unwrap();
    p.arrive(fetch.id, Ok(png), &Place::Memory);
    let shown = p.shown().unwrap();
    assert_eq!(shown.kind, Kind::Image);
    assert!(matches!(shown.body, Body::Image(_)));

    // Sniffed only after a capped fetch: too large.
    p.focus(Some(file("scan", 40 * MIB)), 2.0);
    let fetch = p.due(3.0, MIB).unwrap();
    let mut head = b"\x89PNG\r\n\x1a\n".to_vec();
    head.resize(MIB as usize, 0);
    p.arrive(fetch.id, Ok(head), &Place::Memory);
    assert!(matches!(p.shown().unwrap().body, Body::TooLarge(s) if s == 40 * MIB));
}
