//! Preview apart from egui and the network: which kind a file is, when its fetch starts
//! and what is shown once the bytes arrive or fail to.

use sshexplorer::kind::Kind;
use sshexplorer::preview::{self, Body, Focused, Preview};
use sshexplorer::scratch::Place;
use sshexplorer::ssh::Entry;

const MIB: u64 = 1 << 20;

fn file(name: &str, size: u64, mtime: u32) -> Focused {
    Focused {
        path: format!("/srv/{name}"),
        name: name.into(),
        entry: Some(Entry {
            name: name.into(),
            size: Some(size),
            modified: Some(mtime),
            permissions: Some(0o100644),
            ..Entry::default()
        }),
    }
}

fn dir(name: &str) -> Focused {
    Focused {
        path: format!("/srv/{name}"),
        name: name.into(),
        entry: Some(Entry {
            name: name.into(),
            is_dir: true,
            ..Entry::default()
        }),
    }
}

#[test]
fn the_extension_decides_the_kind_first() {
    assert_eq!(
        preview::detect("main.rs", b"\x89PNG\r\n\x1a\n"),
        Kind::Source
    );
    assert_eq!(preview::detect("photo.png", b"plain words"), Kind::Image);
    assert_eq!(preview::detect("notes.txt", b"\0\0\0"), Kind::Text);
}

#[test]
fn without_a_known_extension_the_first_bytes_decide() {
    let cases: [(&[u8], Kind); 12] = [
        (b"\x89PNG\r\n\x1a\nrest", Kind::Image),
        (b"\xff\xd8\xff\xe0rest", Kind::Image),
        (b"GIF89a....", Kind::Image),
        (b"RIFF\0\0\0\0WEBPVP8 ", Kind::Image),
        (b"%PDF-1.7\n", Kind::Pdf),
        (b"PK\x03\x04rest", Kind::Archive),
        (b"\x1f\x8b\x08\0", Kind::Archive),
        (b"ID3\x04\0\0", Kind::Audio),
        (b"fLaC\0\0\0\x22", Kind::Audio),
        (b"\x1a\x45\xdf\xa3rest", Kind::Video),
        (b"\0\0\0\x18ftypisom", Kind::Video),
        (b"#!/bin/sh\necho hi\n", Kind::Text),
    ];
    for (head, kind) in cases {
        assert_eq!(preview::detect("blob", head), kind, "{head:?}");
    }
    let mut tar = vec![0u8; 512];
    tar[..4].copy_from_slice(b"file");
    tar[257..262].copy_from_slice(b"ustar");
    assert_eq!(preview::detect("backup", &tar), Kind::Archive);
}

#[test]
fn text_is_valid_utf8_without_nul_in_the_first_8_kib() {
    assert_eq!(
        preview::detect("README", "héllo wörld\n".as_bytes()),
        Kind::Text
    );
    assert_eq!(preview::detect("blob", b"abc\0def"), Kind::Other);
    assert_eq!(preview::detect("blob", b"\xc3\x28 bad"), Kind::Other);
    // A NUL past the first 8 KiB is not looked at.
    let mut late = vec![b'a'; 9000];
    late[8500] = 0;
    assert_eq!(preview::detect("blob", &late), Kind::Text);
    // The 8 KiB cut may split a character: that is still text.
    let mut split = vec![b'a'; 8191];
    split.extend_from_slice("é".as_bytes());
    assert_eq!(preview::detect("blob", &split), Kind::Text);
    assert_eq!(preview::detect("empty", b""), Kind::Text);
}

#[test]
fn the_banner_and_failure_lines_read_as_the_spec_says() {
    let total = 2_469_606_195; // 2.3 GiB
    assert_eq!(
        preview::capped_banner(10 * MIB, total),
        "Showing the first 10 MiB of 2.3 GiB"
    );
    assert_eq!(
        preview::failure_line("Permission denied"),
        "Cannot read: permission denied"
    );
    assert_eq!(
        preview::failure_line("No such file"),
        "Cannot read: no longer exists"
    );
    assert_eq!(
        preview::failure_line("connection lost"),
        "Cannot read: connection lost"
    );
}

#[test]
fn the_fetch_starts_once_focus_has_rested_and_not_for_an_entry_left_sooner() {
    let mut p = Preview::default();
    assert!(p.focus(Some(file("a.txt", 5, 1)), 0.0));
    assert!(matches!(p.shown().unwrap().body, Body::Loading));
    assert_eq!(p.due(0.1, 10 * MIB), None);
    // Moving on before it is due: nothing is fetched for a.txt.
    assert!(p.focus(Some(file("b.txt", 7, 1)), 0.15));
    assert_eq!(p.due(0.3, 10 * MIB), None);
    let fetch = p.due(0.36, 10 * MIB).expect("b.txt is due");
    assert_eq!(fetch.path, "/srv/b.txt");
    assert_eq!(fetch.len, 7);
    // Due once only.
    assert_eq!(p.due(1.0, 10 * MIB), None);
}

#[test]
fn the_fetch_is_capped_and_says_how_much_was_left_out() {
    let mut p = Preview::default();
    p.focus(Some(file("big.log", 30 * MIB, 1)), 0.0);
    let fetch = p.due(1.0, 10 * MIB).unwrap();
    assert_eq!(fetch.len, 10 * MIB);
    p.arrive(
        fetch.id,
        Ok(vec![b'x'; (10 * MIB) as usize]),
        &Place::Memory,
    );
    let Body::Text(text) = &p.shown().unwrap().body else {
        panic!("a log shows as text");
    };
    assert_eq!(
        text.fetched.banner().as_deref(),
        Some("Showing the first 10 MiB of 30 MiB")
    );
}

#[test]
fn text_arrives_split_into_lines_and_a_stale_answer_is_ignored() {
    let mut p = Preview::default();
    p.focus(Some(file("a.txt", 12, 1)), 0.0);
    let first = p.due(1.0, MIB).unwrap();
    p.focus(Some(file("b.txt", 12, 1)), 1.0);
    let second = p.due(2.0, MIB).unwrap();
    p.arrive(first.id, Ok(b"from a\n".to_vec()), &Place::Memory);
    assert!(matches!(p.shown().unwrap().body, Body::Loading));
    p.arrive(second.id, Ok(b"one\ntwo\r\nthree".to_vec()), &Place::Memory);
    let Body::Text(text) = &p.shown().unwrap().body else {
        panic!("b.txt shows as text");
    };
    assert_eq!(text.line_count(), 3);
    assert_eq!(text.line(0), "one");
    assert_eq!(text.line(1), "two");
    assert_eq!(text.line(2), "three");
    assert_eq!(text.fetched.banner(), None);
}

#[test]
fn a_file_without_a_known_kind_is_sniffed_after_the_fetch() {
    let mut p = Preview::default();
    p.focus(Some(file("run", 20, 1)), 0.0);
    let fetch = p.due(1.0, MIB).unwrap();
    p.arrive(
        fetch.id,
        Ok(b"#!/bin/sh\necho hi\n".to_vec()),
        &Place::Memory,
    );
    let shown = p.shown().unwrap();
    assert_eq!(shown.kind, Kind::Text);
    assert!(matches!(shown.body, Body::Text(_)));

    p.focus(Some(file("blob", 4, 1)), 2.0);
    let fetch = p.due(3.0, MIB).unwrap();
    p.arrive(fetch.id, Ok(vec![0, 1, 2, 3]), &Place::Memory);
    let shown = p.shown().unwrap();
    assert!(matches!(shown.body, Body::Unsupported));
    assert_eq!(preview::UNSUPPORTED, "No preview for this file type.");
}

#[test]
fn directories_show_the_header_only_and_failures_one_line() {
    let mut p = Preview::default();
    p.focus(Some(dir("www")), 0.0);
    assert!(matches!(p.shown().unwrap().body, Body::Header));
    assert_eq!(p.due(5.0, MIB), None);

    p.focus(Some(file("gone.txt", 3, 1)), 5.0);
    let fetch = p.due(6.0, MIB).unwrap();
    p.arrive(fetch.id, Err("No such file".to_string()), &Place::Memory);
    let Body::Failed(line) = &p.shown().unwrap().body else {
        panic!("a failure line");
    };
    assert_eq!(line, "Cannot read: no longer exists");

    p.focus(None, 7.0);
    assert!(p.shown().is_none());
}

#[test]
fn a_focused_file_reloads_only_when_its_size_or_time_changes() {
    let mut p = Preview::default();
    p.focus(Some(file("a.txt", 5, 1)), 0.0);
    let fetch = p.due(1.0, MIB).unwrap();
    p.arrive(fetch.id, Ok(b"hello".to_vec()), &Place::Memory);
    // The same file again, as a Refresh lists it unchanged: nothing restarts.
    assert!(!p.focus(Some(file("a.txt", 5, 1)), 2.0));
    assert!(matches!(p.shown().unwrap().body, Body::Text(_)));
    assert_eq!(p.due(3.0, MIB), None);
    // Grown: it loads again.
    assert!(p.focus(Some(file("a.txt", 9, 1)), 4.0));
    assert!(p.due(5.0, MIB).is_some());
    // Touched: again.
    assert!(p.focus(Some(file("a.txt", 9, 2)), 6.0));
    assert!(p.due(7.0, MIB).is_some());
}

/// A zip holding `a.txt` ("hello").
fn small_zip() -> Vec<u8> {
    use std::io::Write as _;
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    writer.start_file("a.txt", options).unwrap();
    writer.write_all(b"hello").unwrap();
    writer.finish().unwrap().into_inner()
}

/// Polls `p` until an archive arriving in it is listed, off the caller's thread.
fn listed(p: &mut Preview) {
    let start = std::time::Instant::now();
    while matches!(p.shown().unwrap().body, Body::Loading) {
        assert!(start.elapsed().as_secs() < 10, "the listing never came");
        std::thread::sleep(std::time::Duration::from_millis(1));
        p.poll();
    }
}

#[test]
fn an_archive_arrives_as_its_listing() {
    let bytes = small_zip();
    let mut p = Preview::default();
    p.focus(Some(file("bundle.zip", bytes.len() as u64, 1)), 0.0);
    assert!(matches!(p.shown().unwrap().body, Body::Loading));
    let fetch = p.due(1.0, MIB).expect("a zip is fetched");
    p.arrive(fetch.id, Ok(bytes), &Place::Memory);
    // Listed on a thread of its own: the frame goes on meanwhile, and wakes to look.
    assert!(matches!(p.shown().unwrap().body, Body::Loading));
    assert!(p.wait(1.0).is_some());
    listed(&mut p);
    assert_eq!(p.wait(1.0), None);
    let Body::Archive(listing) = &p.shown().unwrap().body else {
        panic!("a zip shows its listing");
    };
    assert_eq!(listing.members.len(), 1);
    assert_eq!(listing.members[0].path, "a.txt");
    assert_eq!(listing.members[0].size, 5);
}

#[test]
fn an_archive_without_an_archive_name_is_listed_by_its_signature() {
    let bytes = small_zip();
    let mut p = Preview::default();
    p.focus(Some(file("download", bytes.len() as u64, 1)), 0.0);
    let fetch = p.due(1.0, MIB).unwrap();
    p.arrive(fetch.id, Ok(bytes), &Place::Memory);
    listed(&mut p);
    let shown = p.shown().unwrap();
    assert_eq!(shown.kind, Kind::Archive);
    assert!(matches!(shown.body, Body::Archive(_)));
}

#[test]
fn an_archive_over_the_cap_is_not_fetched_and_says_how_large_it_is() {
    let mut p = Preview::default();
    p.focus(Some(file("huge.tar.gz", 2_469_606_195, 1)), 0.0);
    assert_eq!(p.due(1.0, 10 * MIB), None);
    let Body::TooLarge(line) = &p.shown().unwrap().body else {
        panic!("too large to preview");
    };
    assert_eq!(line, "Too large to preview (2.3 GiB)");
    assert_eq!(p.wait(1.0), None);
}

#[test]
fn a_corrupt_archive_shows_one_failure_line_and_others_are_unsupported() {
    let bad = b"PK\x03\x04 not really".to_vec();
    let mut p = Preview::default();
    p.focus(Some(file("bad.zip", bad.len() as u64, 1)), 0.0);
    let fetch = p.due(1.0, MIB).unwrap();
    p.arrive(fetch.id, Ok(bad), &Place::Memory);
    listed(&mut p);
    let Body::Failed(line) = &p.shown().unwrap().body else {
        panic!("a failure line");
    };
    assert_eq!(line, "Cannot preview: corrupt archive");

    // 7z is an archive Preview does not read: nothing is fetched.
    p.focus(Some(file("photos.7z", 100, 1)), 2.0);
    assert!(matches!(p.shown().unwrap().body, Body::Unsupported));
    assert_eq!(p.due(3.0, MIB), None);
}

#[test]
fn unknown_sizes_check_one_extra_byte_and_never_display_a_partial_document_as_complete() {
    for (name, data, truncated) in [
        ("log.txt", &b"abcd"[..], false),
        ("log.txt", &b"abcde"[..], true),
        ("report.pdf", &b"abcde"[..], true),
    ] {
        let mut focus = file(name, 0, 1);
        focus.entry.as_mut().unwrap().size = None;
        let mut preview = Preview::default();
        preview.focus(Some(focus), 0.);
        let read = preview.due(1., 4).unwrap();
        assert_eq!(read.len, 5);
        preview.arrive(read.id, Ok(data.to_vec()), &Place::Memory);
        match &preview.shown().unwrap().body {
            Body::Text(text) => {
                assert_eq!(text.text.as_ref(), "abcd");
                assert_eq!(text.fetched.banner().is_some(), truncated);
            }
            Body::TooLarge(_) if name == "report.pdf" => {}
            _ => panic!("unexpected preview"),
        }
    }
}
