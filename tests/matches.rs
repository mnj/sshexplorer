//! The Matches as a list: focus, the Selection under one parent, opening, and a rerun
//! that restores the Focused and selected Matches by path.

use sshexplorer::folder::Command;
use sshexplorer::matches::{Effect, Matches};
use sshexplorer::search::{Ended, How, Match, Query};

fn query() -> Query {
    Query {
        start: "/home/tester".into(),
        text: "a".into(),
        follow: false,
        cap: 1000,
    }
}

fn m(path: &str, is_dir: bool) -> Match {
    Match {
        path: path.into(),
        is_dir,
        link: false,
    }
}

/// docs/a.txt, docs/b.txt, photos (dir), photos/a.png
fn four() -> Matches {
    let mut matches = Matches::new(query());
    matches.found(vec![m("docs/a.txt", false), m("docs/b.txt", false)]);
    matches.found(vec![m("photos", true), m("photos/a.png", false)]);
    matches
}

fn selected(matches: &Matches) -> Vec<&str> {
    (0..matches.len())
        .filter(|i| matches.is_selected(*i))
        .map(|i| matches.get(i).path.as_str())
        .collect()
}

const DONE: Ended = Ended {
    how: How::Finished,
    unreadable: 0,
};

#[test]
fn the_first_match_is_focused_and_nothing_is_selected_at_first() {
    let matches = four();
    assert_eq!(matches.focus(), Some(0));
    assert!(selected(&matches).is_empty());
    assert_eq!(Matches::new(query()).focus(), None);
}

#[test]
fn moving_focuses_and_selects_like_the_listing() {
    let mut matches = four();
    matches.apply(Command::Down);
    assert_eq!(matches.focus(), Some(1));
    assert_eq!(selected(&matches), vec!["docs/b.txt"]);
    matches.apply(Command::End);
    assert_eq!(matches.focus(), Some(3));
    matches.apply(Command::FocusUp);
    assert_eq!(matches.focus(), Some(2));
    assert_eq!(selected(&matches), vec!["photos/a.png"]);
}

#[test]
fn the_selection_keeps_to_one_parent() {
    let mut matches = four();
    matches.apply(Command::Home);
    matches.apply(Command::ExtendDown);
    matches.apply(Command::ExtendDown);
    // photos is under the start, not under docs: extending to it starts over there.
    assert_eq!(selected(&matches), vec!["photos"]);

    matches.apply(Command::Click(0));
    matches.apply(Command::CtrlClick(1));
    assert_eq!(selected(&matches), vec!["docs/a.txt", "docs/b.txt"]);
    matches.apply(Command::CtrlClick(3));
    assert_eq!(selected(&matches), vec!["photos/a.png"]);

    matches.apply(Command::Click(1));
    matches.apply(Command::SelectAll);
    assert_eq!(selected(&matches), vec!["docs/a.txt", "docs/b.txt"]);
    let (dir, entries) = matches.selection().unwrap();
    assert_eq!(dir, "/home/tester/docs");
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["a.txt", "b.txt"]);
}

#[test]
fn a_symlinked_directory_match_is_selected_as_a_link_so_download_can_skip_it() {
    let mut matches = Matches::new(query());
    let link = Match {
        path: "docs/linked".into(),
        is_dir: true,
        link: true,
    };
    matches.found(vec![link, m("docs/real", true)]);
    matches.apply(Command::Home);
    matches.apply(Command::SelectAll);

    let (_, entries) = matches.selection().unwrap();
    let links: Vec<(&str, bool)> = entries
        .iter()
        .map(|e| (e.name.as_str(), e.link.is_some()))
        .collect();
    assert_eq!(links, vec![("linked", true), ("real", false)]);
}

#[test]
fn enter_and_a_double_click_open_the_match() {
    let mut matches = four();
    matches.apply(Command::Down);
    assert_eq!(
        matches.apply(Command::Open),
        Effect::Open(m("docs/b.txt", false))
    );
    assert_eq!(
        matches.apply(Command::DoubleClick(2)),
        Effect::Open(m("photos", true))
    );
}

#[test]
fn a_rerun_restores_the_focused_and_selected_matches_that_still_match() {
    let mut matches = four();
    matches.apply(Command::Click(0));
    matches.apply(Command::CtrlClick(1));
    matches.end(DONE);

    let mut again = matches.rerun();
    assert!(again.is_empty());
    assert_eq!(again.ended(), None);
    again.found(vec![m("new.txt", false), m("docs/b.txt", false)]);
    again.end(DONE);

    assert_eq!(again.focus(), Some(1), "docs/b.txt is focused again");
    assert_eq!(selected(&again), vec!["docs/b.txt"]);
    assert_eq!(again.query(), &query());
}
