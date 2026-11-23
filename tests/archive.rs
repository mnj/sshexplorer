//! Archive listings apart from egui and the network: the members of a zip, tar or
//! `.tar.gz`, how the format is found, and what a damaged archive gives.

use std::io::{Cursor, Write as _};

use sshexplorer::archive::{self, Limits, Listing, Member, Stamp};

/// A zip holding `files`, each `(path, contents)`; a path ending in `/` is a directory.
fn zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::from_date_and_time(2024, 5, 1, 12, 30, 0).unwrap());
    for (path, contents) in files {
        if let Some(dir) = path.strip_suffix('/') {
            writer.add_directory(dir, options).unwrap();
        } else {
            writer.start_file(*path, options).unwrap();
            writer.write_all(contents).unwrap();
        }
    }
    writer.finish().unwrap().into_inner()
}

/// A tar holding `files`, each `(path, contents)`, modified at `mtime`.
fn tar(files: &[(&str, &[u8])], mtime: u64) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for (path, contents) in files {
        let mut header = tar::Header::new_ustar();
        header.set_size(contents.len() as u64);
        header.set_mtime(mtime);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append_data(&mut header, path, *contents).unwrap();
    }
    builder.into_inner().unwrap()
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

fn listed(name: &str, bytes: &[u8]) -> Listing {
    archive::list(name, bytes)
        .expect("a format it reads")
        .expect("a sound archive")
}

#[test]
fn a_zip_lists_each_member_with_its_size_and_time() {
    let bytes = zip(&[
        ("docs/", b""),
        ("docs/a.txt", b"hello"),
        ("b.bin", &[7; 300]),
    ]);
    let listing = listed("bundle.zip", &bytes);
    let at = Stamp::Civil {
        year: 2024,
        month: 5,
        day: 1,
        hour: 12,
        minute: 30,
    };
    assert_eq!(
        listing.members,
        [
            Member {
                path: "docs/".into(),
                size: 0,
                is_dir: true,
                modified: Some(at),
            },
            Member {
                path: "docs/a.txt".into(),
                size: 5,
                is_dir: false,
                modified: Some(at),
            },
            Member {
                path: "b.bin".into(),
                size: 300,
                is_dir: false,
                modified: Some(at),
            },
        ]
    );
}

#[test]
fn a_tar_and_a_tar_gz_list_each_member_with_its_size_and_time() {
    let plain = tar(
        &[("etc/hosts", b"127.0.0.1 localhost\n"), ("empty", b"")],
        1_700_000_000,
    );
    let expected = [
        Member {
            path: "etc/hosts".into(),
            size: 20,
            is_dir: false,
            modified: Some(Stamp::Unix(1_700_000_000)),
        },
        Member {
            path: "empty".into(),
            size: 0,
            is_dir: false,
            modified: Some(Stamp::Unix(1_700_000_000)),
        },
    ];
    assert_eq!(listed("backup.tar", &plain).members, expected);
    let packed = gzip(&plain);
    assert_eq!(listed("backup.tar.gz", &packed).members, expected);
    assert_eq!(listed("backup.tgz", &packed).members, expected);
}

#[test]
fn the_format_is_found_by_signature_whatever_the_name() {
    let z = zip(&[("a", b"1")]);
    let t = tar(&[("a", b"1")], 0);
    let g = gzip(&t);
    for bytes in [&z, &t, &g] {
        assert_eq!(listed("blob", bytes).members.len(), 1);
        assert_eq!(listed("wrong.zip", bytes).members.len(), 1);
    }
    // An empty zip is only its end record.
    assert!(listed("blob", &zip(&[])).members.is_empty());
}

#[test]
fn a_gz_that_is_not_a_tar_and_unknown_formats_are_not_read() {
    // Not a tar inside, and not named as one.
    assert!(archive::list("notes.txt.gz", &gzip(b"just some words\n")).is_none());
    assert!(archive::list("blob", b"BZh91AY&SY").is_none());
    assert!(archive::list("photo.7z", b"7z\xbc\xaf\x27\x1c").is_none());
}

#[test]
fn which_names_are_worth_fetching() {
    for name in ["a.zip", "a.JAR", "a.tar", "a.tar.gz", "a.tgz", "a.gz"] {
        assert!(archive::may_list(name), "{name}");
    }
    for name in ["a.7z", "a.rar", "a.tar.xz", "a.bz2", "a.deb", "a.iso"] {
        assert!(!archive::may_list(name), "{name}");
    }
}

#[test]
fn a_damaged_archive_is_a_failure_not_a_listing() {
    let z = zip(&[("a.txt", b"hello"), ("b.txt", b"world")]);
    let cut = &z[..z.len() - 10];
    assert!(archive::list("a.zip", cut).unwrap().is_err());

    let t = tar(&[("a.txt", &[1; 2000]), ("b.txt", b"x")], 0);
    // Cut inside the first member's contents.
    assert!(archive::list("a.tar", &t[..1000]).unwrap().is_err());
    let mut bad = t.clone();
    bad[148..156].copy_from_slice(b"garbage!");
    assert!(archive::list("a.tar", &bad).unwrap().is_err());

    let g = gzip(&t);
    assert!(archive::list("a.tgz", &g[..g.len() / 2]).unwrap().is_err());
    // Named as a zip but nothing like one.
    assert!(
        archive::list("a.zip", b"not a zip at all")
            .unwrap()
            .is_err()
    );
    assert!(
        archive::list("a.zip", b"PK\x03\x04 not really")
            .unwrap()
            .is_err()
    );
    assert_eq!(archive::CORRUPT, "Cannot preview: corrupt archive");
}

fn paths(listing: &Listing) -> Vec<&str> {
    listing.members.iter().map(|m| m.path.as_str()).collect()
}

#[test]
fn a_listing_stops_at_the_member_limit_and_says_it_is_partial() {
    let files: Vec<(String, Vec<u8>)> = (0..5).map(|i| (format!("f{i}"), vec![b'x'])).collect();
    let files: Vec<(&str, &[u8])> = files.iter().map(|(p, c)| (p.as_str(), &c[..])).collect();
    let limits = Limits {
        members: 3,
        unpacked: u64::MAX,
    };
    for bytes in [tar(&files, 0), zip(&files)] {
        let listing = archive::list_within("a", &bytes, limits).unwrap().unwrap();
        assert_eq!(paths(&listing), ["f0", "f1", "f2"]);
        assert!(listing.partial);
    }
    let whole = listed("a.tar", &tar(&files, 0));
    assert_eq!(whole.members.len(), 5);
    assert!(!whole.partial);
}

#[test]
fn a_tar_gz_that_unpacks_past_the_limit_lists_what_came_before() {
    // A small download that unpacks to far more: the second member is 4 MiB of zeros.
    let zeros = vec![0u8; 4 << 20];
    let bytes = gzip(&tar(
        &[("first", b"1"), ("bomb", &zeros), ("after", b"2")],
        0,
    ));
    assert!(bytes.len() < 64 << 10);
    let limits = Limits {
        members: usize::MAX,
        unpacked: 1 << 20,
    };
    let listing = archive::list_within("a.tar.gz", &bytes, limits)
        .unwrap()
        .expect("stopping at the limit is not damage");
    assert_eq!(paths(&listing), ["first", "bomb"]);
    assert!(listing.partial);
}
