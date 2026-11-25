//! Tree view's model apart from egui: the tree rooted at `/`, fetching a directory the
//! first time it is expanded, the Focused entry and Selection, and the Tree keys.

use sshexplorer::folder::{Command, Sort};
use sshexplorer::ssh::{Entry, Listing};
use sshexplorer::tree::{Effect, Line, Tree};

fn file(name: &str) -> Entry {
    Entry {
        name: name.into(),
        size: Some(1),
        ..Entry::default()
    }
}

fn dir(name: &str) -> Entry {
    Entry {
        name: name.into(),
        is_dir: true,
        ..Entry::default()
    }
}

fn listing(path: &str, entries: Vec<Entry>) -> Listing {
    Listing {
        path: path.into(),
        entries,
    }
}

/// Each row as `depth` spaces and its text: an entry's name, "Loading…", or the reason.
fn rows(tree: &Tree) -> Vec<String> {
    (0..tree.len())
        .map(|i| {
            let row = tree.row(i);
            let text = match row.line {
                Line::Entry { name, .. } => name.to_owned(),
                Line::Loading => "Loading…".to_owned(),
                Line::Failed(reason) => format!("Cannot read: {reason}"),
            };
            format!("{}{text}", " ".repeat(row.depth))
        })
        .collect()
}

fn focused(tree: &Tree) -> Option<&str> {
    tree.focused_path()
}

/// A tree opened in `/home/tester`, with `/` and `/home` listed.
fn opened() -> Tree {
    let mut tree = Tree::new(Sort::default(), false);
    tree.open_at(listing(
        "/home/tester",
        vec![
            file("notes.txt"),
            dir("docs"),
            file(".profile"),
            dir("photos"),
        ],
    ));
    tree.listed(
        "/",
        Ok(listing("/", vec![dir("home"), dir("etc"), file("vmlinuz")])),
    );
    tree.listed(
        "/home",
        Ok(listing("/home", vec![dir("tester"), dir("other")])),
    );
    tree
}

#[test]
fn opening_fetches_the_ancestors_and_shows_loading_until_they_arrive() {
    let mut tree = Tree::new(Sort::default(), false);
    let fetch = tree.open_at(listing("/home/tester", vec![file("notes.txt")]));
    assert_eq!(fetch, ["/", "/home"]);
    assert_eq!(rows(&tree), ["/", " Loading…"]);
    // Nothing is focused yet, so the Current directory is the Initial remote folder.
    assert_eq!(focused(&tree), None);
    assert_eq!(tree.current_dir(), "/home/tester");

    tree.listed("/", Ok(listing("/", vec![dir("home"), file("vmlinuz")])));
    assert_eq!(rows(&tree), ["/", " home", "  Loading…", " vmlinuz"]);
    tree.listed("/home", Ok(listing("/home", vec![dir("tester")])));
    // The Initial remote folder is focused and selected, and is not expanded itself.
    assert_eq!(rows(&tree), ["/", " home", "  tester", " vmlinuz"]);
    assert_eq!(focused(&tree), Some("/home/tester"));
    assert_eq!(tree.selection(), ["/home/tester"]);
    assert!(tree.take_reveal());
    assert!(!tree.take_reveal());
}

#[test]
fn directories_come_first_and_hidden_entries_are_left_out() {
    let mut tree = opened();
    assert_eq!(
        tree.apply(Command::Expand, 0.0),
        Effect::None,
        "the Initial remote folder is already fetched"
    );
    assert_eq!(
        rows(&tree),
        [
            "/",
            " etc",
            " home",
            "  other",
            "  tester",
            "   docs",
            "   photos",
            "   notes.txt",
            " vmlinuz"
        ]
    );
    tree.arrange(Sort::default(), true);
    assert!(rows(&tree).contains(&"   .profile".to_owned()));
}

#[test]
fn right_expands_then_moves_to_the_first_child_fetching_once() {
    let mut tree = opened();
    tree.apply(Command::Expand, 0.0);
    tree.apply(Command::Down, 0.0);
    assert_eq!(focused(&tree), Some("/home/tester/docs"));
    assert_eq!(
        tree.apply(Command::Expand, 0.0),
        Effect::Fetch("/home/tester/docs".into())
    );
    assert!(rows(&tree).contains(&"    Loading…".to_owned()));
    // While loading there is no child to move to.
    assert_eq!(tree.apply(Command::Expand, 0.0), Effect::None);
    assert_eq!(focused(&tree), Some("/home/tester/docs"));
    tree.listed(
        "/home/tester/docs",
        Ok(listing("/home/tester/docs", vec![file("plan.txt")])),
    );
    tree.apply(Command::Expand, 0.0);
    assert_eq!(focused(&tree), Some("/home/tester/docs/plan.txt"));
    // The Current directory follows the Focused entry: a file's parent.
    assert_eq!(tree.current_dir(), "/home/tester/docs");

    // Left on a file goes to the parent; Left again collapses; Right reuses the fetch.
    tree.apply(Command::Collapse, 0.0);
    assert_eq!(focused(&tree), Some("/home/tester/docs"));
    assert_eq!(tree.current_dir(), "/home/tester/docs");
    tree.apply(Command::Collapse, 0.0);
    assert!(!rows(&tree).contains(&"    plan.txt".to_owned()));
    assert_eq!(tree.apply(Command::Expand, 0.0), Effect::None);
    assert!(rows(&tree).contains(&"    plan.txt".to_owned()));
}

#[test]
fn left_moves_up_past_the_initial_remote_folder_to_the_root() {
    let mut tree = opened();
    tree.apply(Command::Collapse, 0.0);
    assert_eq!(focused(&tree), Some("/home"));
    tree.apply(Command::Collapse, 0.0);
    assert_eq!(rows(&tree), ["/", " etc", " home", " vmlinuz"]);
    tree.apply(Command::Collapse, 0.0);
    assert_eq!(focused(&tree), Some("/"));
    assert_eq!(tree.current_dir(), "/");
    tree.apply(Command::Collapse, 0.0);
    assert_eq!(rows(&tree), ["/"]);
    // The root is never selected.
    assert!(tree.selection().is_empty());
}

#[test]
fn enter_toggles_a_directory_and_asks_for_preview_on_a_file() {
    let mut tree = opened();
    assert_eq!(tree.apply(Command::Open, 0.0), Effect::None);
    assert!(rows(&tree).contains(&"   notes.txt".to_owned()));
    tree.apply(Command::Open, 0.0);
    assert!(!rows(&tree).contains(&"   notes.txt".to_owned()));
    tree.apply(Command::Up, 0.0);
    tree.apply(Command::Up, 0.0);
    assert_eq!(focused(&tree), Some("/home"));
    tree.apply(Command::End, 0.0);
    assert_eq!(focused(&tree), Some("/vmlinuz"));
    assert_eq!(tree.apply(Command::Open, 0.0), Effect::Preview);
    // Backspace moves to the parent.
    tree.apply(Command::Parent, 0.0);
    assert_eq!(focused(&tree), Some("/"));
}

#[test]
fn a_failed_fetch_shows_one_row_with_the_reason() {
    let mut tree = opened();
    tree.apply(Command::Up, 0.0);
    tree.apply(Command::Up, 0.0);
    tree.apply(Command::Up, 0.0);
    assert_eq!(focused(&tree), Some("/etc"));
    assert_eq!(
        tree.apply(Command::Expand, 0.0),
        Effect::Fetch("/etc".into())
    );
    tree.listed("/etc", Err("permission denied".into()));
    assert_eq!(
        &rows(&tree)[..3],
        ["/", " etc", "  Cannot read: permission denied"]
    );
    // The reason row is passed over by the keys.
    tree.apply(Command::Down, 0.0);
    assert_eq!(focused(&tree), Some("/home"));
}

#[test]
fn the_selection_keeps_to_one_parent() {
    let mut tree = opened();
    tree.apply(Command::Expand, 0.0);
    tree.apply(Command::Down, 0.0);
    tree.apply(Command::ExtendDown, 0.0);
    assert_eq!(
        tree.selection(),
        ["/home/tester/docs", "/home/tester/photos"]
    );
    // Extending onto another parent's entry starts a new Selection.
    tree.apply(Command::ExtendDown, 0.0);
    tree.apply(Command::ExtendDown, 0.0);
    assert_eq!(focused(&tree), Some("/vmlinuz"));
    assert_eq!(tree.selection(), ["/vmlinuz"]);
    // Ctrl+A selects the visible entries beside the Focused one.
    tree.apply(Command::SelectAll, 0.0);
    assert_eq!(tree.selection(), ["/etc", "/home", "/vmlinuz"]);
    // Ctrl+click under another parent replaces the Selection.
    let notes = (0..tree.len())
        .find(|i| tree.path(*i) == Some("/home/tester/notes.txt"))
        .unwrap();
    tree.apply(Command::CtrlClick(notes), 0.0);
    assert_eq!(tree.selection(), ["/home/tester/notes.txt"]);
}

#[test]
fn reveal_expands_to_an_entry_and_focuses_it() {
    let mut tree = opened();
    let fetch = tree.reveal("/home/tester/docs", Some("plan.txt"));
    assert_eq!(fetch, ["/home/tester/docs"]);
    tree.listed(
        "/home/tester/docs",
        Ok(listing("/home/tester/docs", vec![file("plan.txt")])),
    );
    assert_eq!(focused(&tree), Some("/home/tester/docs/plan.txt"));
    assert!(tree.take_reveal());
    // A directory itself, with nothing to fetch.
    assert!(tree.reveal("/home", None).is_empty());
    assert_eq!(focused(&tree), Some("/home"));
}

#[test]
fn typing_jumps_to_a_visible_name() {
    let mut tree = opened();
    tree.apply(Command::Type('v'), 0.0);
    assert_eq!(focused(&tree), Some("/vmlinuz"));
    tree.apply(Command::Type('o'), 5.0);
    assert_eq!(focused(&tree), Some("/home/other"));
}

#[test]
#[ignore = "local performance measurement"]
fn refresh_ten_thousand_directories() {
    let entries = (0..10_000).map(|n| dir(&format!("dir-{n:05}"))).collect();
    let original = listing("/", entries);
    let mut tree = Tree::new(Sort::default(), false);
    tree.open_at(original.clone());
    for _ in 0..3 {
        let listing = original.clone();
        let start = std::time::Instant::now();
        tree.refreshed("/", Ok(listing));
        eprintln!("10,000-directory refresh: {:?}", start.elapsed());
    }
}
