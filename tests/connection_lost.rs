//! Search, Download and Preview when the connection drops mid-operation, against the
//! test server (tests/common/fixture.rs): a Search ends as lost with its Matches, a
//! Download pauses, discards its temp file and fetches that file again once resumed on
//! the new connection, and Preview keeps what it loaded.

mod common;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use common::fixture::{self, Fixture};
use sshexplorer::download::{self, End, Event as DownloadEvent, Job, Options, Queue};
use sshexplorer::preview::{Body, Focused, Preview};
use sshexplorer::scratch::Place;
use sshexplorer::search::{self, Event as SearchEvent, How, Query};
use sshexplorer::ssh::{Event as Opening, Ssh, Target};

const WAIT: Duration = Duration::from_secs(10);

/// A fixture with a Session open on it.
struct Setup {
    fixture: Fixture,
    _ssh: Ssh,
    session: sshexplorer::ssh::Session,
    _known: tempfile::TempDir,
}

impl Setup {
    fn new() -> Setup {
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

    fn remote(&self, path: &str) -> PathBuf {
        self.fixture.root().join(path.trim_start_matches('/'))
    }

    fn write(&self, path: &str, bytes: &[u8]) {
        let at = self.remote(path);
        std::fs::create_dir_all(at.parent().unwrap()).unwrap();
        std::fs::write(at, bytes).unwrap();
    }

    fn entry(&self, dir: &str, name: &str) -> sshexplorer::ssh::Entry {
        let listing = self
            .session
            .list(dir.to_string())
            .recv_timeout(WAIT)
            .unwrap()
            .unwrap();
        listing
            .entries
            .into_iter()
            .find(|e| e.name == name)
            .unwrap()
    }

    /// Drops the connection and opens it again with the held password.
    fn drop_and_reconnect(&self) -> sshexplorer::ssh::Session {
        self.fixture.drop_connections();
        until("the drop", || self.session.lost().is_some());
        match self.session.reconnect().recv_timeout(WAIT) {
            Some(Opening::Opened(opened)) => opened.session,
            _ => panic!("expected the reconnect to open"),
        }
    }
}

fn until(what: &str, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// 1 MiB of bytes that are not all the same.
fn big() -> Vec<u8> {
    (0..1024 * 1024).map(|i| (i % 251) as u8).collect()
}

#[test]
fn parallel_files_restart_on_reconnect_and_queued_files_finish() {
    let s = Setup::new();
    let files = ["a.bin", "b.bin", "c.bin", "d.bin", "e.bin"];
    let contents = big();
    for name in files {
        s.write(&format!("/home/tester/{name}"), &contents);
    }
    let local = tempfile::tempdir().unwrap();
    let entries = files
        .iter()
        .map(|name| s.entry(fixture::HOME, name))
        .collect();
    s.fixture.slow_reads(Duration::from_millis(30));
    let running = download::start(
        &s.session,
        Job {
            dir: fixture::HOME.to_string(),
            entries,
            local_folder: local.path().to_path_buf(),
            options: Options::default(),
        },
    );
    until("bytes to arrive", || running.progress().bytes_done > 0);

    assert_eq!(running.progress().active_files, 4);
    s.fixture.drop_connections();
    until("the pause", || running.progress().lost);
    assert!(std::fs::read_dir(local.path()).unwrap().next().is_none());
    assert_eq!(running.progress().bytes_done, 0);
    assert!(running.try_next().is_none(), "paused, not finished");

    s.fixture.slow_reads(Duration::ZERO);
    let session = match s.session.reconnect().recv_timeout(WAIT) {
        Some(Opening::Opened(opened)) => opened.session,
        _ => panic!("expected the reconnect to open"),
    };
    running.resume(&session);
    let Some(DownloadEvent::Finished(summary)) = running.recv_timeout(WAIT) else {
        panic!("expected the Download to finish");
    };
    assert_eq!(summary.end, End::Finished);
    assert_eq!(summary.copied, files.len());
    assert!(summary.failed.is_empty(), "{:?}", summary.failed);
    for name in files {
        assert_eq!(std::fs::read(local.path().join(name)).unwrap(), contents);
    }
    assert_eq!(
        running.progress().bytes_done,
        (files.len() * contents.len()) as u64
    );
    assert!(!running.progress().lost);
}

#[test]
fn a_paused_download_can_be_cancelled() {
    let s = Setup::new();
    s.write("/home/tester/big.bin", &big());
    let local = tempfile::tempdir().unwrap();
    let entry = s.entry(fixture::HOME, "big.bin");
    s.fixture.slow_reads(Duration::from_millis(30));
    let running = download::start(
        &s.session,
        Job {
            dir: fixture::HOME.to_string(),
            entries: vec![entry],
            local_folder: local.path().to_path_buf(),
            options: Options::default(),
        },
    );
    until("bytes to arrive", || running.progress().bytes_done > 0);
    s.fixture.drop_connections();
    until("the pause", || running.progress().lost);
    running.cancel();
    let Some(DownloadEvent::Finished(summary)) = running.recv_timeout(WAIT) else {
        panic!("expected the Download to end");
    };
    assert_eq!(summary.end, End::Cancelled);
    assert!(!local.path().join("big.bin").exists());
}

#[test]
fn the_queue_starts_nothing_on_a_lost_connection_and_resumes_on_the_new_one() {
    let s = Setup::new();
    s.write("/home/tester/big.bin", &big());
    s.write("/home/tester/small.txt", b"small");
    let local = tempfile::tempdir().unwrap();
    let job = |name: &str| Job {
        dir: fixture::HOME.to_string(),
        entries: vec![s.entry(fixture::HOME, name)],
        local_folder: local.path().to_path_buf(),
        options: Options::default(),
    };
    let (first, second) = (job("big.bin"), job("small.txt"));
    s.fixture.slow_reads(Duration::from_millis(30));
    let mut queue = Queue::default();
    queue.push(&s.session, first);
    queue.push(&s.session, second);
    until("bytes to arrive", || {
        queue.running().unwrap().progress().bytes_done > 0
    });
    s.fixture.slow_reads(Duration::ZERO);
    let session = s.drop_and_reconnect();
    until("the pause", || queue.running().unwrap().progress().lost);
    // Polled on the lost connection: the first stays paused and the second waits.
    queue.poll(&s.session);
    assert_eq!(queue.queued(), 1);

    queue.resume(&session);
    until("both Downloads", || {
        queue.poll(&session);
        !queue.is_busy()
    });
    assert_eq!(std::fs::read(local.path().join("big.bin")).unwrap(), big());
    assert_eq!(
        std::fs::read(local.path().join("small.txt")).unwrap(),
        b"small"
    );
}

#[test]
fn a_search_on_a_dropped_connection_ends_as_lost_after_its_matches() {
    let s = Setup::new();
    s.write("/home/tester/a/match-one", b"x");
    s.write("/home/tester/b/match-two", b"x");
    s.fixture.hold_search();
    let running = search::start(
        &s.session,
        Query {
            start: fixture::HOME.to_string(),
            text: "match".to_string(),
            follow: false,
            cap: 1000,
        },
    );
    let mut found = Vec::new();
    while found.len() < 2 {
        match running.recv_timeout(WAIT).expect("Matches") {
            SearchEvent::Found(batch) => found.extend(batch),
            SearchEvent::Ended(ended) => panic!("ended early: {ended:?}"),
        }
    }
    s.fixture.drop_connections();
    let ended = loop {
        match running.recv_timeout(WAIT).expect("the end") {
            SearchEvent::Found(batch) => found.extend(batch),
            SearchEvent::Ended(ended) => break ended,
        }
    };
    assert_eq!(ended.how, How::Lost);
    assert_eq!(found.len(), 2);
}

fn text_file(name: &str) -> Focused {
    Focused {
        path: format!("/srv/{name}"),
        name: name.into(),
        entry: Some(sshexplorer::ssh::Entry {
            name: name.into(),
            size: Some(5),
            modified: Some(1),
            ..sshexplorer::ssh::Entry::default()
        }),
    }
}

fn failed(preview: &Preview) -> Option<String> {
    match &preview.shown()?.body {
        Body::Failed(line) => Some(line.clone()),
        _ => None,
    }
}

#[test]
fn preview_keeps_what_is_loaded_when_the_connection_drops() {
    let mut preview = Preview::default();
    preview.focus(Some(text_file("a.txt")), 0.0);
    let fetch = preview.due(1.0, 1 << 20).expect("a fetch");
    preview.arrive(fetch.id, Ok(b"hello".to_vec()), &Place::Memory);
    preview.lost();
    assert!(matches!(preview.shown().unwrap().body, Body::Text(_)));
    assert!(!preview.resume(2.0), "nothing to load again");
}

#[test]
fn an_in_flight_preview_load_shows_connection_lost_and_loads_again_on_reconnect() {
    let mut preview = Preview::default();
    preview.focus(Some(text_file("a.txt")), 0.0);
    let fetch = preview.due(1.0, 1 << 20).expect("a fetch");
    preview.lost();
    assert_eq!(
        failed(&preview).as_deref(),
        Some("Cannot read: connection lost")
    );
    // The read's own failure, arriving late, changes nothing.
    preview.arrive(fetch.id, Err("channel closed".into()), &Place::Memory);
    assert_eq!(
        failed(&preview).as_deref(),
        Some("Cannot read: connection lost")
    );

    assert!(preview.resume(2.0));
    assert!(matches!(preview.shown().unwrap().body, Body::Loading));
    let again = preview.due(2.5, 1 << 20).expect("fetched again");
    preview.arrive(again.id, Ok(b"hello".to_vec()), &Place::Memory);
    assert!(matches!(preview.shown().unwrap().body, Body::Text(_)));
}

#[test]
fn no_new_preview_load_starts_while_the_connection_is_lost() {
    let mut preview = Preview::default();
    preview.focus(Some(text_file("b.txt")), 0.0);
    preview.lost();
    assert_eq!(
        failed(&preview).as_deref(),
        Some("Cannot read: connection lost")
    );
    assert!(preview.due(1.0, 1 << 20).is_none());
}
