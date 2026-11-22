//! Folder view's listing apart from egui: sort, hidden entries, focus and Selection, and
//! the browsing commands the keys and the pointer give.

use std::cmp::Ordering;

use sshexplorer::folder::{Column, Command, Effect, Folder, Sort, natural_cmp};
use sshexplorer::ssh::{Entry, Listing};

fn file(name: &str, size: u64, mtime: u32) -> Entry {
    Entry {
        name: name.into(),
        size: Some(size),
        modified: Some(mtime),
        permissions: Some(0o100644),
        ..Entry::default()
    }
}

fn dir(name: &str) -> Entry {
    Entry {
        name: name.into(),
        is_dir: true,
        permissions: Some(0o040755),
        ..Entry::default()
    }
}

fn listing(path: &str, entries: Vec<Entry>) -> Listing {
    Listing {
        path: path.into(),
        entries,
    }
}

fn names(folder: &Folder) -> Vec<&str> {
    (0..folder.len()).map(|i| folder.name(i)).collect()
}

fn home() -> Folder {
    Folder::new(
        listing(
            "/home/tester",
            vec![
                file("file10", 30, 3),
                file("File2", 10, 1),
                dir("photos"),
                file(".profile", 5, 9),
                dir("Docs"),
                file("file1", 20, 2),
            ],
        ),
        Sort::default(),
        false,
    )
}

#[test]
fn names_compare_naturally_ignoring_case() {
    assert_eq!(natural_cmp("file2", "file10"), Ordering::Less);
    assert_eq!(natural_cmp("File2", "file10"), Ordering::Less);
    assert_eq!(natural_cmp("b", "A"), Ordering::Greater);
    assert_eq!(natural_cmp("a01", "a1"), Ordering::Less);
    assert_ne!(natural_cmp("File2", "file2"), Ordering::Equal);
    assert_eq!(natural_cmp("x9y", "x10"), Ordering::Less);
    assert_eq!(natural_cmp("abc", "abcd"), Ordering::Less);
}

#[test]
fn dots_then_directories_then_files_by_natural_name_without_hidden_entries() {
    assert_eq!(
        names(&home()),
        [".", "..", "Docs", "photos", "file1", "File2", "file10"]
    );
}

#[test]
fn show_hidden_lists_dot_names_too() {
    let mut folder = home();
    folder.arrange(Sort::default(), true);
    assert_eq!(
        names(&folder),
        [
            ".", "..", "Docs", "photos", ".profile", "file1", "File2", "file10"
        ]
    );
}

#[test]
fn clicking_a_header_sorts_by_it_and_again_reverses_keeping_directories_first() {
    let sort = Sort::default().by(Column::Size);
    assert_eq!(
        sort,
        Sort {
            column: Column::Size,
            ascending: true
        }
    );
    let mut folder = home();
    folder.arrange(sort, false);
    assert_eq!(
        names(&folder),
        [".", "..", "Docs", "photos", "File2", "file1", "file10"]
    );
    let reversed = sort.by(Column::Size);
    assert!(!reversed.ascending);
    folder.arrange(reversed, false);
    assert_eq!(
        names(&folder),
        [".", "..", "photos", "Docs", "file10", "file1", "File2"]
    );
    folder.arrange(Sort::default().by(Column::Name), false);
    assert_eq!(
        names(&folder),
        [".", "..", "photos", "Docs", "file10", "File2", "file1"]
    );
}

#[test]
fn sort_by_modified_and_permissions() {
    let mut folder = home();
    folder.arrange(
        Sort {
            column: Column::Modified,
            ascending: true,
        },
        false,
    );
    assert_eq!(
        names(&folder),
        [".", "..", "Docs", "photos", "File2", "file1", "file10"]
    );
    folder.arrange(
        Sort {
            column: Column::Permissions,
            ascending: false,
        },
        false,
    );
    // Equal permissions fall back to the name, in the same direction.
    assert_eq!(
        names(&folder),
        [".", "..", "photos", "Docs", "file10", "File2", "file1"]
    );
}

#[test]
fn the_sort_action_steps_to_the_next_column_ascending() {
    let mut sort = Sort::default();
    let mut seen = Vec::new();
    for _ in 0..4 {
        sort = sort.next();
        seen.push(sort.column);
        assert!(sort.ascending);
    }
    assert_eq!(
        seen,
        [
            Column::Size,
            Column::Modified,
            Column::Permissions,
            Column::Name
        ]
    );
}

fn selection(folder: &Folder) -> Vec<&str> {
    folder.selection()
}

fn focused(folder: &Folder) -> &str {
    folder.name(folder.focus())
}

#[test]
fn a_new_folder_focuses_the_first_line_and_selects_nothing() {
    let folder = home();
    assert_eq!(focused(&folder), ".");
    assert!(selection(&folder).is_empty());
}

#[test]
fn arrows_set_focus_and_selection_to_one_entry_but_never_select_dots() {
    let mut folder = home();
    folder.apply(Command::Down, 0.0);
    assert_eq!(focused(&folder), "..");
    assert!(selection(&folder).is_empty());
    folder.apply(Command::Down, 0.0);
    folder.apply(Command::Down, 0.0);
    assert_eq!(focused(&folder), "photos");
    assert_eq!(selection(&folder), ["photos"]);
    folder.apply(Command::Up, 0.0);
    assert_eq!(selection(&folder), ["Docs"]);
    folder.apply(Command::End, 0.0);
    assert_eq!(focused(&folder), "file10");
    folder.apply(Command::Down, 0.0);
    assert_eq!(focused(&folder), "file10", "stays on the last line");
    folder.apply(Command::Home, 0.0);
    assert_eq!(focused(&folder), ".");
    folder.apply(Command::Up, 0.0);
    assert_eq!(focused(&folder), ".");
}

#[test]
fn page_keys_move_by_a_page() {
    let mut folder = home();
    folder.apply(Command::PageDown(3), 0.0);
    assert_eq!(focused(&folder), "photos");
    folder.apply(Command::PageDown(3), 0.0);
    assert_eq!(focused(&folder), "file10");
    folder.apply(Command::PageUp(4), 0.0);
    assert_eq!(focused(&folder), "Docs");
    assert_eq!(selection(&folder), ["Docs"]);
    folder.apply(Command::PageUp(4), 0.0);
    assert_eq!(focused(&folder), ".");
}

#[test]
fn shift_arrows_extend_and_ctrl_arrows_move_focus_only() {
    let mut folder = home();
    folder.apply(Command::Click(3), 0.0); // photos
    folder.apply(Command::ExtendDown, 0.0);
    folder.apply(Command::ExtendDown, 0.0);
    assert_eq!(selection(&folder), ["photos", "file1", "File2"]);
    folder.apply(Command::ExtendUp, 0.0);
    assert_eq!(selection(&folder), ["photos", "file1"]);
    folder.apply(Command::FocusDown, 0.0);
    folder.apply(Command::FocusDown, 0.0);
    assert_eq!(focused(&folder), "file10");
    assert_eq!(selection(&folder), ["photos", "file1"]);
    folder.apply(Command::Toggle, 0.0);
    assert_eq!(selection(&folder), ["photos", "file1", "file10"]);
    folder.apply(Command::Toggle, 0.0);
    assert_eq!(selection(&folder), ["photos", "file1"]);
    folder.apply(Command::FocusUp, 0.0);
    assert_eq!(focused(&folder), "File2");
}

#[test]
fn ctrl_click_toggles_and_shift_click_selects_a_range_from_the_anchor() {
    let mut folder = home();
    folder.apply(Command::Click(2), 0.0); // Docs
    folder.apply(Command::CtrlClick(5), 0.0); // File2
    assert_eq!(selection(&folder), ["Docs", "File2"]);
    assert_eq!(focused(&folder), "File2");
    folder.apply(Command::CtrlClick(2), 0.0);
    assert_eq!(selection(&folder), ["File2"]);
    // The anchor is the entry last clicked or toggled.
    folder.apply(Command::ShiftClick(3), 0.0);
    assert_eq!(selection(&folder), ["Docs", "photos"]);
    folder.apply(Command::ShiftClick(6), 0.0);
    assert_eq!(
        selection(&folder),
        ["Docs", "photos", "file1", "File2", "file10"]
    );
    assert_eq!(focused(&folder), "file10");
    // A range that reaches the dots leaves them out.
    folder.apply(Command::ShiftClick(0), 0.0);
    assert_eq!(selection(&folder), ["Docs"]);
}

#[test]
fn ctrl_a_selects_every_visible_entry_but_the_dots() {
    let mut folder = home();
    folder.apply(Command::SelectAll, 0.0);
    assert_eq!(
        selection(&folder),
        ["Docs", "photos", "file1", "File2", "file10"]
    );
}

#[test]
fn hiding_entries_drops_them_from_the_selection_and_keeps_focus_by_name() {
    let mut folder = home();
    folder.arrange(Sort::default(), true);
    folder.apply(Command::SelectAll, 0.0);
    folder.apply(Command::CtrlClick(5), 0.0); // file1, focused
    folder.arrange(Sort::default().by(Column::Name), false);
    assert_eq!(focused(&folder), "file1");
    assert_eq!(selection(&folder), ["photos", "Docs", "file10", "File2"]);
}

#[test]
fn typing_jumps_to_a_name_prefix_and_the_buffer_resets_after_a_second() {
    let mut folder = home();
    folder.apply(Command::Type('f'), 10.0);
    assert_eq!(focused(&folder), "file1");
    assert_eq!(selection(&folder), ["file1"]);
    folder.apply(Command::Type('I'), 10.3);
    folder.apply(Command::Type('l'), 10.6);
    folder.apply(Command::Type('e'), 10.9);
    folder.apply(Command::Type('1'), 11.2);
    folder.apply(Command::Type('0'), 11.5);
    assert_eq!(focused(&folder), "file10");
    // After a pause the buffer starts again.
    folder.apply(Command::Type('p'), 13.0);
    assert_eq!(focused(&folder), "photos");
    // The same letter again moves on to the next name with it.
    folder.apply(Command::Type('f'), 15.0);
    folder.apply(Command::Type('f'), 15.2);
    assert_eq!(focused(&folder), "File2");
    // No match leaves the focus where it is.
    folder.apply(Command::Type('z'), 17.0);
    assert_eq!(focused(&folder), "File2");
}

#[test]
fn enter_lists_a_directory_and_dot_dot_and_previews_a_file() {
    let mut folder = home();
    assert_eq!(folder.apply(Command::Open, 0.0), Effect::None, "on `.`");
    folder.apply(Command::Down, 0.0);
    assert_eq!(
        folder.apply(Command::Open, 0.0),
        Effect::List {
            path: "/home".into(),
            focus: Some("tester".into())
        }
    );
    folder.apply(Command::Down, 0.0);
    assert_eq!(
        folder.apply(Command::Open, 0.0),
        Effect::List {
            path: "/home/tester/Docs".into(),
            focus: None
        }
    );
    folder.apply(Command::End, 0.0);
    assert_eq!(folder.apply(Command::Open, 0.0), Effect::Preview);
    assert_eq!(
        folder.apply(Command::DoubleClick(3), 0.0),
        Effect::List {
            path: "/home/tester/photos".into(),
            focus: None
        }
    );
}

#[test]
fn parent_lists_the_parent_and_selects_the_directory_just_left() {
    let mut folder = home();
    assert_eq!(
        folder.apply(Command::Parent, 0.0),
        Effect::List {
            path: "/home".into(),
            focus: Some("tester".into())
        }
    );
    let mut up = Folder::new(
        listing("/home", vec![dir("other"), dir("tester")]),
        Sort::default(),
        false,
    );
    up.focus_name("tester");
    assert_eq!(focused(&up), "tester");
    assert_eq!(selection(&up), ["tester"]);
    let mut root = Folder::new(listing("/", vec![dir("home")]), Sort::default(), false);
    assert_eq!(root.apply(Command::Parent, 0.0), Effect::None);
    root.apply(Command::End, 0.0);
    assert_eq!(
        root.apply(Command::Open, 0.0),
        Effect::List {
            path: "/home".into(),
            focus: None
        }
    );
}

#[test]
fn the_columns_read_as_sizes_times_and_mode_strings() {
    use sshexplorer::folder::{modified_text, permissions_text, size_text};
    assert_eq!(size_text(0), "0 B");
    assert_eq!(size_text(1023), "1023 B");
    assert_eq!(size_text(1536), "1.5 KiB");
    assert_eq!(size_text(10 * 1024 * 1024), "10.0 MiB");
    assert_eq!(size_text(3 * 1024 * 1024 * 1024), "3.0 GiB");
    assert_eq!(permissions_text(0o040755), "drwxr-xr-x");
    assert_eq!(permissions_text(0o100640), "-rw-r-----");
    assert_eq!(permissions_text(0o120777), "lrwxrwxrwx");
    assert_eq!(permissions_text(0o104755), "-rwsr-xr-x");
    assert_eq!(permissions_text(0o041777), "drwxrwxrwt");
    let utc = jiff::tz::TimeZone::UTC;
    assert_eq!(modified_text(1_700_000_000, &utc), "2023-11-14 22:13");
}

#[test]
fn a_symlinked_directory_is_entered_like_a_directory() {
    let mut link = dir("site");
    link.link = Some("/var/www".into());
    let mut folder = Folder::new(listing("/srv", vec![link]), Sort::default(), false);
    folder.apply(Command::End, 0.0);
    assert_eq!(
        folder.apply(Command::Open, 0.0),
        Effect::List {
            path: "/srv/site".into(),
            focus: None
        }
    );
}
