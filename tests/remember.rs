//! Remember at the SSH layer: a Session of a remembering Host reads its secret from the
//! credential store, writes the one that logged in once the Session is open, and asks
//! again when the host rejects it. The store is a MemoryStore, never the OS one.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::fixture::{self, Accepts, Fixture, Keyboard};
use common::keys::{PASSPHRASE, key_path, public_key};
use sshexplorer::credentials::{self, Key, Kind, MemoryStore};
use sshexplorer::hosts::AuthMethod;
use sshexplorer::ssh::{Event, Failure, Opened, Opening, Options, Remember, Saved, Ssh, Target};

const HOST: &str = "host-1";

struct Setup {
    fixture: Fixture,
    _dir: tempfile::TempDir,
    options: Options,
    store: Arc<MemoryStore>,
}

fn setup(accepts: Accepts, auth: AuthMethod) -> Setup {
    let fixture = Fixture::start_accepting(accepts);
    let dir = tempfile::tempdir().unwrap();
    let known_hosts = dir.path().join("known_hosts");
    std::fs::write(&known_hosts, format!("{}\n", fixture.known_hosts_line())).unwrap();
    let store = Arc::new(MemoryStore::default());
    let mut options = Options::new(known_hosts);
    options.auth = auth;
    options.remember = Some(Remember {
        store: store.clone(),
        host: HOST.into(),
    });
    Setup {
        fixture,
        _dir: dir,
        options,
        store,
    }
}

fn password() -> Accepts {
    Accepts {
        password: true,
        ..Accepts::default()
    }
}

fn password_key() -> Key {
    Key::new(HOST, Kind::Password)
}

fn passphrase_key() -> Key {
    Key::new(HOST, Kind::Passphrase)
}

impl Setup {
    fn open(&self, ssh: &Ssh) -> Opening {
        let target = Target {
            address: self.fixture.address().to_string(),
            port: self.fixture.port(),
            account: fixture::ACCOUNT.to_string(),
        };
        ssh.connect(target, self.options.clone(), "")
    }

    fn keep(&self, key: &Key, secret: &str) {
        use credentials::Store as _;
        self.store.set(key, secret).unwrap();
    }
}

fn next(opening: &Opening) -> Event {
    opening
        .recv_timeout(Duration::from_secs(20))
        .expect("an event from the opening Session")
}

fn expect_opened(opening: &Opening) -> Opened {
    match next(opening) {
        Event::Opened(opened) => opened,
        Event::Failed(f) => panic!("expected the Session to open, it failed: {f}"),
        _ => panic!("expected the Session to open, something was asked"),
    }
}

/// Answers the password prompt with `password`, returning what it said.
fn enter_password(opening: &Opening, password: &str) -> (bool, Saved) {
    let Event::Password(prompt) = next(opening) else {
        panic!("expected the password prompt");
    };
    let said = (prompt.rejected, prompt.saved);
    prompt.enter(password.into());
    said
}

#[test]
fn a_remembered_password_opens_the_session_without_asking() {
    let setup = setup(password(), AuthMethod::Password);
    setup.keep(&password_key(), fixture::PASSWORD);
    let ssh = Ssh::new(|| {});
    let opened = expect_opened(&setup.open(&ssh));
    assert_eq!(opened.note, None);
}

#[test]
fn a_password_entered_is_kept_once_the_session_is_open() {
    let setup = setup(password(), AuthMethod::Password);
    let ssh = Ssh::new(|| {});
    let opening = setup.open(&ssh);
    assert_eq!(enter_password(&opening, "wrong"), (false, Saved::Nothing));
    assert_eq!(setup.store.secret(&password_key()), None);
    assert_eq!(
        enter_password(&opening, fixture::PASSWORD),
        (true, Saved::Nothing)
    );
    expect_opened(&opening);
    assert_eq!(
        setup.store.secret(&password_key()).as_deref(),
        Some(fixture::PASSWORD)
    );
    assert_eq!(setup.store.secret(&passphrase_key()), None);
}

#[test]
fn a_rejected_saved_password_asks_again_and_stays_until_a_new_one_works() {
    let setup = setup(password(), AuthMethod::Password);
    setup.keep(&password_key(), "old");
    let ssh = Ssh::new(|| {});
    let opening = setup.open(&ssh);
    assert_eq!(enter_password(&opening, "wrong"), (true, Saved::Rejected));
    assert_eq!(
        enter_password(&opening, fixture::PASSWORD),
        (true, Saved::Nothing)
    );
    // Not yet overwritten while it was only typed.
    let opened = expect_opened(&opening);
    assert_eq!(opened.note, None);
    assert_eq!(
        setup.store.secret(&password_key()).as_deref(),
        Some(fixture::PASSWORD)
    );
}

#[test]
fn cancelling_after_a_rejected_saved_password_keeps_it() {
    let setup = setup(password(), AuthMethod::Password);
    setup.keep(&password_key(), "old");
    let ssh = Ssh::new(|| {});
    let opening = setup.open(&ssh);
    let Event::Password(prompt) = next(&opening) else {
        panic!("expected the password prompt");
    };
    assert_eq!(prompt.saved, Saved::Rejected);
    prompt.cancel();
    assert!(matches!(next(&opening), Event::Failed(Failure::Cancelled)));
    assert_eq!(setup.store.secret(&password_key()).as_deref(), Some("old"));
}

#[test]
fn a_store_that_returns_nothing_asks_with_that_line() {
    let setup = setup(password(), AuthMethod::Password);
    setup.keep(&password_key(), fixture::PASSWORD);
    setup.store.fail_get(true);
    let ssh = Ssh::new(|| {});
    let opening = setup.open(&ssh);
    assert_eq!(
        enter_password(&opening, "wrong"),
        (false, Saved::Unavailable)
    );
    assert_eq!(
        enter_password(&opening, fixture::PASSWORD),
        (true, Saved::Nothing)
    );
    expect_opened(&opening);
}

#[test]
fn a_store_that_refuses_the_write_opens_the_session_with_a_note() {
    let setup = setup(password(), AuthMethod::Password);
    setup.store.fail_set(true);
    let ssh = Ssh::new(|| {});
    let opening = setup.open(&ssh);
    enter_password(&opening, fixture::PASSWORD);
    let opened = expect_opened(&opening);
    assert_eq!(opened.note.as_deref(), Some(credentials::NOT_KEPT));
}

#[test]
fn without_remember_nothing_is_read_or_kept() {
    let mut setup = setup(password(), AuthMethod::Password);
    setup.keep(&password_key(), fixture::PASSWORD);
    setup.options.remember = None;
    setup.store.fail_get(true);
    let ssh = Ssh::new(|| {});
    let opening = setup.open(&ssh);
    assert_eq!(enter_password(&opening, "typed"), (false, Saved::Nothing));
    assert_eq!(
        enter_password(&opening, fixture::PASSWORD),
        (true, Saved::Nothing)
    );
    expect_opened(&opening);
    assert_eq!(
        setup.store.secret(&password_key()).as_deref(),
        Some(fixture::PASSWORD)
    );
}

fn locked_key() -> Setup {
    let name = "openssh_ed25519_locked";
    let mut setup = setup(
        Accepts {
            keys: vec![public_key(name)],
            ..Accepts::default()
        },
        AuthMethod::KeyFile,
    );
    setup.options.key_files = vec![key_path(name)];
    setup
}

#[test]
fn a_remembered_passphrase_unlocks_the_key_without_asking() {
    let setup = locked_key();
    setup.keep(&passphrase_key(), PASSPHRASE);
    let ssh = Ssh::new(|| {});
    expect_opened(&setup.open(&ssh));
}

#[test]
fn a_rejected_saved_passphrase_asks_and_the_one_entered_is_kept() {
    let setup = locked_key();
    setup.keep(&passphrase_key(), "old");
    let ssh = Ssh::new(|| {});
    let opening = setup.open(&ssh);
    let Event::Passphrase(prompt) = next(&opening) else {
        panic!("expected the passphrase prompt");
    };
    assert_eq!((prompt.rejected, prompt.saved), (true, Saved::Rejected));
    prompt.enter(PASSPHRASE.into());
    expect_opened(&opening);
    assert_eq!(
        setup.store.secret(&passphrase_key()).as_deref(),
        Some(PASSPHRASE)
    );
    assert_eq!(setup.store.secret(&password_key()), None);
}

#[test]
fn an_unlocked_key_keeps_nothing() {
    let name = "openssh_ed25519";
    let mut setup = setup(
        Accepts {
            keys: vec![public_key(name)],
            ..Accepts::default()
        },
        AuthMethod::KeyFile,
    );
    setup.options.key_files = vec![key_path(name)];
    let ssh = Ssh::new(|| {});
    expect_opened(&setup.open(&ssh));
    assert_eq!(setup.store.secret(&password_key()), None);
    assert_eq!(setup.store.secret(&passphrase_key()), None);
}

#[test]
fn a_keyboard_interactive_password_prompt_is_remembered() {
    let accepts = Accepts {
        keyboard: Keyboard::Password,
        ..Accepts::default()
    };
    let setup = setup(accepts.clone(), AuthMethod::Password);
    let ssh = Ssh::new(|| {});
    let opening = setup.open(&ssh);
    enter_password(&opening, fixture::PASSWORD);
    expect_opened(&opening);
    assert_eq!(
        setup.store.secret(&password_key()).as_deref(),
        Some(fixture::PASSWORD)
    );
    // And used at the next open.
    expect_opened(&setup.open(&ssh));
}

#[test]
fn a_keyboard_interactive_round_of_several_prompts_is_not_remembered() {
    let setup = setup(
        Accepts {
            keyboard: Keyboard::PasswordAndCode,
            ..Accepts::default()
        },
        AuthMethod::Password,
    );
    let ssh = Ssh::new(|| {});
    let opening = setup.open(&ssh);
    let Event::Questions(prompt) = next(&opening) else {
        panic!("expected the questions");
    };
    prompt.answer(vec![fixture::PASSWORD.into(), fixture::CODE.into()]);
    expect_opened(&opening);
    assert_eq!(setup.store.secret(&password_key()), None);
}

#[test]
fn the_host_closing_after_its_own_attempt_limit_reconnects_quietly() {
    let setup = setup(
        Accepts {
            password: true,
            max_tries: Some(2),
            ..Accepts::default()
        },
        AuthMethod::Password,
    );
    setup.keep(&password_key(), "old");
    let ssh = Ssh::new(|| {});
    let opening = setup.open(&ssh);
    assert_eq!(enter_password(&opening, "wrong 1"), (true, Saved::Rejected));
    assert_eq!(enter_password(&opening, "wrong 2"), (true, Saved::Nothing));
    assert_eq!(enter_password(&opening, "wrong 3"), (true, Saved::Nothing));
    assert_eq!(
        enter_password(&opening, fixture::PASSWORD),
        (true, Saved::Nothing)
    );
    expect_opened(&opening);
    assert_eq!(
        setup.store.secret(&password_key()).as_deref(),
        Some(fixture::PASSWORD)
    );
}

#[test]
fn a_connection_failure_is_not_a_rejection() {
    let setup = setup(password(), AuthMethod::Password);
    setup.keep(&password_key(), "old");
    let port = setup.fixture.port();
    let options = setup.options.clone();
    let store = setup.store.clone();
    drop(setup);
    let ssh = Ssh::new(|| {});
    let target = Target {
        address: "127.0.0.1".into(),
        port,
        account: fixture::ACCOUNT.into(),
    };
    let opening = ssh.connect(target, options, "");
    assert!(matches!(
        next(&opening),
        Event::Failed(Failure::Connection(_))
    ));
    assert_eq!(store.secret(&password_key()).as_deref(), Some("old"));
}
