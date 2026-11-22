//! An entry's Kind, which picks its icon: by extension only.

use sshexplorer::kind::Kind;

#[test]
fn kind_is_by_extension_ignoring_case() {
    let cases = [
        ("notes.txt", Kind::Text),
        ("server.LOG", Kind::Text),
        ("main.rs", Kind::Source),
        ("build.sh", Kind::Source),
        ("config.toml", Kind::Source),
        ("readme.md", Kind::Markdown),
        ("photo.JPG", Kind::Image),
        ("logo.svg", Kind::Image),
        ("manual.pdf", Kind::Pdf),
        ("backup.tar.gz", Kind::Archive),
        ("site.zip", Kind::Archive),
        ("song.mp3", Kind::Audio),
        ("clip.mkv", Kind::Video),
        ("Makefile", Kind::Other),
        ("data.bin", Kind::Other),
        (".profile", Kind::Other),
        ("trailing.", Kind::Other),
    ];
    for (name, kind) in cases {
        assert_eq!(Kind::of(name, false), kind, "{name}");
    }
}

#[test]
fn a_directory_is_a_directory_whatever_its_name() {
    assert_eq!(Kind::of("photos", true), Kind::Directory);
    assert_eq!(Kind::of("backup.zip", true), Kind::Directory);
}
