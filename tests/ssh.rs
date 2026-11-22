//! Opening a Session over SSH against the test server (tests/common/fixture.rs): the host
//! key check, password authentication and the home directory's listing.

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use common::fixture::{self, Fixture};
use sshexplorer::ssh::{Event, Failure, Opening, Ssh, Target};

fn target(fixture: &Fixture) -> Target {
    Target {
        address: fixture.address().to_string(),
        port: fixture.port(),
        account: fixture::ACCOUNT.to_string(),
    }
}

/// A `known_hosts` path in a fresh temp directory, holding `text` unless it is empty.
fn known_hosts(text: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("known_hosts");
    if !text.is_empty() {
        std::fs::write(&path, text).unwrap();
    }
    (dir, path)
}

fn next(opening: &Opening) -> Event {
    opening
        .recv_timeout(Duration::from_secs(10))
        .expect("an event from the opening Session")
}

#[test]
fn an_unknown_key_is_trusted_and_the_password_opens_the_home_listing() {
    let fixture = Fixture::start();
    let (_dir, path) = known_hosts("");
    let wakes = Arc::new(AtomicUsize::new(0));
    let woken = wakes.clone();
    let ssh = Ssh::new(move || {
        woken.fetch_add(1, Ordering::SeqCst);
    });
    let opening = ssh.open(target(&fixture), path.clone());
    // `open` returns at once; nothing has been asked yet.
    assert!(opening.try_next().is_none());

    let Event::TrustHostKey(prompt) = next(&opening) else {
        panic!("expected the trust prompt");
    };
    // Every event wakes the frame thread. The wake runs just after the event is sent,
    // on the background thread, so wait for it briefly.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while wakes.load(Ordering::SeqCst) == 0 && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(wakes.load(Ordering::SeqCst), 1);
    assert_eq!(prompt.host, format!("[127.0.0.1]:{}", fixture.port()));
    assert_eq!(prompt.fingerprint, fixture::HOST_KEY_FINGERPRINT);
    prompt.trust();

    let Event::Password(prompt) = next(&opening) else {
        panic!("expected the password prompt");
    };
    assert!(!prompt.rejected);
    prompt.enter(fixture::PASSWORD.to_string());

    let Event::Opened(opened) = next(&opening) else {
        panic!("expected the Session to open");
    };
    assert_eq!(opened.home.path, fixture::HOME);
    let names: Vec<&str> = opened
        .home
        .entries
        .iter()
        .map(|e| e.name.as_str())
        .collect();
    let mut expected = vec![".profile"];
    expected.extend_from_slice(fixture::HOME_ENTRIES);
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(sorted, expected);
    let docs = opened
        .home
        .entries
        .iter()
        .find(|e| e.name == "docs")
        .unwrap();
    assert!(docs.is_dir);

    // Trust appended the key, so the next open asks nothing about it.
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text, format!("{}\n", fixture.known_hosts_line()));
}

#[test]
fn a_known_key_asks_nothing_and_a_wrong_password_asks_again() {
    let fixture = Fixture::start();
    let (_dir, path) = known_hosts(&format!("# mine\n{}\n", fixture.known_hosts_line()));
    let ssh = Ssh::new(|| {});
    let opening = ssh.open(target(&fixture), path.clone());

    let Event::Password(prompt) = next(&opening) else {
        panic!("a known key goes straight to the password");
    };
    prompt.enter("wrong".to_string());
    let Event::Password(prompt) = next(&opening) else {
        panic!("a rejected password asks again");
    };
    assert!(prompt.rejected);
    prompt.enter(fixture::PASSWORD.to_string());
    assert!(matches!(next(&opening), Event::Opened(_)));
    // Nothing was added to known_hosts.
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text.lines().count(), 2);
}

#[test]
fn declining_the_password_opens_no_session() {
    let fixture = Fixture::start();
    let (_dir, path) = known_hosts(&format!("{}\n", fixture.known_hosts_line()));
    let ssh = Ssh::new(|| {});
    let opening = ssh.open(target(&fixture), path);
    let Event::Password(prompt) = next(&opening) else {
        panic!("expected the password prompt");
    };
    drop(prompt);
    assert!(matches!(next(&opening), Event::Failed(Failure::Cancelled)));
}

#[test]
fn cancelling_an_unknown_key_opens_no_session_and_records_nothing() {
    let fixture = Fixture::start();
    let (_dir, path) = known_hosts("");
    let ssh = Ssh::new(|| {});
    let opening = ssh.open(target(&fixture), path.clone());
    let Event::TrustHostKey(prompt) = next(&opening) else {
        panic!("expected the trust prompt");
    };
    prompt.cancel();
    assert!(matches!(next(&opening), Event::Failed(Failure::Cancelled)));
    assert!(!path.exists());
}

#[test]
fn a_changed_key_is_refused_with_both_fingerprints_and_the_line() {
    let fixture = Fixture::start();
    let recorded = fixture.known_hosts_line_with(fixture::OTHER_KEY);
    let (_dir, path) = known_hosts(&format!(
        "# first\nexample.org ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAILpyrGi4Hlcfe6TxXZCp7KltPy8jQmt6l40b5P6FcwW0\n{recorded}\n"
    ));
    let ssh = Ssh::new(|| {});
    let opening = ssh.open(target(&fixture), path.clone());

    let Event::Failed(Failure::HostKeyChanged(mismatch)) = next(&opening) else {
        panic!("a changed key is refused without a prompt");
    };
    assert_eq!(mismatch.host, format!("[127.0.0.1]:{}", fixture.port()));
    assert_eq!(mismatch.fingerprint, fixture::HOST_KEY_FINGERPRINT);
    assert_eq!(
        mismatch.recorded.fingerprint,
        fixture::OTHER_KEY_FINGERPRINT
    );
    assert_eq!(mismatch.recorded.line_no, 3);
    assert_eq!(mismatch.recorded.line, recorded);
    assert_eq!(mismatch.file, path);
    // Nothing else arrives: the connection is gone.
    assert!(opening.recv_timeout(Duration::from_millis(200)).is_none());
}

#[test]
fn an_unreachable_host_fails_with_the_reason() {
    // A port that was free a moment ago: nothing listens there.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let (_dir, path) = known_hosts("");
    let ssh = Ssh::new(|| {});
    let opening = ssh.open(
        Target {
            address: "127.0.0.1".into(),
            port,
            account: fixture::ACCOUNT.into(),
        },
        path,
    );
    assert!(matches!(
        next(&opening),
        Event::Failed(Failure::Connection(_))
    ));
}
