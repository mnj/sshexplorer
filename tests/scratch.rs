//! The Scratch folder: in memory, or private files that go when the preview does.

use sshexplorer::scratch::{self, Place};
use sshexplorer::settings::{PreviewSettings, ScratchChoice};

#[test]
fn in_memory_keeps_the_bytes_and_writes_nothing() {
    let kept = Place::Memory.keep(b"hello".to_vec()).unwrap();
    assert_eq!(&*kept.bytes().unwrap(), b"hello");
    assert_eq!(kept.path(), None);
}

#[test]
fn on_disk_a_file_is_private_and_deleted_when_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let place = Place::Dir(dir.path().join("scratch"));
    let kept = place.keep(b"secret bytes".to_vec()).unwrap();
    let path = kept.path().unwrap().to_owned();
    assert!(path.starts_with(dir.path().join("scratch")));
    assert_eq!(std::fs::read(&path).unwrap(), b"secret bytes");
    assert_eq!(&*kept.bytes().unwrap(), b"secret bytes");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    drop(kept);
    assert!(!path.exists());
}

#[test]
fn leftovers_are_cleared_and_nothing_else_in_a_chosen_directory() {
    let dir = tempfile::tempdir().unwrap();
    let place = Place::Dir(dir.path().to_owned());
    // What a crashed instance left: its pid is no process.
    let left = dir.path().join(format!("{}4294967294-0", scratch::PREFIX));
    std::fs::write(&left, "left behind").unwrap();
    let lock = dir.path().join(format!("{}4294967294.lock", scratch::PREFIX));
    std::fs::write(&lock, "").unwrap();
    std::fs::write(dir.path().join("mine.txt"), "keep me").unwrap();

    let settings = PreviewSettings {
        scratch: ScratchChoice::Chosen,
        scratch_dir: Some(dir.path().to_owned()),
        ..PreviewSettings::default()
    };
    scratch::clear_leftovers(&settings);
    assert!(!left.exists());
    assert!(!lock.exists());
    assert!(dir.path().join("mine.txt").exists());
}

#[test]
fn a_running_instance_keeps_its_files_when_another_launches() {
    let dir = tempfile::tempdir().unwrap();
    let place = Place::Dir(dir.path().to_owned());
    // This process stands in for the running instance.
    let kept = place.keep(b"still shown".to_vec()).unwrap();

    scratch::clear_dir(dir.path());
    assert_eq!(&*kept.bytes().unwrap(), b"still shown");
}

#[test]
fn the_settings_choose_the_place() {
    let mut s = PreviewSettings::default();
    assert_eq!(s.scratch, ScratchChoice::Memory);
    assert_eq!(Place::of(&s), Place::Memory);
    s.scratch = ScratchChoice::Temp;
    assert_eq!(Place::of(&s), Place::Dir(scratch::temp_dir()));
    assert!(scratch::temp_dir().starts_with(std::env::temp_dir()));
    s.scratch = ScratchChoice::Chosen;
    // Chosen with no directory yet keeps the bytes in memory.
    assert_eq!(Place::of(&s), Place::Memory);
    s.scratch_dir = Some("/var/tmp/x".into());
    assert_eq!(Place::of(&s), Place::Dir("/var/tmp/x".into()));
}
