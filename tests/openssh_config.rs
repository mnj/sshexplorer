//! OpenSSH config resolution (ADR-0002): which `~/.ssh/config` values a Session's address
//! picks up, as `ssh <address>` would pick them, and how they combine with the Host's own
//! fields. Every test writes its config files into a fresh directory standing in for
//! `~/.ssh`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use sshexplorer::openssh_config::{Config, HostFields, IdentityAgent};
use sshexplorer::ssh::Target;

/// A home directory with a `.ssh` folder holding `files` (name, text).
struct Home {
    dir: tempfile::TempDir,
}

impl Home {
    fn with(files: &[(&str, &str)]) -> Home {
        let dir = tempfile::tempdir().unwrap();
        for (name, text) in files {
            let path = dir.path().join(".ssh").join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        Home { dir }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn ssh(&self, name: &str) -> PathBuf {
        self.path().join(".ssh").join(name)
    }

    fn config(&self) -> Config {
        Config::read(&self.ssh("config"), self.path())
    }
}

fn config(text: &str) -> (Home, Config) {
    let home = Home::with(&[("config", text)]);
    let config = home.config();
    (home, config)
}

#[test]
fn an_alias_picks_up_its_host_block() {
    let (_home, config) = config(
        "Host web\n\
         \x20   HostName 10.0.0.5\n\
         \x20   User deploy\n\
         \x20   Port 2222\n",
    );
    let web = config.lookup("web");
    assert_eq!(web.host_name.as_deref(), Some("10.0.0.5"));
    assert_eq!(web.user.as_deref(), Some("deploy"));
    assert_eq!(web.port, Some(2222));

    let other = config.lookup("db");
    assert_eq!(other.host_name, None);
    assert_eq!(other.user, None);
    assert_eq!(other.port, None);
}

#[test]
fn the_first_value_for_a_keyword_wins_across_every_matching_block() {
    let (_home, config) = config(
        "User early\n\
         Host web\n\
         \x20   Port 2222\n\
         \x20   User deploy\n\
         Host web*\n\
         \x20   Port 2200\n\
         \x20   HostName web.example.org\n\
         Host *\n\
         \x20   User fallback\n\
         \x20   Port 2000\n",
    );
    let web = config.lookup("web");
    assert_eq!(web.user.as_deref(), Some("early"));
    assert_eq!(web.port, Some(2222));
    assert_eq!(web.host_name.as_deref(), Some("web.example.org"));

    let other = config.lookup("db");
    assert_eq!(other.user.as_deref(), Some("early"));
    assert_eq!(other.port, Some(2000));
}

#[test]
fn patterns_take_wildcards_and_negation_and_ignore_case() {
    let (_home, config) = config(
        "Host *.example.org !secret.example.org\n\
         \x20   User staff\n\
         Host db?\n\
         \x20   Port 5000\n",
    );
    assert_eq!(
        config.lookup("WWW.Example.org").user.as_deref(),
        Some("staff")
    );
    assert_eq!(config.lookup("secret.example.org").user, None);
    assert_eq!(config.lookup("example.org").user, None);
    assert_eq!(config.lookup("db1").port, Some(5000));
    assert_eq!(config.lookup("db12").port, None);
}

#[test]
fn a_block_of_only_negated_patterns_matches_nothing() {
    let (_home, config) = config("Host !web\n    User nobody\n");
    assert_eq!(config.lookup("db").user, None);
}

#[test]
fn values_may_follow_an_equals_sign_and_be_quoted() {
    let (_home, config) = config(
        "# a comment\n\
         \n\
         Host=web\n\
         \x20   HostName = \"web host.example.org\"\n\
         \x20   User=deploy # trailing comment\n\
         \tPort\t=\t2222\n",
    );
    let web = config.lookup("web");
    assert_eq!(web.host_name.as_deref(), Some("web host.example.org"));
    assert_eq!(web.user.as_deref(), Some("deploy"));
    assert_eq!(web.port, Some(2222));
}

#[test]
fn match_blocks_are_skipped_up_to_the_next_host_line() {
    let (_home, config) = config(
        "Match host web\n\
         \x20   User matched\n\
         \x20   Port 1111\n\
         Host web\n\
         \x20   User deploy\n\
         Match all\n\
         \x20   Port 3333\n",
    );
    let web = config.lookup("web");
    assert_eq!(web.user.as_deref(), Some("deploy"));
    assert_eq!(web.port, None);
}

#[test]
fn include_reads_files_at_its_place_relative_to_the_ssh_folder_in_lexical_order() {
    let home = Home::with(&[
        (
            "config",
            "Host web\n\
             \x20   Include config.d/*\n\
             Host *\n\
             \x20   User fallback\n",
        ),
        ("config.d/20-port", "Port 2200\nUser second\n"),
        ("config.d/10-user", "User first\n"),
    ]);
    let config = home.config();
    let web = config.lookup("web");
    assert_eq!(web.user.as_deref(), Some("first"));
    assert_eq!(web.port, Some(2200));
    // The include sits in web's block, so other hosts never see it.
    let db = config.lookup("db");
    assert_eq!(db.user.as_deref(), Some("fallback"));
    assert_eq!(db.port, None);
}

#[test]
fn an_included_file_has_its_own_host_blocks_and_tilde_expands_to_home() {
    let home = Home::with(&[
        (
            "config",
            "Include ~/.ssh/extra missing-file\nHost *\n  Port 4000\n",
        ),
        ("extra", "Host db\n  Port 5432\n"),
    ]);
    let config = home.config();
    assert_eq!(config.lookup("db").port, Some(5432));
    // The included file ending inside `Host db` does not stop `Host *` applying after.
    assert_eq!(config.lookup("web").port, Some(4000));
    assert_eq!(config.error(), None);
}

#[test]
fn a_missing_config_is_an_empty_one() {
    let home = Home::with(&[]);
    let config = home.config();
    assert_eq!(config.error(), None);
    assert_eq!(config.lookup("web").user, None);
}

#[test]
fn an_unreadable_line_is_reported_with_its_file_and_line_and_nothing_applies() {
    let (home, config) = config("Host web\n  User deploy\n  Port many\n");
    let error = config.error().expect("Port many does not parse");
    assert_eq!(error.file, home.ssh("config"));
    assert_eq!(error.line, 3);
    assert_eq!(config.lookup("web").user, None);
    assert_eq!(
        error.note(home.path()),
        "~/.ssh/config line 3: bad Port \"many\". Opened from the Host fields alone."
    );

    let home = Home::with(&[
        ("config", "Include other\n"),
        ("other", "\n\nUser \"unclosed\n"),
    ]);
    let error = home.config().error().cloned().expect("an unclosed quote");
    assert_eq!(error.file, home.ssh("other"));
    assert_eq!(error.line, 3);
}

#[test]
fn an_include_loop_stops_with_an_error() {
    let home = Home::with(&[("config", "Include config\n")]);
    assert!(home.config().error().is_some());
}

fn fields(address: &str, port: Option<u16>, account: &str, key_file: &str) -> HostFields {
    HostFields {
        address: address.into(),
        port,
        account: account.into(),
        key_file: key_file.into(),
    }
}

fn target(address: &str, port: u16, account: &str) -> Target {
    Target {
        address: address.into(),
        port,
        account: account.into(),
    }
}

const WEB: &str = "Host web\n\
                   \x20   HostName %h.example.org\n\
                   \x20   User deploy\n\
                   \x20   Port 2222\n\
                   \x20   IdentityFile ~/.ssh/id_%r@%h\n\
                   \x20   IdentityFile /keys/%n-%p\n\
                   \x20   IdentitiesOnly yes\n\
                   \x20   IdentityAgent ~/agent.sock\n\
                   \x20   UserKnownHostsFile ~/.ssh/known_%h /etc/known\n\
                   \x20   ConnectTimeout 7\n\
                   \x20   ServerAliveInterval 15\n\
                   \x20   Compression yes\n\
                   \x20   StrictHostKeyChecking no\n";

#[test]
fn the_config_fills_whatever_the_host_leaves_empty() {
    let (home, config) = config(WEB);
    let resolved = config.resolve(&fields("web", None, "", ""), "me").unwrap();
    assert_eq!(resolved.target, target("web.example.org", 2222, "deploy"));
    assert_eq!(
        resolved.key_files,
        vec![
            home.ssh("id_deploy@web.example.org"),
            PathBuf::from("/keys/web-2222"),
        ]
    );
    assert!(resolved.identities_only);
    assert_eq!(
        resolved.identity_agent,
        IdentityAgent::Socket(home.path().join("agent.sock"))
    );
    assert_eq!(
        resolved.known_hosts,
        vec![
            home.ssh("known_web.example.org"),
            PathBuf::from("/etc/known")
        ]
    );
    assert_eq!(resolved.connect_timeout, Some(Duration::from_secs(7)));
    assert_eq!(
        resolved.server_alive_interval,
        Some(Duration::from_secs(15))
    );
}

#[test]
fn a_filled_host_field_wins_over_the_config() {
    let (home, config) = config(WEB);
    let resolved = config
        .resolve(&fields("web", Some(2200), "root", "~/my-key"), "me")
        .unwrap();
    assert_eq!(resolved.target, target("web.example.org", 2200, "root"));
    assert_eq!(resolved.key_files, vec![home.path().join("my-key")]);
}

#[test]
fn without_a_config_value_the_defaults_apply() {
    let (_home, config) = config(WEB);
    let resolved = config.resolve(&fields("db", None, "", ""), "me").unwrap();
    assert_eq!(resolved.target, target("db", 22, "me"));
    assert!(resolved.key_files.is_empty());
    assert!(!resolved.identities_only);
    assert_eq!(resolved.identity_agent, IdentityAgent::Default);
    assert!(resolved.known_hosts.is_empty());
    assert_eq!(resolved.connect_timeout, None);
    assert_eq!(resolved.server_alive_interval, None);
}

#[test]
fn an_unreadable_config_resolves_from_the_host_fields_alone() {
    let (_home, config) = config("Host web\n  User deploy\n  Port 0\n");
    let resolved = config.resolve(&fields("web", None, "", ""), "me").unwrap();
    assert_eq!(resolved.target, target("web", 22, "me"));
    assert!(config.note().unwrap().contains("line 3"));
}

#[test]
fn identity_agent_none_turns_the_agent_off() {
    let (_home, config) =
        config("Host a\n IdentityAgent none\nHost b\n IdentityAgent SSH_AUTH_SOCK\n");
    let agent = |address| {
        config
            .resolve(&fields(address, None, "", ""), "me")
            .unwrap()
            .identity_agent
    };
    assert_eq!(agent("a"), IdentityAgent::Off);
    assert_eq!(agent("b"), IdentityAgent::Default);
}

#[test]
fn a_proxy_fails_the_session_and_names_its_keyword() {
    let (_home, config) = config(
        "Host jump\n  ProxyJump bastion\n\
         Host command\n  ProxyCommand ssh -W %h:%p bastion\n\
         Host direct\n  ProxyJump none\n  ProxyCommand none\n",
    );
    let resolve = |address| config.resolve(&fields(address, None, "", ""), "me");
    assert_eq!(
        resolve("jump").unwrap_err().to_string(),
        "This host needs ProxyJump, which sshexplorer does not support yet."
    );
    assert_eq!(
        resolve("command").unwrap_err().to_string(),
        "This host needs ProxyCommand, which sshexplorer does not support yet."
    );
    assert_eq!(
        resolve("direct").unwrap().target,
        target("direct", 22, "me")
    );
}
