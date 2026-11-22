//! Search against the test server (tests/common/fixture.rs), whose exec runs the
//! command under the local `sh` and `find`: what matches, the kinds, following symlinked
//! directories, unreadable directories, the cap, stopping, and a host that cannot run it.

mod common;

use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::time::Duration;

use common::fixture::{self, Fixture};
use sshexplorer::search::{self, Ended, Event, How, Match, Query};
use sshexplorer::ssh::{Event as Opening, Ssh, Target};

const WAIT: Duration = Duration::from_secs(10);

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

    fn file(&self, path: &str) {
        let at = self.remote(path);
        std::fs::create_dir_all(at.parent().unwrap()).unwrap();
        std::fs::write(at, "x").unwrap();
    }

    fn dir(&self, path: &str) {
        std::fs::create_dir_all(self.remote(path)).unwrap();
    }

    fn link(&self, path: &str, target: &str) {
        std::os::unix::fs::symlink(target, self.remote(path)).unwrap();
    }

    fn query(&self, text: &str) -> Query {
        Query {
            start: fixture::HOME.to_string(),
            text: text.to_string(),
            follow: false,
            cap: 1000,
        }
    }

    /// Runs `query` to its end: every Match, sorted by path, and how it ended.
    fn run(&self, query: Query) -> (Vec<Match>, Ended) {
        let running = search::start(&self.session, query);
        collect(&running)
    }
}

fn collect(running: &search::Search) -> (Vec<Match>, Ended) {
    let mut matches = Vec::new();
    loop {
        match running.recv_timeout(WAIT).expect("the Search goes on") {
            Event::Found(batch) => matches.extend(batch),
            Event::Ended(ended) => {
                matches.sort_by(|a, b| a.path.cmp(&b.path));
                return (matches, ended);
            }
        }
    }
}

fn file(path: &str) -> Match {
    Match {
        path: path.to_string(),
        is_dir: false,
        link: false,
    }
}

fn dir(path: &str) -> Match {
    Match {
        path: path.to_string(),
        is_dir: true,
        link: false,
    }
}

fn dir_link(path: &str) -> Match {
    Match {
        path: path.to_string(),
        is_dir: true,
        link: true,
    }
}

const FINISHED: Ended = Ended {
    how: How::Finished,
    unreadable: 0,
};

#[test]
fn a_substring_matches_names_at_and_below_the_start_whatever_their_case() {
    let s = Setup::new();
    s.file("/home/tester/docs/Report.TXT");
    s.dir("/home/tester/photos/reports");
    s.file("/home/tester/other.md");
    s.file("/home/elsewhere/report.txt");

    let (matches, ended) = s.run(s.query("report"));

    assert_eq!(
        matches,
        vec![file("docs/Report.TXT"), dir("photos/reports")]
    );
    assert_eq!(ended, FINISHED);
}

#[test]
fn dot_names_match_like_any_other() {
    let s = Setup::new();
    s.file("/home/tester/docs/.hidden-note");

    let (matches, _) = s.run(s.query("note"));

    assert_eq!(matches, vec![file("docs/.hidden-note"), file("notes.txt")]);
}

#[test]
fn text_with_a_star_or_question_mark_is_a_glob_over_the_whole_name() {
    let s = Setup::new();
    s.file("/home/tester/docs/a.md");
    s.file("/home/tester/docs/a.md.bak");

    let (stars, _) = s.run(s.query("*.MD"));
    let (marks, _) = s.run(s.query("?.md"));

    assert_eq!(stars, vec![file("docs/a.md"), file("readme.md")]);
    assert_eq!(marks, vec![file("docs/a.md")]);
}

#[test]
fn brackets_and_quotes_in_plain_text_are_taken_literally() {
    let s = Setup::new();
    s.file("/home/tester/docs/it's [draft] $HOME.txt");
    s.file("/home/tester/docs/d.txt");

    let (matches, _) = s.run(s.query("'s [draft] $HOME"));

    assert_eq!(matches, vec![file("docs/it's [draft] $HOME.txt")]);
}

#[test]
fn symlinked_directories_are_not_followed_by_default_but_a_matching_link_is_a_match() {
    let s = Setup::new();
    s.file("/home/data/inner-match.txt");
    s.link("/home/tester/match-link", "../data");

    let (matches, ended) = s.run(s.query("match"));

    assert_eq!(matches, vec![dir_link("match-link")]);
    assert_eq!(ended, FINISHED);
}

#[test]
fn following_walks_each_link_target_once_and_keeps_matching_links() {
    let s = Setup::new();
    s.file("/home/data/inner-match.txt");
    s.link("/home/tester/match-link", "../data");
    s.link("/home/tester/docs/again", "../../data");
    // A link back up to the start must not search the start twice.
    s.link("/home/tester/docs/up-match", "..");
    s.file("/home/tester/match-here.txt");

    let mut query = s.query("match");
    query.follow = true;
    let (matches, ended) = s.run(query);

    assert!(matches.contains(&dir_link("match-link")), "{matches:?}");
    assert!(matches.contains(&dir_link("docs/up-match")), "{matches:?}");
    let paths: Vec<&str> = matches.iter().map(|m| m.path.as_str()).collect();
    assert!(paths.contains(&"match-link"), "{paths:?}");
    assert!(paths.contains(&"docs/up-match"), "{paths:?}");
    assert!(paths.contains(&"match-here.txt"), "{paths:?}");
    let inner = paths.iter().filter(|p| p.ends_with("inner-match.txt"));
    assert_eq!(inner.count(), 1, "the target is searched once: {paths:?}");
    assert_eq!(
        paths
            .iter()
            .filter(|p| p.ends_with("match-here.txt"))
            .count(),
        1,
        "the start is searched once: {paths:?}"
    );
    assert_eq!(ended, FINISHED);
}

#[test]
fn unreadable_directories_are_skipped_and_counted() {
    let s = Setup::new();
    s.file("/home/tester/locked/a/hit.txt");
    s.file("/home/tester/shut/hit.txt");
    s.file("/home/tester/docs/hit.txt");
    for dir in ["locked", "shut"] {
        let at = s.remote(&format!("/home/tester/{dir}"));
        std::fs::set_permissions(&at, std::fs::Permissions::from_mode(0o000)).unwrap();
    }

    let (matches, ended) = s.run(s.query("hit"));

    for dir in ["locked", "shut"] {
        let at = s.remote(&format!("/home/tester/{dir}"));
        std::fs::set_permissions(&at, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    assert_eq!(matches, vec![file("docs/hit.txt")]);
    assert_eq!(
        ended,
        Ended {
            how: How::Finished,
            unreadable: 2
        }
    );
}

#[test]
fn the_search_ends_at_the_match_cap() {
    let s = Setup::new();
    for i in 0..10 {
        s.file(&format!("/home/tester/docs/many-{i}"));
    }
    let mut query = s.query("many");
    query.cap = 4;

    let (matches, ended) = s.run(query);

    assert_eq!(matches.len(), 4);
    assert_eq!(ended.how, How::Capped);
}

#[test]
fn stop_keeps_what_was_found_and_kills_the_find_on_the_host() {
    let s = Setup::new();
    s.fixture.hold_search();
    let running = search::start(&s.session, s.query("notes"));
    let Some(Event::Found(found)) = running.recv_timeout(WAIT) else {
        panic!("expected the first Matches");
    };
    assert_eq!(found, vec![file("notes.txt")]);
    assert_eq!(running.recv_timeout(Duration::from_millis(300)), None);

    running.stop();

    let (_, ended) = collect(&running);
    assert_eq!(ended.how, How::Stopped);
}

#[test]
fn a_host_that_cannot_run_commands_cannot_search() {
    let s = Setup::new();
    s.fixture.refuse_search();

    let (matches, ended) = s.run(s.query("notes"));

    assert!(matches.is_empty());
    assert_eq!(ended.how, How::Unavailable);
}

#[test]
fn the_pattern_is_a_substring_unless_it_holds_glob_characters() {
    assert_eq!(search::pattern("abc"), "*abc*");
    assert_eq!(search::pattern("a[b]"), "*a\\[b\\]*");
    assert_eq!(search::pattern("a*b"), "a*b");
    assert_eq!(search::pattern("a?"), "a?");
}
