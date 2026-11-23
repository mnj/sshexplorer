//! Refresh apart from egui: what a Refresh fetches again, how the new listings merge into
//! Tree view and Folder view, and when Auto-refresh ticks.

use sshexplorer::folder::{Command, Folder, Sort};
use sshexplorer::refresh::Timer;
use sshexplorer::settings::AutoRefresh;
use sshexplorer::ssh::{Entry, Listing, NO_SUCH_FILE};
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

fn row_of(tree: &Tree, path: &str) -> usize {
    (0..tree.len())
        .find(|i| tree.path(*i) == Some(path))
        .unwrap_or_else(|| panic!("no row {path}"))
}

fn home() -> Vec<Entry> {
    vec![
        file("notes.txt"),
        dir("docs"),
        dir("photos"),
        file("todo.txt"),
    ]
}

/// A tree opened in `/home/tester` with `/`, `/home` and `docs` listed, and the home
/// directory and `docs` expanded; `notes.txt` focused.
fn opened() -> Tree {
    let mut tree = Tree::new(Sort::default(), false);
    tree.open_at(listing("/home/tester", home()));
    tree.listed("/", Ok(listing("/", vec![dir("home"), dir("etc")])));
    tree.listed("/home", Ok(listing("/home", vec![dir("tester")])));
    tree.apply(Command::Expand, 0.0);
    tree.apply(Command::Down, 0.0);
    tree.apply(Command::Expand, 0.0);
    tree.listed(
        "/home/tester/docs",
        Ok(listing("/home/tester/docs", vec![file("plan.txt")])),
    );
    let notes = row_of(&tree, "/home/tester/notes.txt");
    tree.apply(Command::Click(notes), 0.0);
    tree
}

#[test]
fn refresh_fetches_the_current_directory_and_the_expanded_ones_below_it() {
    let mut tree = opened();
    assert_eq!(tree.current_dir(), "/home/tester");
    assert_eq!(
        tree.refresh_paths(),
        ["/home/tester", "/home/tester/docs"],
        "not /, /home, or anything collapsed"
    );
    // A collapsed directory that is the Current directory is fetched on its own.
    let photos = row_of(&tree, "/home/tester/photos");
    tree.apply(Command::Click(photos), 0.0);
    assert_eq!(tree.refresh_paths(), ["/home/tester/photos"]);
    // From the root, everything expanded.
    tree.apply(Command::Home, 0.0);
    assert_eq!(
        tree.refresh_paths(),
        ["/", "/home", "/home/tester", "/home/tester/docs"]
    );
}

#[test]
fn a_merge_keeps_survivors_drops_the_removed_and_inserts_the_new_collapsed() {
    let mut tree = opened();
    // photos is removed; zebra and aardvark.txt are new; docs survives with its child.
    let mut entries = home();
    entries.retain(|e| e.name != "photos");
    entries.push(dir("zebra"));
    entries.push(file("aardvark.txt"));
    tree.refreshed("/home/tester", Ok(listing("/home/tester", entries)));
    assert_eq!(
        rows(&tree),
        [
            "/",
            " etc",
            " home",
            "  tester",
            "   docs",
            "    plan.txt",
            "   zebra",
            "   aardvark.txt",
            "   notes.txt",
            "   todo.txt",
        ]
    );
    let zebra = row_of(&tree, "/home/tester/zebra");
    assert!(matches!(
        tree.row(zebra).line,
        Line::Entry {
            expanded: Some(false),
            ..
        }
    ));
}

#[test]
fn changed_metadata_updates_in_place_and_a_changed_type_counts_as_new() {
    let mut tree = opened();
    let mut entries = home();
    for e in &mut entries {
        if e.name == "notes.txt" {
            e.size = Some(99);
        }
    }
    // docs becomes a file, then a directory again: nothing kept of the old one.
    entries.retain(|e| e.name != "docs");
    entries.push(file("docs"));
    let changed = tree.refreshed("/home/tester", Ok(listing("/home/tester", entries.clone())));
    assert!(changed, "the Focused file's size changed");
    let notes = row_of(&tree, "/home/tester/notes.txt");
    match tree.row(notes).line {
        Line::Entry { entry, .. } => assert_eq!(entry.unwrap().size, Some(99)),
        other => panic!("{other:?}"),
    }
    assert!(!rows(&tree).contains(&"    plan.txt".to_owned()));
    entries.retain(|e| e.name != "docs");
    entries.push(dir("docs"));
    let changed = tree.refreshed("/home/tester", Ok(listing("/home/tester", entries)));
    assert!(!changed, "nothing about notes.txt changed this time");
    let docs = row_of(&tree, "/home/tester/docs");
    assert!(matches!(
        tree.row(docs).line,
        Line::Entry {
            expanded: Some(false),
            ..
        }
    ));
    // Expanding it fetches it afresh.
    tree.apply(Command::Click(docs), 0.0);
    assert_eq!(
        tree.apply(Command::Expand, 0.0),
        Effect::Fetch("/home/tester/docs".into())
    );
}

#[test]
fn a_vanished_focus_moves_to_the_next_else_the_previous_else_the_parent() {
    let mut tree = opened();
    assert_eq!(tree.focused_path(), Some("/home/tester/notes.txt"));
    let without = |names: &[&str]| {
        let mut entries = home();
        entries.retain(|e| !names.contains(&e.name.as_str()));
        listing("/home/tester", entries)
    };
    tree.refreshed("/home/tester", Ok(without(&["notes.txt"])));
    assert_eq!(tree.focused_path(), Some("/home/tester/todo.txt"));
    // The emptied Selection becomes the Focused entry.
    assert_eq!(tree.selection(), ["/home/tester/todo.txt"]);
    // Nothing follows todo.txt: the previous row that is left.
    tree.refreshed("/home/tester", Ok(without(&["notes.txt", "todo.txt"])));
    assert_eq!(tree.focused_path(), Some("/home/tester/photos"));
    // Nothing left in the directory: up to it.
    tree.refreshed("/home/tester", Ok(listing("/home/tester", vec![])));
    assert_eq!(tree.focused_path(), Some("/home/tester"));
    assert_eq!(tree.selection(), ["/home/tester"]);
}

#[test]
fn vanished_entries_leave_the_selection_silently() {
    let mut tree = opened();
    tree.apply(Command::ExtendDown, 0.0);
    assert_eq!(
        tree.selection(),
        ["/home/tester/notes.txt", "/home/tester/todo.txt"]
    );
    let mut entries = home();
    entries.retain(|e| e.name != "todo.txt");
    tree.refreshed("/home/tester", Ok(listing("/home/tester", entries)));
    assert_eq!(tree.selection(), ["/home/tester/notes.txt"]);
    assert_eq!(tree.focused_path(), Some("/home/tester/notes.txt"));
}

#[test]
fn a_gone_directory_leaves_its_parent_and_an_unreadable_one_shows_the_reason() {
    let mut tree = opened();
    tree.refreshed("/home/tester/docs", Err(NO_SUCH_FILE.into()));
    assert!(!rows(&tree).iter().any(|r| r.trim() == "docs"));
    assert!(!rows(&tree).iter().any(|r| r.trim() == "plan.txt"));
    // The parent listing it again does not bring back what was kept under it.
    tree.refreshed("/home/tester", Ok(listing("/home/tester", home())));
    let docs = row_of(&tree, "/home/tester/docs");
    assert!(matches!(
        tree.row(docs).line,
        Line::Entry {
            expanded: Some(false),
            ..
        }
    ));

    let mut tree = opened();
    tree.refreshed("/home/tester/docs", Err("permission denied".into()));
    let docs = row_of(&tree, "/home/tester/docs");
    assert_eq!(rows(&tree)[docs + 1], "    Cannot read: permission denied");
    // Readable again, it lists again.
    tree.refreshed(
        "/home/tester/docs",
        Ok(listing("/home/tester/docs", vec![file("plan.txt")])),
    );
    assert_eq!(rows(&tree)[docs + 1], "    plan.txt");
}

#[test]
fn an_answer_for_a_directory_dropped_meanwhile_is_ignored() {
    let mut tree = opened();
    let mut entries = home();
    entries.retain(|e| e.name != "docs");
    tree.refreshed("/home/tester", Ok(listing("/home/tester", entries.clone())));
    tree.refreshed(
        "/home/tester/docs",
        Ok(listing("/home/tester/docs", vec![file("plan.txt")])),
    );
    assert!(!rows(&tree).iter().any(|r| r.trim() == "plan.txt"));
    entries.push(dir("docs"));
    tree.refreshed("/home/tester", Ok(listing("/home/tester", entries)));
    let docs = row_of(&tree, "/home/tester/docs");
    tree.apply(Command::Click(docs), 0.0);
    assert_eq!(
        tree.apply(Command::Expand, 0.0),
        Effect::Fetch("/home/tester/docs".into()),
        "the late answer was not kept"
    );
}

#[test]
fn auto_refresh_off_never_ticks() {
    let mut timer = Timer::default();
    for now in [0.0, 100.0, 10_000.0] {
        assert!(!timer.tick(now, AutoRefresh::Off, true));
    }
    assert_eq!(timer.wait(10_000.0, AutoRefresh::Off), None);
}

#[test]
fn auto_refresh_ticks_each_interval_and_restarts_after_any_refresh() {
    let mut timer = Timer::default();
    let every = AutoRefresh::Seconds10;
    assert!(!timer.tick(0.0, every, true), "the interval starts now");
    assert_eq!(timer.wait(4.0, every), Some(6.0));
    assert!(!timer.tick(9.9, every, true));
    assert!(timer.tick(10.0, every, true));
    assert!(!timer.tick(10.1, every, true));
    // A Refresh the operator asked for at 15 s, finished at 16 s, restarts the interval.
    timer.restart(16.0);
    assert!(!timer.tick(20.0, every, true));
    assert!(timer.tick(26.0, every, true));
    // Choosing another interval starts it afresh.
    assert!(!timer.tick(30.0, AutoRefresh::Seconds30, true));
    assert!(!timer.tick(59.0, AutoRefresh::Seconds30, true));
    assert!(timer.tick(60.0, AutoRefresh::Seconds30, true));
}

#[test]
fn a_skipped_tick_refreshes_once_when_the_session_can_again() {
    let mut timer = Timer::default();
    let every = AutoRefresh::Minute1;
    timer.tick(0.0, every, true);
    // Minimized, or a Refresh still running: skipped.
    assert!(!timer.tick(60.0, every, false));
    assert!(!timer.tick(200.0, every, false));
    // Shown again much later (a hidden tab is not ticked at all): once, then the interval.
    assert!(timer.tick(500.0, every, true));
    assert!(!timer.tick(501.0, every, true));
    assert!(timer.tick(560.0, every, true));
}

/// Folder view of the home directory: `.`, `..`, docs, photos, notes.txt, todo.txt, with
/// notes.txt focused.
fn folder() -> Folder {
    let mut folder = Folder::new(listing("/home/tester", home()), Sort::default(), false);
    folder.focus_name("notes.txt");
    folder
}

fn names(folder: &Folder) -> Vec<&str> {
    (0..folder.len()).map(|i| folder.name(i)).collect()
}

#[test]
fn folder_view_merges_a_refresh_keeping_focus_and_selection_by_name() {
    let mut f = folder();
    f.apply(Command::ExtendDown, 0.0);
    let mut entries = home();
    entries.push(file("new.txt"));
    let changed = f.refreshed(listing("/home/tester", entries));
    assert!(!changed);
    assert_eq!(
        names(&f),
        [
            ".",
            "..",
            "docs",
            "photos",
            "new.txt",
            "notes.txt",
            "todo.txt"
        ]
    );
    assert_eq!(f.name(f.focus()), "todo.txt");
    assert_eq!(f.selection(), ["notes.txt", "todo.txt"]);
}

#[test]
fn folder_view_moves_a_vanished_focus_on_and_reports_a_changed_file() {
    let mut f = folder();
    let mut entries = home();
    entries.retain(|e| e.name != "notes.txt");
    f.refreshed(listing("/home/tester", entries.clone()));
    assert_eq!(f.name(f.focus()), "todo.txt");
    assert_eq!(f.selection(), ["todo.txt"]);
    entries.retain(|e| e.name != "todo.txt");
    f.refreshed(listing("/home/tester", entries.clone()));
    assert_eq!(f.name(f.focus()), "photos");
    for e in &mut entries {
        if e.name == "photos" {
            e.modified = Some(7);
        }
    }
    assert!(
        !f.refreshed(listing("/home/tester", entries.clone())),
        "a directory is not previewed"
    );
    entries.push(file("todo.txt"));
    f.refreshed(listing("/home/tester", entries.clone()));
    f.focus_name("todo.txt");
    for e in &mut entries {
        if e.name == "todo.txt" {
            e.modified = Some(7);
        }
    }
    assert!(f.refreshed(listing("/home/tester", entries)));
}
