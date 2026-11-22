//! Audio and video in Preview's model: they stream once focus has rested, are never
//! fetched whole and are exempt from the size cap.

use sshexplorer::kind::Kind;
use sshexplorer::preview::{Body, Focused, Preview, Stream};
use sshexplorer::scratch::Place;
use sshexplorer::ssh::Entry;

const MIB: u64 = 1 << 20;

fn file(name: &str, size: Option<u64>) -> Focused {
    Focused {
        path: format!("/srv/{name}"),
        name: name.into(),
        entry: Some(Entry {
            name: name.into(),
            size,
            modified: Some(1),
            ..Entry::default()
        }),
    }
}

#[test]
fn audio_and_video_stream_once_focus_has_rests_whatever_their_size() {
    for (name, kind) in [("film.mkv", Kind::Video), ("song.flac", Kind::Audio)] {
        let mut p = Preview::default();
        p.focus(Some(file(name, Some(3 * 1024 * MIB))), 0.0);
        assert!(matches!(p.shown().unwrap().body, Body::Loading));
        assert_eq!(p.due(0.1, 10 * MIB), None);
        // Nothing to fetch: the panel streams it.
        assert_eq!(p.due(0.3, 10 * MIB), None);
        let shown = p.shown().unwrap();
        assert_eq!(shown.kind, kind);
        let Body::Media(stream) = &shown.body else {
            panic!("{name} streams");
        };
        assert_eq!(
            *stream,
            Stream {
                path: format!("/srv/{name}"),
                size: Some(3 * 1024 * MIB),
            }
        );
        assert_eq!(p.wait(0.3), None);
    }
}

#[test]
fn media_without_a_known_size_still_streams() {
    let mut p = Preview::default();
    p.focus(Some(file("clip.webm", None)), 0.0);
    p.due(1.0, MIB);
    let Body::Media(stream) = &p.shown().unwrap().body else {
        panic!("it streams");
    };
    assert_eq!(stream.size, None);
}

#[test]
fn media_found_by_its_signature_streams_after_the_sniff() {
    let mut p = Preview::default();
    p.focus(Some(file("recording", Some(50 * MIB))), 0.0);
    let fetch = p.due(1.0, 10 * MIB).unwrap();
    p.arrive(fetch.id, Ok(b"OggS\0\x02rest".to_vec()), &Place::Memory);
    let shown = p.shown().unwrap();
    assert_eq!(shown.kind, Kind::Audio);
    let Body::Media(stream) = &shown.body else {
        panic!("it streams");
    };
    assert_eq!(stream.path, "/srv/recording");
    assert_eq!(stream.size, Some(50 * MIB));
}
