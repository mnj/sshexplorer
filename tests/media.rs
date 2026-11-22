//! The media player apart from the window and the network: it plays from any seekable
//! reader, with the sound going nowhere, so no sound device is needed.

use std::io::Cursor;
use std::sync::Arc;
use std::time::{Duration, Instant};

use sshexplorer::media::{AudioOut, Options, Phase, Player};

fn clip() -> Vec<u8> {
    std::fs::read("tests/media/clip.webm").unwrap()
}

fn tone() -> Vec<u8> {
    std::fs::read("tests/media/tone.ogg").unwrap()
}

fn options() -> Options {
    Options {
        audio: AudioOut::Silent,
        max_side: 8192,
        wake: Arc::new(|| {}),
    }
}

fn open(bytes: Vec<u8>, options: Options) -> Player {
    let size = bytes.len() as u64;
    Player::open(Cursor::new(bytes), Some(size), options)
}

/// Polls `player` until `done` holds, for up to ten seconds.
fn until(player: &mut Player, what: &str, done: impl Fn(&Player) -> bool) {
    let start = Instant::now();
    loop {
        player.poll();
        if done(player) {
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "timed out waiting for {what}: {:?}",
            player.phase()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn ready(player: &mut Player) {
    until(player, "ready", |p| *p.phase() != Phase::Opening);
    assert_eq!(*player.phase(), Phase::Ready);
}

#[test]
fn a_video_shows_its_first_frame_paused() {
    let mut player = open(clip(), options());
    ready(&mut player);
    let frame = player.take_frame().expect("the first frame");
    assert_eq!(frame.size, [64, 48]);
    assert!(!player.is_playing());
    assert_eq!(player.position(), Some(Duration::ZERO));
    let total = player.duration().expect("a duration");
    assert!(
        total > Duration::from_millis(1800) && total < Duration::from_millis(2200),
        "{total:?}"
    );
}

#[test]
fn audio_has_no_frame() {
    let mut player = open(tone(), options());
    ready(&mut player);
    assert!(!player.has_video());
    assert!(player.take_frame().is_none());
    assert!(player.duration().is_some());
}

#[test]
fn a_frame_is_never_larger_than_the_texture_limit() {
    let mut player = open(
        clip(),
        Options {
            max_side: 16,
            ..options()
        },
    );
    ready(&mut player);
    let frame = player.take_frame().expect("the first frame");
    assert_eq!(frame.size, [16, 12]);
}

#[test]
fn playing_moves_on_and_pausing_holds() {
    let mut player = open(tone(), options());
    ready(&mut player);
    player.play();
    assert!(player.is_playing());
    until(&mut player, "playback to move", |p| {
        p.position()
            .is_some_and(|at| at > Duration::from_millis(300))
    });
    player.pause();
    assert!(!player.is_playing());
    std::thread::sleep(Duration::from_millis(100));
    let held = player.position().unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let later = player.position().unwrap();
    assert!(
        later < held + Duration::from_millis(100),
        "{held:?} → {later:?}"
    );
}

#[test]
fn it_plays_to_the_end_and_again_from_the_start() {
    let mut player = open(tone(), options());
    ready(&mut player);
    player.seek(Duration::from_millis(1700));
    player.play();
    until(&mut player, "the end", |p| p.has_ended());
    assert!(!player.is_playing());
    player.play();
    assert!(player.is_playing());
    until(&mut player, "playback from the start", |p| {
        p.position()
            .is_some_and(|at| at < Duration::from_millis(1500))
    });
}

#[test]
fn seeking_moves_the_position_while_paused() {
    let mut player = open(clip(), options());
    ready(&mut player);
    player.take_frame();
    player.seek(Duration::from_secs(1));
    until(&mut player, "the seek", |p| {
        p.position()
            .is_some_and(|at| at >= Duration::from_millis(900) && at <= Duration::from_millis(1100))
    });
    until(&mut player, "the frame there", |p| p.take_frame().is_some());
    assert!(!player.is_playing());
}

#[test]
fn volume_and_mute_are_kept() {
    let mut player = open(tone(), options());
    ready(&mut player);
    assert_eq!(player.volume(), 1.0);
    assert!(!player.is_muted());
    player.set_volume(0.25);
    player.set_muted(true);
    assert_eq!(player.volume(), 0.25);
    assert!(player.is_muted());
    player.set_volume(3.0);
    assert_eq!(player.volume(), 1.0);
}

#[test]
fn a_file_it_cannot_decode_fails_with_one_line() {
    let mut player = open(
        b"not a video at all, just some words".repeat(100),
        options(),
    );
    until(&mut player, "the failure", |p| *p.phase() != Phase::Opening);
    assert_eq!(
        *player.phase(),
        Phase::Failed("Cannot preview: unsupported format".to_owned())
    );
}

/// A reader whose reads fail after the first few bytes, as a dropped connection does.
struct Breaks(Cursor<Vec<u8>>);

impl std::io::Read for Breaks {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.0.position() > 200 {
            return Err(std::io::Error::other("connection lost"));
        }
        let n = buf.len().min(100);
        self.0.read(&mut buf[..n])
    }
}

impl std::io::Seek for Breaks {
    fn seek(&mut self, to: std::io::SeekFrom) -> std::io::Result<u64> {
        self.0.seek(to)
    }
}

#[test]
fn a_failed_read_shows_the_read_failure() {
    let bytes = clip();
    let size = bytes.len() as u64;
    let mut player = Player::open(Breaks(Cursor::new(bytes)), Some(size), options());
    until(&mut player, "the failure", |p| *p.phase() != Phase::Opening);
    assert_eq!(
        *player.phase(),
        Phase::Failed("Cannot read: connection lost".to_owned())
    );
}
