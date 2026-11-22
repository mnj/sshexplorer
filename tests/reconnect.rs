//! Reconnect's timing and the strip's words, on plain data: the Session passes the time
//! in.

use sshexplorer::reconnect::Lost;
use sshexplorer::settings::{AppSettings, ReconnectAttempts, Store};

fn lost(attempts: ReconnectAttempts) -> Lost {
    Lost::new("the host closed the connection".into(), attempts, 100.0)
}

#[test]
fn the_first_attempt_waits_one_second() {
    let mut l = lost(ReconnectAttempts::Five);
    assert_eq!(
        l.headline(),
        "Connection lost: the host closed the connection"
    );
    assert_eq!(
        l.status(100.0, ReconnectAttempts::Five).as_deref(),
        Some("Reconnecting in 1 s (attempt 1 of 5)…")
    );
    assert!(!l.due(100.5));
    assert!(l.due(101.0));
    assert_eq!(
        l.status(101.0, ReconnectAttempts::Five).as_deref(),
        Some("Reconnecting (attempt 1 of 5)…")
    );
}

#[test]
fn waits_double_after_each_failure_up_to_a_minute() {
    let mut l = lost(ReconnectAttempts::Unlimited);
    let mut now = 100.0;
    let mut waits = Vec::new();
    for _ in 0..9 {
        let wait = l.wait(now).expect("waiting");
        waits.push(wait);
        now += wait;
        assert!(l.due(now));
        l.failed(ReconnectAttempts::Unlimited, now);
    }
    assert_eq!(waits, [1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 60.0, 60.0, 60.0]);
    assert_eq!(
        l.status(now + 0.2, ReconnectAttempts::Unlimited).as_deref(),
        Some("Reconnecting in 60 s (attempt 10)…")
    );
}

#[test]
fn the_strip_counts_down_in_whole_seconds() {
    let mut l = lost(ReconnectAttempts::Five);
    assert!(l.due(101.0));
    l.failed(ReconnectAttempts::Five, 101.0);
    assert!(l.due(103.0));
    l.failed(ReconnectAttempts::Five, 103.0);
    assert_eq!(
        l.status(103.5, ReconnectAttempts::Five).as_deref(),
        Some("Reconnecting in 4 s (attempt 3 of 5)…")
    );
}

#[test]
fn running_out_of_attempts_stops() {
    let mut l = lost(ReconnectAttempts::Three);
    let mut now = 100.0;
    for _ in 0..3 {
        now += l.wait(now).unwrap();
        assert!(l.due(now));
        l.failed(ReconnectAttempts::Three, now);
    }
    assert!(l.is_stopped());
    assert_eq!(l.wait(now), None);
    assert_eq!(l.status(now, ReconnectAttempts::Three), None);
    assert!(!l.due(now + 1000.0));
}

#[test]
fn off_never_retries_by_itself() {
    let mut l = lost(ReconnectAttempts::Off);
    assert!(l.is_stopped());
    assert!(!l.due(1000.0));
    // Reconnect still makes one attempt, and a failure stops again.
    l.now();
    assert!(l.is_trying());
    l.failed(ReconnectAttempts::Off, 101.0);
    assert!(l.is_stopped());
}

#[test]
fn stop_ends_the_retries_and_reconnect_starts_them_again() {
    let mut l = lost(ReconnectAttempts::Five);
    assert!(l.due(101.0));
    l.failed(ReconnectAttempts::Five, 101.0);
    l.stop();
    assert!(l.is_stopped());
    assert!(!l.due(1000.0));
    l.now();
    assert!(l.is_trying());
    assert_eq!(
        l.status(1000.0, ReconnectAttempts::Five).as_deref(),
        Some("Reconnecting (attempt 1 of 5)…")
    );
    l.failed(ReconnectAttempts::Five, 1000.0);
    assert_eq!(l.wait(1000.0), Some(2.0));
}

#[test]
fn reconnect_now_skips_the_wait() {
    let mut l = lost(ReconnectAttempts::Five);
    l.now();
    assert!(l.is_trying());
    assert!(!l.due(101.0), "the attempt has started already");
}

#[test]
fn reconnect_attempts_default_to_five_and_persist() {
    assert_eq!(
        AppSettings::default().connections.reconnect_attempts,
        ReconnectAttempts::Five
    );
    let dir = tempfile::tempdir().unwrap();
    let store = Store::at(dir.path().join("settings.toml"));
    let mut settings = AppSettings::default();
    settings.connections.reconnect_attempts = ReconnectAttempts::Unlimited;
    store.save(&settings).unwrap();
    assert_eq!(
        store.load().connections.reconnect_attempts,
        ReconnectAttempts::Unlimited
    );
    std::fs::write(
        dir.path().join("settings.toml"),
        "[connections]\nreconnect_attempts = \"often\"\nuse_ssh_config = false\n",
    )
    .unwrap();
    let loaded = store.load().connections;
    assert_eq!(loaded.reconnect_attempts, ReconnectAttempts::Five);
    assert!(!loaded.use_ssh_config);
}
