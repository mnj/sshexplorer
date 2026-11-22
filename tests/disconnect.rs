//! A Session's connection dropping and reconnecting, against the test server
//! (tests/common/fixture.rs): drops are detected, and a reconnect reuses the Session's
//! secret and checks the host key again.

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use common::fixture::{self, Fixture};
use sshexplorer::ssh::{self, Event, Failure, Opened, Opening, Options, Ssh, Target};

fn target(fixture: &Fixture) -> Target {
    Target {
        address: fixture.address().to_string(),
        port: fixture.port(),
        account: fixture::ACCOUNT.to_string(),
    }
}

fn known_hosts(fixture: &Fixture) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("known_hosts");
    std::fs::write(&path, fixture.known_hosts_line()).unwrap();
    (dir, path)
}

fn next(opening: &Opening) -> Event {
    opening
        .recv_timeout(Duration::from_secs(10))
        .expect("an event from the opening Session")
}

/// Opens a Session with the password, as the operator enters it.
fn open(ssh: &Ssh, fixture: &Fixture, options: Options) -> Opened {
    let opening = ssh.connect(target(fixture), options, "");
    let Event::Password(prompt) = next(&opening) else {
        panic!("expected the password prompt");
    };
    prompt.enter(fixture::PASSWORD.into());
    match next(&opening) {
        Event::Opened(opened) => opened,
        Event::Failed(f) => panic!("did not open: {f}"),
        _ => panic!("expected the Session to open"),
    }
}

/// Waits up to 10 s for the connection to be reported lost.
fn wait_lost(session: &ssh::Session) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if let Some(reason) = session.lost() {
            return reason;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("the drop was not detected");
}

#[test]
fn a_killed_connection_is_lost_and_wakes_the_frame() {
    let fixture = Fixture::start();
    let (_dir, path) = known_hosts(&fixture);
    let wakes = Arc::new(AtomicUsize::new(0));
    let woken = wakes.clone();
    let ssh = Ssh::new(move || {
        woken.fetch_add(1, Ordering::SeqCst);
    });
    let opened = open(&ssh, &fixture, Options::new(path));
    assert_eq!(opened.session.lost(), None);
    std::thread::sleep(Duration::from_millis(100));
    let before = wakes.load(Ordering::SeqCst);

    fixture.drop_connections();
    let reason = wait_lost(&opened.session);
    assert_eq!(reason, "the host closed the connection");
    let deadline = Instant::now() + Duration::from_secs(5);
    while wakes.load(Ordering::SeqCst) == before && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        wakes.load(Ordering::SeqCst) > before,
        "the drop wakes the frame"
    );
}

#[test]
fn a_listing_on_a_dropped_connection_fails_and_the_connection_is_lost() {
    let fixture = Fixture::start();
    let (_dir, path) = known_hosts(&fixture);
    let ssh = Ssh::new(|| {});
    let opened = open(&ssh, &fixture, Options::new(path));
    fixture.drop_connections();
    let answer = opened
        .session
        .list(fixture::HOME.into())
        .recv_timeout(Duration::from_secs(10))
        .expect("an answer");
    assert!(answer.is_err());
    assert!(
        opened.session.lost().is_some(),
        "lost once the request failed"
    );
}

#[test]
fn three_unanswered_keepalives_lose_the_connection() {
    let fixture = Fixture::start();
    let (_dir, path) = known_hosts(&fixture);
    let ssh = Ssh::new(|| {});
    let mut options = Options::new(path);
    options.keepalive = Some(Duration::from_millis(200));
    let opened = open(&ssh, &fixture, options);
    // Answered keepalives keep it up.
    std::thread::sleep(Duration::from_millis(1200));
    assert_eq!(opened.session.lost(), None);

    fixture.stall();
    let started = Instant::now();
    let reason = wait_lost(&opened.session);
    assert_eq!(reason, "the host stopped answering");
    // Three keepalives go unanswered before the fourth tick gives up.
    assert!(started.elapsed() >= Duration::from_millis(600));
}

#[test]
fn reconnecting_reuses_the_entered_password_and_lists_again() {
    let fixture = Fixture::start();
    let (_dir, path) = known_hosts(&fixture);
    let ssh = Ssh::new(|| {});
    let opened = open(&ssh, &fixture, Options::new(path));
    fixture.drop_connections();
    wait_lost(&opened.session);

    let opening = opened.session.reconnect();
    let Event::Opened(again) = next(&opening) else {
        panic!("expected the Session to reopen without asking");
    };
    assert_eq!(again.session.lost(), None);
    let listing = again
        .session
        .list(fixture::HOME.into())
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .unwrap();
    assert!(listing.entries.iter().any(|e| e.name == "notes.txt"));

    // The new connection keeps the secret for the next reconnect too.
    fixture.drop_connections();
    wait_lost(&again.session);
    let opening = again.session.reconnect();
    assert!(matches!(next(&opening), Event::Opened(_)));
    let tries = fixture.attempts();
    assert_eq!(tries, ["password", "password", "password"]);
}

#[test]
fn a_refused_reconnect_fails_as_a_connection_failure() {
    let fixture = Fixture::start();
    let (_dir, path) = known_hosts(&fixture);
    let ssh = Ssh::new(|| {});
    let opened = open(&ssh, &fixture, Options::new(path));
    fixture.refuse(true);
    fixture.drop_connections();
    wait_lost(&opened.session);
    let opening = opened.session.reconnect();
    assert!(matches!(
        next(&opening),
        Event::Failed(Failure::Connection(_))
    ));
}

#[test]
fn a_rejected_secret_on_reconnect_asks_for_it() {
    let fixture = Fixture::start();
    let (_dir, path) = known_hosts(&fixture);
    let ssh = Ssh::new(|| {});
    let opened = open(&ssh, &fixture, Options::new(path));
    fixture.set_password("changed");
    fixture.drop_connections();
    wait_lost(&opened.session);
    let opening = opened.session.reconnect();
    let Event::Password(prompt) = next(&opening) else {
        panic!("expected the password prompt");
    };
    assert!(prompt.rejected);
    prompt.enter("changed".into());
    assert!(matches!(next(&opening), Event::Opened(_)));
}

#[test]
fn a_changed_host_key_on_reconnect_is_refused() {
    let fixture = Fixture::start();
    let (_dir, path) = known_hosts(&fixture);
    let ssh = Ssh::new(|| {});
    let opened = open(&ssh, &fixture, Options::new(path.clone()));
    std::fs::write(&path, fixture.known_hosts_line_with(fixture::OTHER_KEY)).unwrap();
    fixture.drop_connections();
    wait_lost(&opened.session);
    let opening = opened.session.reconnect();
    assert!(matches!(
        next(&opening),
        Event::Failed(Failure::HostKeyChanged(_))
    ));
}

#[test]
fn an_unknown_host_key_on_reconnect_asks_to_trust_it() {
    let fixture = Fixture::start();
    let (_dir, path) = known_hosts(&fixture);
    let ssh = Ssh::new(|| {});
    let opened = open(&ssh, &fixture, Options::new(path.clone()));
    std::fs::write(&path, "").unwrap();
    fixture.drop_connections();
    wait_lost(&opened.session);
    let opening = opened.session.reconnect();
    let Event::TrustHostKey(prompt) = next(&opening) else {
        panic!("expected the trust prompt");
    };
    prompt.trust();
    assert!(matches!(next(&opening), Event::Opened(_)));
}
