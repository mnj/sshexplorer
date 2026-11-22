//! Authentication methods against the test server (tests/common/fixture.rs): key files,
//! the SSH agent, keyboard-interactive and the password, and the order Automatic tries
//! them in.
//!
//! The keys are tests/keys (see tests/common/keys.rs). The agent is russh's own agent
//! server on a socket in a temp directory, never the operator's.

mod common;

use std::time::Duration;

use common::fixture::{self, Accepts, Fixture, Keyboard};
#[cfg(unix)]
use common::keys::TestAgent;
use common::keys::{PASSPHRASE, key_path, public_key};
use sshexplorer::hosts::AuthMethod;
use sshexplorer::ssh::{Event, Failure, Opening, Options, Ssh, Target};

struct Setup {
    fixture: Fixture,
    _dir: tempfile::TempDir,
    options: Options,
}

/// A fixture accepting `accepts`, whose host key is already known, and Options using
/// `auth` with no key files and no agent.
fn setup(accepts: Accepts, auth: AuthMethod) -> Setup {
    let fixture = Fixture::start_accepting(accepts);
    let dir = tempfile::tempdir().unwrap();
    let known_hosts = dir.path().join("known_hosts");
    std::fs::write(&known_hosts, format!("{}\n", fixture.known_hosts_line())).unwrap();
    let mut options = Options::new(known_hosts);
    options.auth = auth;
    Setup {
        fixture,
        _dir: dir,
        options,
    }
}

fn keys(names: &[&str]) -> Accepts {
    Accepts {
        keys: names.iter().map(|n| public_key(n)).collect(),
        ..Accepts::default()
    }
}

fn password() -> Accepts {
    Accepts {
        password: true,
        ..Accepts::default()
    }
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
}

fn next(opening: &Opening) -> Event {
    opening
        .recv_timeout(Duration::from_secs(20))
        .expect("an event from the opening Session")
}

fn expect_opened(opening: &Opening) {
    match next(opening) {
        Event::Opened(_) => {}
        Event::Failed(f) => panic!("expected the Session to open, it failed: {f}"),
        _ => panic!("expected the Session to open, something was asked"),
    }
}

fn expect_failed(opening: &Opening) -> Failure {
    match next(opening) {
        Event::Failed(f) => f,
        Event::Opened(_) => panic!("expected the Session to fail, it opened"),
        _ => panic!("expected the Session to fail, something was asked"),
    }
}

fn ssh() -> Ssh {
    Ssh::new(|| {})
}

#[test]
fn every_unlocked_key_format_opens_a_session_by_key_file() {
    for name in [
        "openssh_ed25519",
        "openssh_ecdsa",
        "openssh_rsa",
        "pem_rsa",
        "pem_ecdsa",
        "pkcs8_rsa",
        "pkcs8_ecdsa",
        "pkcs8_ed25519",
    ] {
        let mut setup = setup(keys(&[name]), AuthMethod::KeyFile);
        setup.options.key_files = vec![key_path(name)];
        let ssh = ssh();
        let opening = setup.open(&ssh);
        match next(&opening) {
            Event::Opened(_) => {}
            Event::Failed(f) => panic!("{name}: {f}"),
            _ => panic!("{name}: something was asked"),
        }
    }
}

/// Answers each passphrase prompt with the next of `passphrases`, and returns whether
/// each prompt said the one before was rejected.
fn enter_passphrases(opening: &Opening, passphrases: &[&str]) -> Vec<bool> {
    let mut rejected = Vec::new();
    for passphrase in passphrases {
        let Event::Passphrase(prompt) = next(opening) else {
            panic!("expected the passphrase prompt");
        };
        rejected.push(prompt.rejected);
        prompt.enter(passphrase.to_string());
    }
    rejected
}

#[test]
fn every_locked_key_format_asks_for_its_passphrase_until_it_unlocks() {
    for name in [
        "openssh_ed25519_locked",
        "openssh_ecdsa_locked",
        "openssh_rsa_locked",
        "pem_rsa_locked",
        "pem_ecdsa_locked",
        "pkcs8_rsa_locked",
        "pkcs8_ecdsa_locked",
        "pkcs8_ed25519_locked",
    ] {
        let mut setup = setup(keys(&[name]), AuthMethod::KeyFile);
        setup.options.key_files = vec![key_path(name)];
        let ssh = ssh();
        let opening = setup.open(&ssh);
        let rejected = enter_passphrases(&opening, &["wrong", PASSPHRASE]);
        assert_eq!(rejected, [false, true], "{name}");
        match next(&opening) {
            Event::Opened(_) => {}
            Event::Failed(f) => panic!("{name}: {f}"),
            _ => panic!("{name}: something else was asked"),
        }
    }
}

#[test]
fn cancelling_the_passphrase_opens_nothing() {
    let mut setup = setup(keys(&["openssh_ed25519_locked"]), AuthMethod::KeyFile);
    setup.options.key_files = vec![key_path("openssh_ed25519_locked")];
    let ssh = ssh();
    let opening = setup.open(&ssh);
    let Event::Passphrase(prompt) = next(&opening) else {
        panic!("expected the passphrase prompt");
    };
    assert_eq!(prompt.key, key_path("openssh_ed25519_locked"));
    drop(prompt);
    assert_eq!(expect_failed(&opening), Failure::Cancelled);
}

#[test]
fn the_key_file_method_never_falls_back_to_a_password() {
    let mut accepts = keys(&["openssh_ecdsa"]);
    accepts.password = true;
    let mut setup = setup(accepts, AuthMethod::KeyFile);
    setup.options.key_files = vec![key_path("openssh_ed25519")];
    let ssh = ssh();
    let failure = expect_failed(&setup.open(&ssh));
    assert_eq!(failure.to_string(), "The host did not accept the key file.");
    assert!(!setup.fixture.attempts().contains(&"password".to_string()));
}

#[test]
fn the_key_file_method_without_a_key_file_says_so() {
    let setup = setup(password(), AuthMethod::KeyFile);
    let ssh = ssh();
    let failure = expect_failed(&setup.open(&ssh));
    assert_eq!(failure.to_string(), "No key file is set for this Host.");
}

#[test]
fn the_password_method_offers_no_key() {
    let mut accepts = keys(&["openssh_ed25519"]);
    accepts.password = true;
    let mut setup = setup(accepts, AuthMethod::Password);
    setup.options.key_files = vec![key_path("openssh_ed25519")];
    let ssh = ssh();
    let opening = setup.open(&ssh);
    let Event::Password(prompt) = next(&opening) else {
        panic!("expected the password prompt");
    };
    prompt.enter(fixture::PASSWORD.into());
    expect_opened(&opening);
    assert_eq!(setup.fixture.attempts(), ["password"]);
}

#[test]
fn automatic_falls_back_from_a_refused_key_to_the_password() {
    let mut accepts = keys(&["openssh_ecdsa"]);
    accepts.password = true;
    let mut setup = setup(accepts, AuthMethod::Automatic);
    setup.options.key_files = vec![key_path("openssh_ed25519")];
    let ssh = ssh();
    let opening = setup.open(&ssh);
    let Event::Password(prompt) = next(&opening) else {
        panic!("expected the password prompt");
    };
    prompt.enter(fixture::PASSWORD.into());
    expect_opened(&opening);
    let attempts = setup.fixture.attempts();
    assert!(attempts[0].starts_with("publickey "), "{attempts:?}");
    assert_eq!(attempts.last().unwrap(), "password");
}

#[cfg(unix)]
#[test]
fn automatic_uses_the_agent_for_a_locked_key_file_it_holds() {
    let agent = TestAgent::holding_keys(&["openssh_ed25519_locked"]);
    let mut setup = setup(keys(&["openssh_ed25519_locked"]), AuthMethod::Automatic);
    setup.options.key_files = vec![key_path("openssh_ed25519_locked")];
    setup.options.agent = Some(agent.socket());
    let ssh = ssh();
    // No passphrase is asked for: the agent signs.
    expect_opened(&setup.open(&ssh));
}

#[cfg(unix)]
#[test]
fn automatic_tries_the_key_file_then_the_agents_other_keys_then_the_password() {
    let agent = TestAgent::holding_keys(&["openssh_ecdsa", "openssh_rsa"]);
    let accepts = Accepts {
        password: true,
        ..Accepts::default()
    };
    // Authorize a key no one offers, so the publickey method is on offer.
    let accepts = Accepts {
        keys: vec![public_key("pkcs8_ecdsa")],
        ..accepts
    };
    let mut setup = setup(accepts, AuthMethod::Automatic);
    setup.options.key_files = vec![key_path("openssh_ed25519")];
    setup.options.agent = Some(agent.socket());
    let ssh = ssh();
    let opening = setup.open(&ssh);
    let Event::Password(prompt) = next(&opening) else {
        panic!("expected the password prompt");
    };
    prompt.enter(fixture::PASSWORD.into());
    expect_opened(&opening);
    let fp = |name: &str| {
        format!(
            "publickey {}",
            public_key(name).fingerprint(Default::default())
        )
    };
    let mut attempts = setup.fixture.attempts();
    assert_eq!(attempts.first(), Some(&fp("openssh_ed25519")));
    assert_eq!(attempts.last().map(String::as_str), Some("password"));
    // The agent lists its keys in an order of its own.
    let mut agent_keys = attempts.drain(1..attempts.len() - 1).collect::<Vec<_>>();
    agent_keys.sort();
    let mut expected = vec![fp("openssh_ecdsa"), fp("openssh_rsa")];
    expected.sort();
    assert_eq!(agent_keys, expected);
}

#[cfg(unix)]
#[test]
fn identities_only_keeps_the_agent_to_the_key_files() {
    let agent = TestAgent::holding_keys(&["openssh_ecdsa"]);
    let mut accepts = keys(&["openssh_ecdsa"]);
    accepts.password = true;
    let mut setup = setup(accepts, AuthMethod::Automatic);
    setup.options.agent = Some(agent.socket());
    setup.options.identities_only = true;
    let ssh = ssh();
    let opening = setup.open(&ssh);
    let Event::Password(prompt) = next(&opening) else {
        panic!("expected the password prompt: the agent's key is not a key file");
    };
    prompt.enter(fixture::PASSWORD.into());
    expect_opened(&opening);
    assert_eq!(setup.fixture.attempts(), ["password"]);
}

#[cfg(unix)]
#[test]
fn the_agent_method_logs_in_with_an_agent_key_and_nothing_else() {
    let agent = TestAgent::holding_keys(&["openssh_rsa"]);
    let mut accepts = keys(&["openssh_rsa"]);
    accepts.password = true;
    let mut setup = setup(accepts, AuthMethod::Agent);
    setup.options.agent = Some(agent.socket());
    let ssh = ssh();
    expect_opened(&setup.open(&ssh));

    // An agent whose keys the host refuses fails without asking for a password.
    let other = TestAgent::holding_keys(&["openssh_ecdsa"]);
    setup.options.agent = Some(other.socket());
    let failure = expect_failed(&setup.open(&ssh));
    assert_eq!(
        failure.to_string(),
        "The host accepted none of the agent's keys."
    );
}

#[test]
fn the_agent_method_without_an_agent_fails() {
    let mut setup = setup(password(), AuthMethod::Agent);
    let ssh = ssh();
    let failure = expect_failed(&setup.open(&ssh));
    assert_eq!(failure.to_string(), "No SSH agent is running.");

    // A socket nobody listens on is no agent either.
    let dir = tempfile::tempdir().unwrap();
    setup.options.agent = Some(dir.path().join("gone.sock"));
    let failure = expect_failed(&setup.open(&ssh));
    assert_eq!(failure.to_string(), "No SSH agent is running.");
}

#[test]
fn automatic_skips_an_unreachable_agent_quietly() {
    let dir = tempfile::tempdir().unwrap();
    let mut setup = setup(password(), AuthMethod::Automatic);
    setup.options.agent = Some(dir.path().join("gone.sock"));
    let ssh = ssh();
    let opening = setup.open(&ssh);
    let Event::Password(prompt) = next(&opening) else {
        panic!("expected the password prompt");
    };
    prompt.enter(fixture::PASSWORD.into());
    expect_opened(&opening);
}

#[test]
fn a_keyboard_interactive_round_with_one_hidden_prompt_is_the_password() {
    let setup = setup(
        Accepts {
            keyboard: Keyboard::Password,
            ..Accepts::default()
        },
        AuthMethod::Password,
    );
    let ssh = ssh();
    let opening = setup.open(&ssh);
    let Event::Password(prompt) = next(&opening) else {
        panic!("expected the password prompt");
    };
    assert!(!prompt.rejected);
    prompt.enter("wrong".into());
    let Event::Password(prompt) = next(&opening) else {
        panic!("expected the password prompt again");
    };
    assert!(prompt.rejected);
    prompt.enter(fixture::PASSWORD.into());
    expect_opened(&opening);
    assert!(
        setup
            .fixture
            .attempts()
            .iter()
            .all(|a| a == "keyboard-interactive")
    );
}

#[test]
fn a_keyboard_interactive_round_with_several_prompts_asks_each() {
    let setup = setup(
        Accepts {
            keyboard: Keyboard::PasswordAndCode,
            ..Accepts::default()
        },
        AuthMethod::Automatic,
    );
    let ssh = ssh();
    let opening = setup.open(&ssh);
    let Event::Questions(prompt) = next(&opening) else {
        panic!("expected the questions prompt");
    };
    let asked: Vec<(&str, bool)> = prompt
        .questions
        .iter()
        .map(|q| (q.prompt.as_str(), q.echo))
        .collect();
    assert_eq!(
        asked,
        [("Password: ", false), ("Verification code: ", true)]
    );
    assert!(!prompt.rejected);
    prompt.answer(vec![fixture::PASSWORD.into(), "000000".into()]);
    let Event::Questions(prompt) = next(&opening) else {
        panic!("expected the questions prompt again");
    };
    assert!(prompt.rejected);
    prompt.answer(vec![fixture::PASSWORD.into(), fixture::CODE.into()]);
    expect_opened(&opening);
}

#[test]
fn a_host_taking_no_password_fails_once_the_keys_are_refused() {
    let setup = setup(keys(&["openssh_ecdsa"]), AuthMethod::Automatic);
    let ssh = ssh();
    let failure = expect_failed(&setup.open(&ssh));
    assert_eq!(
        failure.to_string(),
        "The host accepted no key and does not take a password."
    );
}
