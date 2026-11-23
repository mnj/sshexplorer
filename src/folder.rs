//! Folder view's listing, apart from egui: one directory's lines in their sort order, the
//! Focused entry, the Selection, and what each browsing command does to them.
//!
//! The lines are `.` and `..`, then the directories, then the rest, each group in the
//! Session's [`Sort`]. Names compare in natural order ignoring case ([`natural_cmp`]).
//! Hidden entries (dot-names) are left out unless they are shown.
//!
//! The Selection holds entries of this one directory, never `.` or `..`. Focus and
//! Selection are kept by name, so they survive a new sort or showing hidden entries.
//! The keys and the pointer reach the folder as [`Command`]s; a command that leaves the
//! directory answers with an [`Effect`] for the Session to carry out.

use std::cmp::Ordering;
use std::collections::BTreeSet;

use crate::ssh::{Entry, Listing};

/// How long a typed name prefix lasts after the last character, in seconds.
pub const TYPE_RESET: f64 = 1.0;

/// A column of the listing.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Column {
    #[default]
    Name,
    Size,
    Modified,
    Permissions,
}

impl Column {
    /// In the order the listing shows them.
    pub const ALL: [Column; 4] = [
        Column::Name,
        Column::Size,
        Column::Modified,
        Column::Permissions,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Column::Name => "Name",
            Column::Size => "Size",
            Column::Modified => "Modified",
            Column::Permissions => "Permissions",
        }
    }
}

/// The column a listing is sorted by, and which way. A Session's sort starts as Name
/// ascending.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Sort {
    pub column: Column,
    pub ascending: bool,
}

impl Default for Sort {
    fn default() -> Self {
        Sort {
            column: Column::Name,
            ascending: true,
        }
    }
}

impl Sort {
    /// The sort after clicking `column`'s header: that column ascending, or reversed when
    /// it is already sorted by.
    pub fn by(self, column: Column) -> Sort {
        if self.column == column {
            Sort {
                column,
                ascending: !self.ascending,
            }
        } else {
            Sort {
                column,
                ascending: true,
            }
        }
    }

    /// The sort after the Sort action: the next column, ascending.
    pub fn next(self) -> Sort {
        let i = Column::ALL
            .iter()
            .position(|c| *c == self.column)
            .unwrap_or(0);
        Sort {
            column: Column::ALL[(i + 1) % Column::ALL.len()],
            ascending: true,
        }
    }
}

/// Natural order ignoring case: runs of digits compare by value, so `file2` comes before
/// `file10`. Names equal that way fall back to their exact text, so the order is total.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(p), Some(q)) if p.is_ascii_digit() && q.is_ascii_digit() => {
                let m = digits(&mut x);
                let n = digits(&mut y);
                let (m, n) = (m.trim_start_matches('0'), n.trim_start_matches('0'));
                let by_value = m.len().cmp(&n.len()).then_with(|| m.cmp(n));
                if by_value != Ordering::Equal {
                    return by_value;
                }
            }
            (Some(p), Some(q)) => {
                let by_char = p.to_lowercase().cmp(q.to_lowercase());
                if by_char != Ordering::Equal {
                    return by_char;
                }
                x.next();
                y.next();
            }
        }
    }
}

fn digits(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut run = String::new();
    while let Some(c) = chars.next_if(char::is_ascii_digit) {
        run.push(c);
    }
    run
}

/// One line of the listing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Line {
    /// `.`, the listed directory itself.
    Here,
    /// `..`, its parent.
    Up,
    /// The entry at this index of the listing.
    Entry(usize),
}

/// What a browsing command asks of the keys or the pointer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Command {
    Up,
    Down,
    Home,
    End,
    /// Up by this many lines: a page.
    PageUp(usize),
    PageDown(usize),
    /// Moves the focus and extends the Selection from its anchor to it.
    ExtendUp,
    ExtendDown,
    /// Moves the focus without changing the Selection.
    FocusUp,
    FocusDown,
    /// Toggles the Focused entry in the Selection.
    Toggle,
    SelectAll,
    /// Lists a directory or `..`; on a file, asks for Preview.
    Open,
    /// Lists the parent.
    Parent,
    /// Tree view: expands a directory, or moves to the first child of an expanded one.
    /// Folder view does nothing.
    Expand,
    /// Tree view: collapses a directory, or moves to the parent. Folder view does nothing.
    Collapse,
    /// A typed character of a name prefix.
    Type(char),
    /// A plain click on a line.
    Click(usize),
    CtrlClick(usize),
    ShiftClick(usize),
    DoubleClick(usize),
    /// Tree view: a click on a line's chevron opens or closes that directory. Folder view
    /// does nothing.
    Disclose(usize),
}

/// What the Session does after a command.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Effect {
    None,
    /// List the directory at `path`, and focus and select `focus` in it.
    List {
        path: String,
        focus: Option<String>,
    },
    /// Move the keyboard to Preview, which shows the Focused entry.
    Preview,
}

/// One listed directory with its focus and Selection.
#[derive(Clone, Debug)]
pub struct Folder {
    listing: Listing,
    lines: Vec<Line>,
    focus: usize,
    selected: BTreeSet<String>,
    /// Where a range starts: the entry last clicked, toggled or moved to.
    anchor: Option<String>,
    typed: String,
    typed_at: f64,
    /// What the lines were last arranged with.
    sort: Sort,
    show_hidden: bool,
}

impl Folder {
    /// `listing` in `sort`, focused on its first line with nothing selected.
    pub fn new(listing: Listing, sort: Sort, show_hidden: bool) -> Folder {
        let mut folder = Folder {
            listing,
            lines: Vec::new(),
            focus: 0,
            selected: BTreeSet::new(),
            anchor: None,
            typed: String::new(),
            typed_at: f64::NEG_INFINITY,
            sort,
            show_hidden,
        };
        folder.arrange(sort, show_hidden);
        folder
    }

    /// Merges a Refresh's `listing` of this directory, and gives whether the Focused
    /// entry is a file that survived with a changed size or modified time, for Preview to
    /// reload. Focus and Selection stay by name; a vanished Focused entry moves focus to
    /// the next line left, else the previous one. Vanished entries leave the Selection,
    /// and an emptied Selection becomes the Focused entry.
    pub fn refreshed(&mut self, listing: Listing) -> bool {
        let names: Vec<String> = (0..self.len()).map(|i| self.name(i).to_owned()).collect();
        let at = self.focus;
        let stamp = |f: &Folder| {
            f.entry(f.focus)
                .filter(|e| !e.is_dir)
                .map(|e| (e.name.clone(), e.size, e.modified))
        };
        let before = stamp(self);
        let had_selection = !self.selected.is_empty();
        self.listing = listing;
        // The old lines index the old listing: lay out afresh and find the focus by name.
        self.lines.clear();
        self.arrange(self.sort, self.show_hidden);
        let next = || names[at + 1..].iter().find_map(|n| self.find(n));
        let previous = || names[..at].iter().rev().find_map(|n| self.find(n));
        self.focus = self
            .find(&names[at])
            .or_else(next)
            .or_else(previous)
            .unwrap_or(0);
        if had_selection && self.selected.is_empty() {
            self.choose(self.focus);
        }
        match (before, stamp(self)) {
            (Some(before), Some(after)) => after.0 == before.0 && after != before,
            _ => false,
        }
    }

    /// The listed directory's absolute path.
    pub fn path(&self) -> &str {
        &self.listing.path
    }

    /// The number of lines.
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// Never: `.` and `..` are always there.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn line(&self, i: usize) -> Line {
        self.lines[i]
    }

    /// The entry on line `i`; `None` for `.` and `..`.
    pub fn entry(&self, i: usize) -> Option<&Entry> {
        match self.lines.get(i)? {
            Line::Entry(e) => self.listing.entries.get(*e),
            _ => None,
        }
    }

    /// The name on line `i`.
    pub fn name(&self, i: usize) -> &str {
        match self.lines[i] {
            Line::Here => ".",
            Line::Up => "..",
            Line::Entry(e) => &self.listing.entries[e].name,
        }
    }

    /// The Focused entry's line.
    pub fn focus(&self) -> usize {
        self.focus
    }

    pub fn is_selected(&self, i: usize) -> bool {
        self.entry(i)
            .is_some_and(|e| self.selected.contains(&e.name))
    }

    /// The selected names, in line order.
    pub fn selection(&self) -> Vec<&str> {
        (0..self.len())
            .filter(|i| self.is_selected(*i))
            .map(|i| self.name(i))
            .collect()
    }

    /// Lays the lines out again in `sort`, with or without hidden entries. Focus stays on
    /// its name while that is still listed; entries no longer listed leave the Selection.
    pub fn arrange(&mut self, sort: Sort, show_hidden: bool) {
        self.sort = sort;
        self.show_hidden = show_hidden;
        let focused = self
            .lines
            .get(self.focus)
            .map(|_| self.name(self.focus).to_owned());
        let entries = &self.listing.entries;
        let mut shown: Vec<usize> = (0..entries.len())
            .filter(|i| show_hidden || !entries[*i].name.starts_with('.'))
            .collect();
        shown.sort_by(|a, b| {
            let (a, b) = (&entries[*a], &entries[*b]);
            b.is_dir.cmp(&a.is_dir).then_with(|| {
                let order = by_column(sort.column, a, b);
                if sort.ascending {
                    order
                } else {
                    order.reverse()
                }
            })
        });
        self.lines = [Line::Here, Line::Up]
            .into_iter()
            .chain(shown.into_iter().map(Line::Entry))
            .collect();
        self.focus = focused
            .and_then(|name| self.find(&name))
            .unwrap_or(0)
            .min(self.lines.len() - 1);
        let visible: BTreeSet<String> = (0..self.len())
            .filter_map(|i| self.entry(i).map(|e| e.name.clone()))
            .collect();
        self.selected.retain(|name| visible.contains(name));
    }

    /// Focuses and selects the entry `name`, when it is listed: the directory just left.
    pub fn focus_name(&mut self, name: &str) {
        if let Some(i) = self.find(name) {
            self.choose(i);
        }
    }

    /// Carries out `command`; `now` is in seconds, for the typed prefix.
    pub fn apply(&mut self, command: Command, now: f64) -> Effect {
        let last = self.len() - 1;
        let focus = self.focus;
        let up = move |n: usize| focus.saturating_sub(n);
        let down = move |n: usize| (focus + n).min(last);
        match command {
            Command::Up => self.choose(up(1)),
            Command::Down => self.choose(down(1)),
            Command::Home => self.choose(0),
            Command::End => self.choose(last),
            Command::PageUp(n) => self.choose(up(n.max(1))),
            Command::PageDown(n) => self.choose(down(n.max(1))),
            Command::ExtendUp => self.extend_to(up(1)),
            Command::ExtendDown => self.extend_to(down(1)),
            Command::FocusUp => self.focus = up(1),
            Command::FocusDown => self.focus = down(1),
            Command::Toggle => self.toggle(self.focus),
            Command::SelectAll => {
                self.selected = (0..self.len())
                    .filter_map(|i| self.entry(i).map(|e| e.name.clone()))
                    .collect();
            }
            Command::Open => return self.open(self.focus),
            Command::Parent => return self.parent(),
            Command::Expand | Command::Collapse => {}
            Command::Type(c) => self.typed(c, now),
            Command::Click(i) => self.choose(i.min(last)),
            Command::CtrlClick(i) => self.toggle(i.min(last)),
            Command::ShiftClick(i) => self.extend_to(i.min(last)),
            Command::DoubleClick(i) => {
                self.choose(i.min(last));
                return self.open(self.focus);
            }
            Command::Disclose(_) => {}
        }
        Effect::None
    }

    fn find(&self, name: &str) -> Option<usize> {
        (0..self.len()).find(|i| self.name(*i) == name)
    }

    /// Focus and Selection both to line `i`; `.` and `..` are never selected.
    fn choose(&mut self, i: usize) {
        self.focus = i;
        self.selected.clear();
        self.anchor = self.entry(i).map(|e| e.name.clone());
        if let Some(name) = self.anchor.clone() {
            self.selected.insert(name);
        }
    }

    fn toggle(&mut self, i: usize) {
        self.focus = i;
        let Some(name) = self.entry(i).map(|e| e.name.clone()) else {
            return;
        };
        if !self.selected.remove(&name) {
            self.selected.insert(name.clone());
        }
        self.anchor = Some(name);
    }

    /// Focus to line `i`, and the Selection to the range from the anchor to it.
    fn extend_to(&mut self, i: usize) {
        let from = self
            .anchor
            .as_deref()
            .and_then(|name| self.find(name))
            .unwrap_or(self.focus);
        self.focus = i;
        let (lo, hi) = (from.min(i), from.max(i));
        self.selected = (lo..=hi)
            .filter_map(|l| self.entry(l).map(|e| e.name.clone()))
            .collect();
        if self.anchor.is_none() {
            self.anchor = self.entry(from).map(|e| e.name.clone());
        }
    }

    fn open(&self, i: usize) -> Effect {
        match self.lines[i] {
            Line::Here => Effect::None,
            Line::Up => self.parent(),
            Line::Entry(e) => {
                let entry = &self.listing.entries[e];
                if entry.is_dir {
                    Effect::List {
                        path: join(self.path(), &entry.name),
                        focus: None,
                    }
                } else {
                    Effect::Preview
                }
            }
        }
    }

    fn parent(&self) -> Effect {
        match parent(self.path()) {
            Some((parent, left)) => Effect::List {
                path: parent,
                focus: Some(left),
            },
            None => Effect::None,
        }
    }

    /// Jumps to the next name that starts with what was typed. A pause of
    /// [`TYPE_RESET`] starts the prefix again; one character typed again moves on to the
    /// next name with it.
    fn typed(&mut self, c: char, now: f64) {
        if now - self.typed_at > TYPE_RESET {
            self.typed.clear();
        }
        self.typed_at = now;
        self.typed.push(c);
        let mut chars = self.typed.chars();
        let first = chars.next().unwrap_or(c);
        let repeat = chars.all(|d| d == first);
        let (prefix, start) = if repeat {
            (first.to_lowercase().to_string(), self.focus + 1)
        } else {
            (self.typed.to_lowercase(), self.focus)
        };
        let len = self.len();
        let hit = (0..len).map(|k| (start + k) % len).find(|i| {
            self.entry(*i)
                .is_some_and(|e| e.name.to_lowercase().starts_with(&prefix))
        });
        if let Some(i) = hit {
            self.choose(i);
        }
    }
}

/// How two entries compare by `column`, ties broken by [`natural_cmp`] on the name.
pub fn by_column(column: Column, a: &Entry, b: &Entry) -> Ordering {
    let key = match column {
        Column::Name => Ordering::Equal,
        Column::Size => a.size.cmp(&b.size),
        Column::Modified => a.modified.cmp(&b.modified),
        Column::Permissions => a.permissions.cmp(&b.permissions),
    };
    key.then_with(|| natural_cmp(&a.name, &b.name))
}

/// `name` in the directory `dir`.
pub fn join(dir: &str, name: &str) -> String {
    format!("{}/{name}", dir.trim_end_matches('/'))
}

/// The parent of the absolute path `path`, and the name of `path` in it; `None` at `/`.
pub fn parent(path: &str) -> Option<(String, String)> {
    let trimmed = path.trim_end_matches('/');
    let (dir, name) = trimmed.rsplit_once('/')?;
    if name.is_empty() {
        return None;
    }
    let dir = if dir.is_empty() { "/" } else { dir };
    Some((dir.to_owned(), name.to_owned()))
}

/// A file size as the Size column shows it: bytes below 1 KiB, else KiB, MiB, GiB or TiB
/// with one decimal.
pub fn size_text(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KiB", "MiB", "GiB", "TiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// Mode bits as `ls -l` shows them: the type, then read, write and execute for owner,
/// group and others, with setuid, setgid and sticky folded in.
pub fn permissions_text(mode: u32) -> String {
    let kind = match mode & 0o170000 {
        0o040000 => 'd',
        0o120000 => 'l',
        0o020000 => 'c',
        0o060000 => 'b',
        0o010000 => 'p',
        0o140000 => 's',
        _ => '-',
    };
    let mut text = String::from(kind);
    for (shift, special, set) in [(6, 0o4000, 's'), (3, 0o2000, 's'), (0, 0o1000, 't')] {
        let bits = (mode >> shift) & 0o7;
        text.push(if bits & 4 != 0 { 'r' } else { '-' });
        text.push(if bits & 2 != 0 { 'w' } else { '-' });
        let exec = bits & 1 != 0;
        text.push(match (mode & special != 0, exec) {
            (true, true) => set,
            (true, false) => set.to_ascii_uppercase(),
            (false, true) => 'x',
            (false, false) => '-',
        });
    }
    text
}

/// A modification time, seconds since the epoch, as `YYYY-MM-DD HH:MM` in `zone`.
pub fn modified_text(seconds: u32, zone: &jiff::tz::TimeZone) -> String {
    match jiff::Timestamp::from_second(i64::from(seconds)) {
        Ok(at) => zone.to_datetime(at).strftime("%Y-%m-%d %H:%M").to_string(),
        Err(_) => String::new(),
    }
}
