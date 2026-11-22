//! Media read in ranges over a Session's connection, against the test server.

mod common;

use std::io::{Read as _, Seek as _, SeekFrom};
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::fixture::{self, Fixture};
use sshexplorer::media::{AudioOut, Options, Phase, Player, RangedReader};
use sshexplorer::ssh::{Event as Opening, Ssh, Target};

const WAIT: Duration = Duration::from_secs(10);

struct Setup {
    fixture: Fixture,
    _ssh: Ssh,
    session: sshexplorer::ssh::Session,
    _known: tempfile::TempDir,
}

fn setup() -> Setup {
    let fixture = Fixture::start();
    let ssh = Ssh::new(|| {});
    let known = tempfile::tempdir().unwrap();
    let path = known.path().join("known_hosts");
    std::fs::write(&path, format!("{}\n", fixture.known_hosts_line())).unwrap();
    let target = Target {
        address: fixture.address().to_string(),
        port: fixture.port(),
        account: fixture::ACCOUNT.to_string(),
    };
    let opening = ssh.open(target, path);
    let Some(Opening::Password(prompt)) = opening.recv_timeout(WAIT) else {
        panic!("expected the password prompt");
    };
    prompt.enter(fixture::PASSWORD.to_string());
    let Some(Opening::Opened(opened)) = opening.recv_timeout(WAIT) else {
        panic!("expected the Session to open");
    };
    Setup {
        fixture,
        _ssh: ssh,
        session: opened.session,
        _known: known,
    }
}

#[test]
fn the_reader_reads_any_range_of_a_remote_file() {
    let s = setup();
    let bytes: Vec<u8> = (0..700_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(s.fixture.root().join("home/tester/big.bin"), &bytes).unwrap();
    let mut reader = RangedReader::new(&s.session, "/home/tester/big.bin".into());

    let mut head = [0; 10];
    reader.read_exact(&mut head).unwrap();
    assert_eq!(head[..], bytes[..10]);

    reader.seek(SeekFrom::Start(400_000)).unwrap();
    let mut middle = vec![0; 300_000];
    reader.read_exact(&mut middle).unwrap();
    assert_eq!(middle[..], bytes[400_000..700_000]);

    reader.seek(SeekFrom::End(-4)).unwrap();
    let mut rest = Vec::new();
    reader.read_to_end(&mut rest).unwrap();
    assert_eq!(rest[..], bytes[700_000 - 4..]);
}

#[test]
fn a_missing_file_reads_as_no_such_file() {
    let s = setup();
    let mut reader = RangedReader::new(&s.session, "/home/tester/gone.webm".into());
    let error = reader.read(&mut [0; 16]).unwrap_err();
    assert_eq!(error.to_string(), "No such file");
}

#[test]
fn a_remote_video_plays_from_ranged_reads() {
    let s = setup();
    let clip = std::fs::read("tests/media/clip.webm").unwrap();
    std::fs::write(s.fixture.root().join("home/tester/clip.webm"), &clip).unwrap();
    let reader = RangedReader::new(&s.session, "/home/tester/clip.webm".into());
    let options = Options {
        audio: AudioOut::Silent,
        max_side: 8192,
        wake: Arc::new(|| {}),
    };
    let mut player = Player::open(reader, Some(clip.len() as u64), options);
    let start = Instant::now();
    while *player.phase() == Phase::Opening {
        assert!(start.elapsed() < WAIT, "timed out opening");
        std::thread::sleep(Duration::from_millis(10));
        player.poll();
    }
    assert_eq!(*player.phase(), Phase::Ready);
    assert_eq!(player.take_frame().unwrap().size, [64, 48]);
}
