//! Tree view's model, apart from egui: the remote filesystem as a tree rooted at `/`, the
//! directories fetched so far, which are expanded, the Focused entry and the Selection.
//!
//! A directory is fetched the first time it is expanded (or listed in Folder view, which
//! hands its listing over with [`Tree::listed`]) and is kept for the Session. While a
//! fetch runs its directory shows one "Loading…" row; a failed fetch shows one row with
//! the reason. Those rows are passed over by the keys.
//!
//! Each directory's children are in the Session's [`Sort`], directories first, with
//! hidden entries left out unless they are shown. Focus and Selection are kept by path.
//! The Selection holds entries of one parent directory, never `/`; selecting an entry
//! under another parent starts a new Selection. A command that needs the host answers
//! with an [`Effect`] for the Session to carry out.

use std::collections::{BTreeSet, HashMap};

use crate::folder::{Command, Sort, TYPE_RESET, by_column, join, parent};
use crate::ssh::{Entry, Listing, NO_SUCH_FILE};

/// What the Session does after a command.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Effect {
    None,
    /// Fetch the directory at this path and hand it to [`Tree::listed`].
    Fetch(String),
    /// Move the keyboard to Preview, which shows the Focused entry.
    Preview,
}

/// What one row of the tree shows.
#[derive(Clone, Copy, Debug)]
pub enum Line<'a> {
    /// An entry, or `/` itself, which has no `entry`.
    Entry {
        name: &'a str,
        path: &'a str,
        entry: Option<&'a Entry>,
        /// Whether a directory is expanded; `None` for anything else.
        expanded: Option<bool>,
    },
    /// The directory above is being fetched.
    Loading,
    /// The directory above could not be fetched, for this reason.
    Failed(&'a str),
}

/// One row: what it shows and how deep it is, `/` being 0.
#[derive(Clone, Copy, Debug)]
pub struct Row<'a> {
    pub depth: usize,
    pub line: Line<'a>,
}

enum Dir {
    Loading,
    Failed(String),
    Listed(Listing),
}

#[derive(Clone, Debug)]
enum Kind {
    Root,
    /// The entry at `index` in the listing of the directory `dir`.
    Entry {
        dir: String,
        index: usize,
    },
    Loading,
    Failed,
}

/// A row as laid out: its path (a status row's is its directory's) and what it is.
#[derive(Clone, Debug)]
struct Laid {
    depth: usize,
    path: String,
    kind: Kind,
}

pub struct Tree {
    dirs: HashMap<String, Dir>,
    expanded: BTreeSet<String>,
    sort: Sort,
    show_hidden: bool,
    rows: Vec<Laid>,
    /// The Focused entry's path; `None` until the Initial remote folder shows.
    focus: Option<String>,
    /// An entry to focus once it shows: the Initial remote folder, or what
    /// [`Tree::reveal`] expands to.
    pending: Option<String>,
    /// The Initial remote folder.
    initial: String,
    selected: BTreeSet<String>,
    anchor: Option<String>,
    typed: String,
    typed_at: f64,
    reveal: bool,
}

impl Tree {
    /// An empty tree: `/` expanded and not yet fetched.
    pub fn new(sort: Sort, show_hidden: bool) -> Tree {
        let mut tree = Tree {
            dirs: HashMap::new(),
            expanded: BTreeSet::from(["/".to_owned()]),
            sort,
            show_hidden,
            rows: Vec::new(),
            focus: None,
            pending: None,
            initial: "/".to_owned(),
            selected: BTreeSet::new(),
            anchor: None,
            typed: String::new(),
            typed_at: f64::NEG_INFINITY,
            reveal: false,
        };
        tree.layout();
        tree
    }

    /// Opens the tree at the Initial remote folder, whose `listing` is already fetched:
    /// its ancestors are expanded, and it is focused once they arrive. Gives the
    /// directories to fetch.
    pub fn open_at(&mut self, listing: Listing) -> Vec<String> {
        let path = listing.path.clone();
        self.initial = path.clone();
        self.dirs.insert(path.clone(), Dir::Listed(listing));
        self.reveal_path(&path)
    }

    /// Expands to the directory `dir` and focuses the entry `name` in it, or `dir` itself
    /// when `name` is `None`. Gives the directories to fetch.
    pub fn reveal(&mut self, dir: &str, name: Option<&str>) -> Vec<String> {
        let path = match name {
            Some(name) => join(dir, name),
            None => dir.to_owned(),
        };
        self.reveal_path(&path)
    }

    fn reveal_path(&mut self, path: &str) -> Vec<String> {
        let mut fetch = Vec::new();
        let mut ancestors = Vec::new();
        let mut at = path.to_owned();
        while let Some((up, _)) = parent(&at) {
            ancestors.push(up.clone());
            at = up;
        }
        for dir in ancestors.into_iter().rev() {
            self.expanded.insert(dir.clone());
            if !self.dirs.contains_key(&dir) {
                self.dirs.insert(dir.clone(), Dir::Loading);
                fetch.push(dir);
            }
        }
        self.pending = Some(path.to_owned());
        self.layout();
        fetch
    }

    /// Takes the answer for the directory at `path`. A failure is only kept for a
    /// directory the tree was fetching; a listing is always kept.
    pub fn listed(&mut self, path: &str, answer: Result<Listing, String>) {
        match answer {
            Ok(listing) => {
                self.dirs.insert(path.to_owned(), Dir::Listed(listing));
            }
            Err(reason) => {
                if matches!(self.dirs.get(path), Some(Dir::Loading)) {
                    self.dirs.insert(path.to_owned(), Dir::Failed(reason));
                }
            }
        }
        self.layout();
    }

    /// What a Refresh fetches again: the Current directory, then every expanded
    /// directory showing below it, in row order. Kept directories that are collapsed or
    /// elsewhere are left alone.
    pub fn refresh_paths(&self) -> Vec<String> {
        let current = self.current_dir();
        let below = |path: &str| current == "/" || path.starts_with(&format!("{current}/"));
        let mut paths = vec![current.clone()];
        for i in 0..self.len() {
            let Some(path) = self.path(i) else {
                continue;
            };
            if below(path) && self.is_dir(i) && self.expanded.contains(path) && path != current {
                paths.push(path.to_owned());
            }
        }
        paths
    }

    /// Merges a Refresh's answer for the directory at `path` into what is kept, and gives
    /// whether the Focused entry is a file that survived with a changed size or modified
    /// time, for Preview to reload.
    ///
    /// Surviving entries keep their expansion and kept children; a removed directory, or
    /// one that is no longer a directory, goes with all that was kept under it, so a new
    /// entry of that name is collapsed and not yet fetched. A directory that no longer
    /// exists ([`NO_SUCH_FILE`]) leaves its parent; one that cannot be read shows the
    /// reason in place of its children. An answer for a directory that is no longer
    /// listed is dropped. If the Focused entry vanished, focus moves to the next row left,
    /// else the previous one, else the parent; an emptied Selection becomes the Focused
    /// entry.
    pub fn refreshed(&mut self, path: &str, answer: Result<Listing, String>) -> bool {
        if !self.still_listed(path) {
            return false;
        }
        let nav: Vec<String> = self
            .nav()
            .into_iter()
            .filter_map(|i| self.path(i).map(str::to_owned))
            .collect();
        let focus = self.focus.clone();
        let stamp = |tree: &Tree| {
            let path = tree.focus.as_deref()?;
            let entry = tree.entry_of(path).filter(|e| !e.is_dir)?;
            Some((path.to_owned(), entry.size, entry.modified))
        };
        let before = stamp(self);
        let had_selection = !self.selected.is_empty();

        match answer {
            Ok(listing) => {
                if let Some(Dir::Listed(old)) = self.dirs.get(path) {
                    let dropped: Vec<String> = old
                        .entries
                        .iter()
                        .filter(|o| o.is_dir)
                        .filter(|o| !listing.entries.iter().any(|n| n.is_dir && n.name == o.name))
                        .map(|o| join(path, &o.name))
                        .collect();
                    for dir in dropped {
                        self.forget(&dir);
                    }
                }
                self.dirs.insert(path.to_owned(), Dir::Listed(listing));
            }
            Err(reason) if reason == NO_SUCH_FILE && parent(path).is_some() => {
                if let Some((up, name)) = parent(path)
                    && let Some(Dir::Listed(listing)) = self.dirs.get_mut(&up)
                {
                    listing.entries.retain(|e| e.name != name);
                }
                self.forget(path);
            }
            Err(reason) => {
                self.dirs.insert(path.to_owned(), Dir::Failed(reason));
            }
        }
        self.layout();

        if let Some(old) = focus.as_deref()
            && self.find(old).is_none()
            && let Some(at) = nav.iter().position(|p| p == old)
        {
            let next = nav[at + 1..].iter().find_map(|p| self.find(p));
            let previous = || nav[..at].iter().rev().find_map(|p| self.find(p));
            if let Some(i) = next.or_else(previous) {
                self.set_focus(i);
            }
        }
        if had_selection
            && self.selected.is_empty()
            && let Some(i) = self.focus()
        {
            self.choose(i);
        }
        match (before, stamp(self)) {
            (Some(before), Some(after)) => after.0 == before.0 && after != before,
            _ => false,
        }
    }

    /// Whether the directory at `path` is still one the tree holds: `/`, or a directory
    /// its parent's kept listing has, or kept under a parent that has no listing.
    fn still_listed(&self, path: &str) -> bool {
        let Some((up, name)) = parent(path) else {
            return true;
        };
        match self.dirs.get(&up) {
            Some(Dir::Listed(listing)) => {
                listing.entries.iter().any(|e| e.is_dir && e.name == name)
            }
            _ => self.dirs.contains_key(path),
        }
    }

    /// Drops the directory at `path` and everything kept or expanded under it.
    fn forget(&mut self, path: &str) {
        let under = format!("{path}/");
        let gone = |p: &String| p == path || p.starts_with(&under);
        self.dirs.retain(|p, _| !gone(p));
        self.expanded.retain(|p| !gone(p));
    }

    /// The entry at the absolute `path`, if its parent's listing is kept.
    fn entry_of(&self, path: &str) -> Option<&Entry> {
        let (up, name) = parent(path)?;
        match self.dirs.get(&up) {
            Some(Dir::Listed(listing)) => listing.entries.iter().find(|e| e.name == name),
            _ => None,
        }
    }

    /// The kept listing of the directory at `path`, if it has been fetched.
    pub fn listing(&self, path: &str) -> Option<&Listing> {
        match self.dirs.get(path) {
            Some(Dir::Listed(listing)) => Some(listing),
            _ => None,
        }
    }

    /// Lays the rows out again in `sort`, with or without hidden entries.
    pub fn arrange(&mut self, sort: Sort, show_hidden: bool) {
        self.sort = sort;
        self.show_hidden = show_hidden;
        self.layout();
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Never: `/` is always there.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn row(&self, i: usize) -> Row<'_> {
        let laid = &self.rows[i];
        let line = match &laid.kind {
            Kind::Root => Line::Entry {
                name: "/",
                path: "/",
                entry: None,
                expanded: Some(self.expanded.contains("/")),
            },
            Kind::Entry { dir, index } => {
                let entry = self.entry_at(dir, *index);
                Line::Entry {
                    name: entry.map_or("", |e| e.name.as_str()),
                    path: &laid.path,
                    entry,
                    expanded: entry
                        .filter(|e| e.is_dir)
                        .map(|_| self.expanded.contains(&laid.path)),
                }
            }
            Kind::Loading => Line::Loading,
            Kind::Failed => match self.dirs.get(&laid.path) {
                Some(Dir::Failed(reason)) => Line::Failed(reason),
                _ => Line::Failed(""),
            },
        };
        Row {
            depth: laid.depth,
            line,
        }
    }

    /// The path of the entry on row `i`; `None` for a status row.
    pub fn path(&self, i: usize) -> Option<&str> {
        let laid = self.rows.get(i)?;
        matches!(laid.kind, Kind::Root | Kind::Entry { .. }).then_some(laid.path.as_str())
    }

    /// The Focused entry's row.
    pub fn focus(&self) -> Option<usize> {
        let focus = self.focus.as_deref()?;
        self.find(focus)
    }

    pub fn focused_path(&self) -> Option<&str> {
        self.focus.as_deref()
    }

    /// Whether the Focused entry is a directory (or `/`).
    fn focus_is_dir(&self) -> bool {
        self.focus().is_some_and(|i| self.is_dir(i))
    }

    pub fn is_selected(&self, i: usize) -> bool {
        self.path(i).is_some_and(|p| self.selected.contains(p))
    }

    /// The selected paths, in row order.
    pub fn selection(&self) -> Vec<&str> {
        (0..self.len())
            .filter(|i| self.is_selected(*i))
            .filter_map(|i| self.path(i))
            .collect()
    }

    /// The Current directory: the Focused entry when it is a directory, else its parent;
    /// the Initial remote folder (or the entry about to be focused) when nothing is.
    pub fn current_dir(&self) -> String {
        match self.focus.as_deref() {
            Some(path) if self.focus_is_dir() => path.to_owned(),
            Some(path) => parent(path).map_or_else(|| "/".to_owned(), |(dir, _)| dir),
            None => self.pending.clone().unwrap_or_else(|| self.initial.clone()),
        }
    }

    /// Whether the Focused entry moved on its own, so should be scrolled into view; once.
    pub fn take_reveal(&mut self) -> bool {
        std::mem::take(&mut self.reveal)
    }

    /// Carries out `command`; `now` is in seconds, for the typed prefix.
    pub fn apply(&mut self, command: Command, now: f64) -> Effect {
        let nav = self.nav();
        if nav.is_empty() {
            return Effect::None;
        }
        // Where the focus is among the entry rows; before it, the first.
        let at = self.focus().and_then(|f| nav.iter().position(|i| *i == f));
        let here = at.unwrap_or(0);
        let up = |n: usize| nav[here.saturating_sub(n)];
        let down = |n: usize| nav[(here + n).min(nav.len() - 1)];
        let start = |step: usize| if at.is_some() { step } else { 0 };
        match command {
            Command::Up => self.choose(up(start(1))),
            Command::Down => self.choose(down(start(1))),
            Command::Home => self.choose(nav[0]),
            Command::End => self.choose(nav[nav.len() - 1]),
            Command::PageUp(n) => self.choose(up(start(n.max(1)))),
            Command::PageDown(n) => self.choose(down(start(n.max(1)))),
            Command::ExtendUp => self.extend_to(up(start(1))),
            Command::ExtendDown => self.extend_to(down(start(1))),
            Command::FocusUp => self.set_focus(up(start(1))),
            Command::FocusDown => self.set_focus(down(start(1))),
            Command::Toggle => {
                if let Some(i) = self.focus() {
                    self.toggle(i);
                }
            }
            Command::SelectAll => self.select_all(),
            Command::Open => {
                let Some(i) = self.focus() else {
                    return Effect::None;
                };
                return self.open(i);
            }
            Command::Parent => self.focus_parent(),
            Command::Expand => return self.expand_or_enter(),
            Command::Collapse => self.collapse_or_leave(),
            Command::Type(c) => self.typed(c, now, &nav),
            Command::Click(i) => self.choose_row(i),
            Command::CtrlClick(i) => {
                if self.path(i).is_some() {
                    self.toggle(i);
                }
            }
            Command::ShiftClick(i) => {
                if self.path(i).is_some() {
                    self.extend_to(i);
                }
            }
            Command::DoubleClick(i) => {
                if self.path(i).is_some() {
                    self.choose(i);
                    return self.open(i);
                }
            }
            Command::Disclose(i) => {
                if self.path(i).is_some() && self.is_dir(i) {
                    return self.open(i);
                }
            }
        }
        Effect::None
    }

    fn entry_at(&self, dir: &str, index: usize) -> Option<&Entry> {
        match self.dirs.get(dir) {
            Some(Dir::Listed(listing)) => listing.entries.get(index),
            _ => None,
        }
    }

    fn is_dir(&self, i: usize) -> bool {
        match &self.rows[i].kind {
            Kind::Root => true,
            Kind::Entry { dir, index } => self.entry_at(dir, *index).is_some_and(|e| e.is_dir),
            _ => false,
        }
    }

    fn find(&self, path: &str) -> Option<usize> {
        (0..self.len()).find(|i| self.path(*i) == Some(path))
    }

    /// The rows the keys move over: every entry row, not the status rows.
    fn nav(&self) -> Vec<usize> {
        (0..self.len())
            .filter(|i| self.path(*i).is_some())
            .collect()
    }

    fn set_focus(&mut self, i: usize) {
        self.focus = self.path(i).map(str::to_owned);
        self.pending = None;
    }

    fn choose_row(&mut self, i: usize) {
        if self.path(i).is_some() {
            self.choose(i);
        }
    }

    /// Focus and Selection both to row `i`; `/` is never selected.
    fn choose(&mut self, i: usize) {
        self.set_focus(i);
        self.selected.clear();
        self.anchor = self.selectable(i);
        if let Some(path) = self.anchor.clone() {
            self.selected.insert(path);
        }
    }

    /// The path of row `i` if it can be selected: an entry, not `/`.
    fn selectable(&self, i: usize) -> Option<String> {
        match self.rows.get(i)?.kind {
            Kind::Entry { .. } => Some(self.rows[i].path.clone()),
            _ => None,
        }
    }

    /// The parent directory the Selection is in.
    fn selection_parent(&self) -> Option<String> {
        let first = self.selected.iter().next()?;
        parent(first).map(|(dir, _)| dir)
    }

    fn toggle(&mut self, i: usize) {
        self.set_focus(i);
        let Some(path) = self.selectable(i) else {
            return;
        };
        let parent_dir = parent(&path).map(|(dir, _)| dir);
        if self
            .selection_parent()
            .is_some_and(|p| Some(p) != parent_dir)
        {
            self.selected.clear();
        }
        if !self.selected.remove(&path) {
            self.selected.insert(path.clone());
        }
        self.anchor = Some(path);
    }

    /// Focus to row `i`, and the Selection to the range from the anchor to it among the
    /// anchor's siblings. A row under another parent starts a new Selection.
    fn extend_to(&mut self, i: usize) {
        let from = self
            .anchor
            .as_deref()
            .and_then(|path| self.find(path))
            .or(self.focus())
            .unwrap_or(i);
        let Some(to) = self.selectable(i) else {
            self.set_focus(i);
            return;
        };
        let sibling = |path: &str| parent(path).map(|(dir, _)| dir);
        let Some(anchor) = self.selectable(from) else {
            self.choose(i);
            return;
        };
        if sibling(&anchor) != sibling(&to) {
            self.choose(i);
            return;
        }
        self.set_focus(i);
        let (lo, hi) = (from.min(i), from.max(i));
        self.selected = (lo..=hi)
            .filter_map(|r| self.selectable(r))
            .filter(|p| sibling(p) == sibling(&anchor))
            .collect();
        self.anchor = Some(anchor);
    }

    fn select_all(&mut self) {
        let Some(focus) = self.focus.clone() else {
            return;
        };
        let Some((dir, _)) = parent(&focus) else {
            return;
        };
        self.selected = (0..self.len())
            .filter_map(|r| self.selectable(r))
            .filter(|p| parent(p).is_some_and(|(d, _)| d == dir))
            .collect();
    }

    fn open(&mut self, i: usize) -> Effect {
        if !self.is_dir(i) {
            return Effect::Preview;
        }
        let path = self.rows[i].path.clone();
        if self.expanded.contains(&path) {
            self.expanded.remove(&path);
            self.layout();
            Effect::None
        } else {
            self.expand(path)
        }
    }

    fn expand(&mut self, path: String) -> Effect {
        self.expanded.insert(path.clone());
        let effect = if self.dirs.contains_key(&path) {
            Effect::None
        } else {
            self.dirs.insert(path.clone(), Dir::Loading);
            Effect::Fetch(path)
        };
        self.layout();
        effect
    }

    fn expand_or_enter(&mut self) -> Effect {
        let Some(i) = self.focus() else {
            return Effect::None;
        };
        if !self.is_dir(i) {
            return Effect::None;
        }
        let path = self.rows[i].path.clone();
        if !self.expanded.contains(&path) {
            return self.expand(path);
        }
        if self.path(i + 1).is_some() && self.rows[i + 1].depth > self.rows[i].depth {
            self.choose(i + 1);
        }
        Effect::None
    }

    fn collapse_or_leave(&mut self) {
        let Some(i) = self.focus() else {
            return;
        };
        let path = self.rows[i].path.clone();
        if self.is_dir(i) && self.expanded.remove(&path) {
            self.layout();
        } else {
            self.focus_parent();
        }
    }

    fn focus_parent(&mut self) {
        let Some(focus) = self.focus.clone() else {
            return;
        };
        if let Some(i) = parent(&focus).and_then(|(dir, _)| self.find(&dir)) {
            self.choose(i);
        }
    }

    /// Jumps to the next visible name that starts with what was typed, as Folder view
    /// does.
    fn typed(&mut self, c: char, now: f64, nav: &[usize]) {
        if now - self.typed_at > TYPE_RESET {
            self.typed.clear();
        }
        self.typed_at = now;
        self.typed.push(c);
        let mut chars = self.typed.chars();
        let first = chars.next().unwrap_or(c);
        let repeat = chars.all(|d| d == first);
        let here = self.focus().and_then(|f| nav.iter().position(|i| *i == f));
        let (prefix, start) = if repeat {
            (first.to_lowercase().to_string(), here.map_or(0, |h| h + 1))
        } else {
            (self.typed.to_lowercase(), here.unwrap_or(0))
        };
        let len = nav.len();
        let hit = (0..len).map(|k| nav[(start + k) % len]).find(|i| {
            matches!(self.row(*i).line, Line::Entry { name, entry: Some(_), .. }
                if name.to_lowercase().starts_with(&prefix))
        });
        if let Some(i) = hit {
            self.choose(i);
        }
    }

    /// Lays the rows out from `/`, then keeps the focus on its path, or on the nearest
    /// ancestor still showing, and focuses a pending entry once it shows.
    fn layout(&mut self) {
        let mut rows = vec![Laid {
            depth: 0,
            path: "/".to_owned(),
            kind: Kind::Root,
        }];
        if self.expanded.contains("/") {
            self.children("/", 1, &mut rows);
        }
        self.rows = rows;

        if let Some(pending) = self.pending.clone()
            && let Some(i) = self.find(&pending)
        {
            self.choose(i);
            self.reveal = true;
        }
        if let Some(mut focus) = self.focus.clone() {
            while self.find(&focus).is_none() {
                match parent(&focus) {
                    Some((up, _)) => focus = up,
                    None => break,
                }
            }
            self.focus = Some(focus);
        }
        let shown: BTreeSet<String> = (0..self.len()).filter_map(|i| self.selectable(i)).collect();
        self.selected.retain(|p| shown.contains(p));
    }

    fn children(&self, dir: &str, depth: usize, rows: &mut Vec<Laid>) {
        let status = |kind| Laid {
            depth,
            path: dir.to_owned(),
            kind,
        };
        let listing = match self.dirs.get(dir) {
            None => return,
            Some(Dir::Loading) => return rows.push(status(Kind::Loading)),
            Some(Dir::Failed(_)) => return rows.push(status(Kind::Failed)),
            Some(Dir::Listed(listing)) => listing,
        };
        let entries = &listing.entries;
        let mut shown: Vec<usize> = (0..entries.len())
            .filter(|i| self.show_hidden || !entries[*i].name.starts_with('.'))
            .collect();
        let sort = self.sort;
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
        for index in shown {
            let entry = &entries[index];
            let path = join(dir, &entry.name);
            rows.push(Laid {
                depth,
                path: path.clone(),
                kind: Kind::Entry {
                    dir: dir.to_owned(),
                    index,
                },
            });
            if entry.is_dir && self.expanded.contains(&path) {
                self.children(&path, depth + 1, rows);
            }
        }
    }
}
