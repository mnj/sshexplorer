//! The Matches of a Search as a list the operator works with: no egui in here.
//!
//! The Matches keep the order they arrived in. The Focused Match and the Selection are
//! kept by path, so a rerun of the same Search ([`Matches::rerun`]) restores them where
//! they still match. Like every Selection, the Matches' Selection holds entries that
//! share one parent directory: selecting a Match under another parent starts a new one.

use std::collections::BTreeSet;

use crate::folder::{self, Command};
use crate::search::{Ended, Match, Query};
use crate::ssh::Entry;

/// What the Session does after a command on the Matches.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Effect {
    None,
    /// Open this Match, which ends the Search.
    Open(Match),
}

/// A Search's Matches, and where the keyboard is in them.
#[derive(Debug, Clone)]
pub struct Matches {
    query: Query,
    items: Vec<Match>,
    /// The Focused Match's path. When it is not among the Matches, the first is focused.
    focus: Option<String>,
    selected: BTreeSet<String>,
    anchor: Option<String>,
    ended: Option<Ended>,
}

impl Matches {
    /// No Matches yet, for a Search of `query` that has just started.
    pub fn new(query: Query) -> Matches {
        Matches {
            query,
            items: Vec::new(),
            focus: None,
            selected: BTreeSet::new(),
            anchor: None,
            ended: None,
        }
    }

    /// The same Search to run again: no Matches yet, with the Focused and selected paths
    /// kept, so they come back as their Matches arrive.
    pub fn rerun(&self) -> Matches {
        Matches {
            items: Vec::new(),
            ended: None,
            ..self.clone()
        }
    }

    pub fn query(&self) -> &Query {
        &self.query
    }

    /// Adds Matches as they arrive.
    pub fn found(&mut self, batch: Vec<Match>) {
        self.items.extend(batch);
    }

    /// The Search has ended. Kept paths that did not come back are dropped.
    pub fn end(&mut self, ended: Ended) {
        self.ended = Some(ended);
        let paths: BTreeSet<&str> = self.items.iter().map(|m| m.path.as_str()).collect();
        self.selected.retain(|p| paths.contains(p.as_str()));
        if self.focus.as_deref().is_some_and(|p| !paths.contains(p)) {
            self.focus = None;
        }
    }

    /// How the Search ended; `None` while it runs.
    pub fn ended(&self) -> Option<Ended> {
        self.ended
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn get(&self, i: usize) -> &Match {
        &self.items[i]
    }

    /// The Focused Match's index: the kept path's, else the first.
    pub fn focus(&self) -> Option<usize> {
        if self.items.is_empty() {
            return None;
        }
        Some(
            self.focus
                .as_deref()
                .and_then(|p| self.find(p))
                .unwrap_or(0),
        )
    }

    pub fn is_selected(&self, i: usize) -> bool {
        self.selected.contains(&self.items[i].path)
    }

    /// The Focused Match's absolute path.
    pub fn absolute(&self, m: &Match) -> String {
        folder::join(&self.query.start, &m.path)
    }

    /// The Selection: the absolute directory its Matches share, and them as entries of
    /// it. `None` when nothing is selected.
    pub fn selection(&self) -> Option<(String, Vec<Entry>)> {
        let chosen: Vec<&Match> = self
            .items
            .iter()
            .filter(|m| self.selected.contains(&m.path))
            .collect();
        let first = chosen.first()?;
        let (dir, _) = folder::parent(&self.absolute(first))?;
        let entries = chosen
            .iter()
            .map(|m| Entry {
                name: name(&m.path).to_owned(),
                is_dir: m.is_dir,
                ..Entry::default()
            })
            .collect();
        Some((dir, entries))
    }

    pub fn apply(&mut self, command: Command) -> Effect {
        let Some(focus) = self.focus() else {
            return Effect::None;
        };
        let last = self.items.len() - 1;
        let up = |n: usize| focus.saturating_sub(n);
        let down = |n: usize| (focus + n).min(last);
        match command {
            Command::Up => self.choose(up(1)),
            Command::Down => self.choose(down(1)),
            Command::Home => self.choose(0),
            Command::End => self.choose(last),
            Command::PageUp(n) => self.choose(up(n.max(1))),
            Command::PageDown(n) => self.choose(down(n.max(1))),
            Command::ExtendUp => self.extend_to(up(1)),
            Command::ExtendDown => self.extend_to(down(1)),
            Command::FocusUp => self.focus = Some(self.items[up(1)].path.clone()),
            Command::FocusDown => self.focus = Some(self.items[down(1)].path.clone()),
            Command::Toggle => self.toggle(focus),
            Command::SelectAll => {
                let dir = parent(&self.items[focus].path).to_owned();
                self.selected = self
                    .items
                    .iter()
                    .filter(|m| parent(&m.path) == dir)
                    .map(|m| m.path.clone())
                    .collect();
            }
            Command::Open => return Effect::Open(self.items[focus].clone()),
            Command::Click(i) => self.choose(i.min(last)),
            Command::CtrlClick(i) => self.toggle(i.min(last)),
            Command::ShiftClick(i) => self.extend_to(i.min(last)),
            Command::DoubleClick(i) => {
                self.choose(i.min(last));
                return Effect::Open(self.items[i.min(last)].clone());
            }
            Command::Parent | Command::Expand | Command::Collapse | Command::Type(_) => {}
        }
        Effect::None
    }

    fn find(&self, path: &str) -> Option<usize> {
        self.items.iter().position(|m| m.path == path)
    }

    fn choose(&mut self, i: usize) {
        let path = self.items[i].path.clone();
        self.focus = Some(path.clone());
        self.anchor = Some(path.clone());
        self.selected = BTreeSet::from([path]);
    }

    fn toggle(&mut self, i: usize) {
        let path = self.items[i].path.clone();
        self.focus = Some(path.clone());
        self.anchor = Some(path.clone());
        if !self.selected.remove(&path) {
            self.keep_parent(&path);
            self.selected.insert(path);
        }
    }

    /// Focus to `i`, and the Selection to the Matches from the anchor to it that share
    /// its parent.
    fn extend_to(&mut self, i: usize) {
        let from = self
            .anchor
            .as_deref()
            .and_then(|p| self.find(p))
            .unwrap_or(i);
        let path = self.items[i].path.clone();
        let dir = parent(&path).to_owned();
        let (lo, hi) = (from.min(i), from.max(i));
        self.selected = self.items[lo..=hi]
            .iter()
            .filter(|m| parent(&m.path) == dir)
            .map(|m| m.path.clone())
            .collect();
        if self.anchor.is_none() {
            self.anchor = Some(self.items[from].path.clone());
        }
        self.focus = Some(path);
    }

    /// Drops selected Matches whose parent is not `path`'s.
    fn keep_parent(&mut self, path: &str) {
        let dir = parent(path);
        self.selected.retain(|p| parent(p) == dir);
    }
}

/// A relative path's directory part: empty at the start.
fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

/// A relative path's last part.
pub fn name(path: &str) -> &str {
    path.rsplit_once('/').map_or(path, |(_, name)| name)
}
