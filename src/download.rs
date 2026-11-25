//! Download: copying the Selection into a Session's Local folder, in the background.
//!
//! [`start`] runs one Download on the SSH runtime and returns a [`Download`] at once.
//! It first walks the Selection, so the totals are known ("counting…" until then), and
//! then copies file by file. A file is copied flat into the Local folder; a directory is
//! recreated with its relative structure. An existing local directory is merged into;
//! only a file that would replace an existing local file is a conflict, which pauses the
//! Download on a [`Conflict`] until it is answered.
//!
//! Each file is written under a temporary name beside its target and renamed into place
//! when complete, so no partial file ever sits under the real name. Cancelling keeps the
//! finished files and removes the one in flight. A remote failure on one entry (it
//! vanished, permission denied, a read error) is recorded and the Download goes on; a
//! local write error stops it.
//!
//! A symlinked directory is followed only when [`Options::follow_links`] is on; one that
//! is not is skipped as "symlink, not followed". A link to a file is copied as the file.
//! No local symlinks are made. [`Queue`] runs a Session's Downloads one after another.
//!
//! When the connection drops, a Download pauses: the file in flight is discarded, and
//! once [`Download::resume`] hands it the new connection it fetches that file again.

mod local;
pub(crate) use local::valid_name;

use cap_std::fs::Dir;
use std::collections::{BTreeSet, VecDeque};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use tokio::io::AsyncReadExt as _;
use tokio::sync::oneshot;

use crate::folder::join;
use crate::ssh::{self, Entry, Remote};

/// How much is read from the host at a time.
const CHUNK: usize = 256 * 1024;
/// The least time between two progress wakes of the frame.
const WAKE_EVERY: Duration = Duration::from_millis(100);
/// The reason an unfollowed symlinked directory is skipped.
pub const NOT_FOLLOWED: &str = "symlink, not followed";

/// The Download settings, read when a Download starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    /// Give each file the remote modification time.
    pub keep_modified: bool,
    /// Make a file executable locally when it is remotely. No effect on Windows.
    pub keep_executable: bool,
    /// Copy the contents of a symlinked directory; else skip it.
    pub follow_links: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            keep_modified: true,
            keep_executable: true,
            follow_links: false,
        }
    }
}

/// What one Download copies, and where to.
#[derive(Clone, Debug)]
pub struct Job {
    /// The remote directory the selected entries share.
    pub dir: String,
    /// The selected entries, as listed.
    pub entries: Vec<Entry>,
    /// The Local folder.
    pub local_folder: PathBuf,
    pub options: Options,
}

/// How far a Download has got.
#[derive(Clone, Debug)]
pub struct Progress {
    /// The file being copied, relative to the Local folder.
    pub current: Option<String>,
    pub files_done: usize,
    /// `None` while counting.
    pub files_total: Option<usize>,
    pub bytes_done: u64,
    /// Unknown while counting, or if the host omitted a size.
    pub bytes_total: Option<u64>,
    started: Instant,
    /// Time spent waiting for the operator or the connection, which the rate leaves out.
    paused: Duration,
    /// Paused because the connection dropped, until [`Download::resume`].
    pub lost: bool,
}

impl Progress {
    fn new() -> Progress {
        Progress {
            current: None,
            files_done: 0,
            files_total: None,
            bytes_done: 0,
            bytes_total: None,
            started: Instant::now(),
            paused: Duration::ZERO,
            lost: false,
        }
    }

    /// Bytes per second so far, not counting time spent on a conflict prompt.
    pub fn rate(&self) -> f64 {
        let busy = self.started.elapsed().saturating_sub(self.paused);
        if busy.as_secs_f64() < 0.05 {
            return 0.0;
        }
        self.bytes_done as f64 / busy.as_secs_f64()
    }
}

/// Something a running Download tells the frame.
pub enum Event {
    /// A file would replace an existing local file: answer it to go on.
    Conflict(Conflict),
    /// The Download is over. This is its last event.
    Finished(Summary),
}

/// A file that would replace an existing local file. The Download waits for the answer;
/// dropping it cancels the Download.
pub struct Conflict {
    /// The existing local file.
    pub path: PathBuf,
    /// The existing file's name.
    pub name: String,
    /// The automatic new name: `name (1).ext`, or the next number free.
    pub suggestion: String,
    answer: oneshot::Sender<(Choice, bool)>,
}

/// An answer to a [`Conflict`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice {
    Skip,
    Overwrite,
    /// Write the file under this name instead, in the same directory.
    Rename(String),
    /// Cancel the whole Download.
    Cancel,
}

impl Conflict {
    /// Answers the conflict. With `apply_to_all`, every later conflict of this Download
    /// gets the same answer without asking; a Rename then takes each file's automatic
    /// name. A Rename to a name edited by hand never applies to all.
    pub fn answer(self, choice: Choice, apply_to_all: bool) {
        let _ = self.answer.send((choice, apply_to_all));
    }
}

/// How a Download ended, and what it copied, skipped and could not copy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    pub copied: usize,
    pub skipped: Vec<Note>,
    pub failed: Vec<Note>,
    pub end: End,
}

impl Summary {
    /// Every file copied: nothing skipped or failed, and not cancelled or stopped.
    pub fn is_clean(&self) -> bool {
        self.skipped.is_empty() && self.failed.is_empty() && self.end == End::Finished
    }
}

/// An entry a Download skipped or could not copy, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    /// Relative to the Local folder, `/`-separated.
    pub path: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum End {
    /// Every entry was tried.
    Finished,
    /// The operator cancelled.
    Cancelled,
    /// A local write failed, for the reason given.
    Stopped(String),
}

/// A running Download. Dropping it cancels the Download.
pub struct Download {
    progress: Arc<Mutex<Progress>>,
    events: mpsc::Receiver<Event>,
    cancel: Arc<AtomicBool>,
    resume: tokio::sync::mpsc::UnboundedSender<Remote>,
}

impl Download {
    /// How far it has got.
    pub fn progress(&self) -> Progress {
        self.progress
            .lock()
            .map(|p| p.clone())
            .unwrap_or_else(|p| p.into_inner().clone())
    }

    /// The next event, if one has arrived. Never blocks.
    pub fn try_next(&self) -> Option<Event> {
        self.events.try_recv().ok()
    }

    /// Waits up to `timeout` for the next event: for tests, not the frame thread.
    pub fn recv_timeout(&self, timeout: Duration) -> Option<Event> {
        self.events.recv_timeout(timeout).ok()
    }

    /// Goes on over `session`'s connection after a drop paused it, fetching the file
    /// that was in flight again. The new connection is kept for a later pause too.
    pub fn resume(&self, session: &ssh::Session) {
        let _ = self.resume.send(session.remote());
    }

    /// Stops after the current chunk, removing the file in flight.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

impl Drop for Download {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// Starts `job` on `session`'s connection.
pub fn start(session: &ssh::Session, job: Job) -> Download {
    start_entries(session, job, Vec::new())
}

fn start_entries(session: &ssh::Session, job: Job, paths: Vec<String>) -> Download {
    let (sender, events) = mpsc::channel();
    let progress = Arc::new(Mutex::new(Progress::new()));
    let cancel = Arc::new(AtomicBool::new(false));
    let (resume, resumed) = tokio::sync::mpsc::unbounded_channel();
    let run = Run {
        remote: session.remote(),
        resumed,
        progress: progress.clone(),
        cancel: cancel.clone(),
        sender,
        wake: session.waker(),
        woken: Instant::now(),
        paths,
        policy: None,
        summary: Summary {
            copied: 0,
            skipped: Vec::new(),
            failed: Vec::new(),
            end: End::Finished,
        },
    };
    session.spawn(run.go(job));
    Download {
        progress,
        events,
        cancel,
        resume,
    }
}

/// `name` with ` (n)` before its extension: `notes (1).txt`, `archive (2).tar.gz`. A
/// leading dot does not start an extension.
pub fn numbered(name: &str, n: usize) -> String {
    let (stem, ext) = stem_and_extension(name);
    format!("{stem} ({n}){ext}")
}

fn stem_and_extension(name: &str) -> (&str, &str) {
    let stem_end = name
        .char_indices()
        .skip(1)
        .find(|(_, c)| *c == '.')
        .map_or(name.len(), |(i, _)| i);
    name.split_at(stem_end)
}

/// The first `name (n).ext` not taken in `dir`.
fn free_name(dir: &Dir, name: &str) -> io::Result<String> {
    let (mut stem, mut ext) = stem_and_extension(name);
    for n in 1.. {
        loop {
            let candidate = format!("{stem} ({n}){ext}");
            match dir.symlink_metadata(&candidate) {
                Ok(_) => break,
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(candidate),
                // A legal filename may already fill the filesystem's component limit.
                Err(e) if e.kind() == io::ErrorKind::InvalidFilename => {
                    let part = if stem.is_empty() { &mut ext } else { &mut stem };
                    let Some((end, _)) = part.char_indices().next_back() else {
                        return Err(e);
                    };
                    *part = &part[..end];
                }
                Err(e) => return Err(e),
            }
        }
    }
    unreachable!()
}

/// One thing to make locally, in the order made.
enum Item {
    Dir {
        rel: Vec<String>,
    },
    File {
        remote: String,
        rel: Vec<String>,
        entry: Entry,
    },
}

enum Transfer<'a> {
    Complete(local::Partial<'a>, u64),
    Failed,
    Lost,
}

/// The task's state.
struct Run {
    /// For a retry, copy only these failed paths, traversing their ancestors.
    paths: Vec<String>,
    remote: Remote,
    /// New connections, after drops.
    resumed: tokio::sync::mpsc::UnboundedReceiver<Remote>,
    progress: Arc<Mutex<Progress>>,
    cancel: Arc<AtomicBool>,
    sender: mpsc::Sender<Event>,
    wake: Arc<dyn Fn() + Send + Sync>,
    woken: Instant,
    /// An answer to apply to every later conflict.
    policy: Option<Choice>,
    summary: Summary,
}

/// Why copying a file stopped short.
enum Halt {
    /// The operator cancelled.
    Cancelled,
    /// A local write failed: stop the Download.
    Local(String),
}

impl Run {
    async fn go(mut self, job: Job) {
        let items = self.walk(&job).await;
        let files = items
            .iter()
            .filter(|i| matches!(i, Item::File { .. }))
            .count();
        let bytes = items.iter().try_fold(0u64, |sum, item| match item {
            Item::File { entry, .. } => sum.checked_add(entry.size?),
            Item::Dir { .. } => Some(sum),
        });
        self.update(true, |p| {
            p.files_total = Some(files);
            p.bytes_total = bytes;
        });
        self.summary.end = match self.copy_items(&job, items).await {
            Ok(()) => End::Finished,
            Err(Halt::Cancelled) => End::Cancelled,
            Err(Halt::Local(reason)) => End::Stopped(reason),
        };
        self.update(true, |p| p.current = None);
        let _ = self.sender.send(Event::Finished(self.summary));
        (self.wake)();
    }

    async fn copy_items(&mut self, job: &Job, items: Vec<Item>) -> Result<(), Halt> {
        if self.cancelled() {
            return Err(Halt::Cancelled);
        }
        if items.is_empty() {
            return Ok(());
        }
        let root = &job.local_folder;
        std::fs::create_dir_all(root)
            .map_err(|e| Halt::Local(format!("Cannot create {}: {e}", root.display())))?;
        let root_dir = Dir::open_ambient_dir(root, cap_std::ambient_authority())
            .map_err(|e| Halt::Local(format!("Cannot open {}: {e}", root.display())))?;
        // The walk is depth-first. A handle stays open until we leave its subtree.
        let mut parents = vec![root_dir];
        for item in items {
            if self.cancelled() {
                return Err(Halt::Cancelled);
            }
            let rel = match &item {
                Item::File { rel, .. } | Item::Dir { rel } => rel,
            };
            let depth = rel.len();
            parents.truncate(depth);
            if parents.len() < depth {
                // No destination handle exists for the blocked ancestor.
                continue;
            }
            let parent = &parents[depth - 1];
            let name = &rel[depth - 1];
            match &item {
                Item::Dir { .. } => match local::child(parent, name) {
                    Ok(Some(dir)) => parents.push(dir),
                    Ok(None) => self.fail(rel, "A local file is in the way"),
                    Err(e) => {
                        return Err(Halt::Local(format!("Cannot create {}: {e}", rel.join("/"))));
                    }
                },
                Item::File { remote, entry, .. } => {
                    self.copy(parent, job, remote, rel, entry).await?
                }
            }
        }
        Ok(())
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    /// The connection dropped: waits, paused, for a live one. False when cancelled.
    async fn reconnect(&mut self) -> bool {
        let since = Instant::now();
        self.update(true, |p| p.lost = true);
        let resumed = loop {
            if self.cancelled() {
                break false;
            }
            match tokio::time::timeout(Duration::from_millis(50), self.resumed.recv()).await {
                // One handed over before this drop may be dead already.
                Ok(Some(remote)) if remote.is_closed() => {}
                Ok(Some(remote)) => {
                    self.remote = remote;
                    break true;
                }
                Ok(None) => break false,
                Err(_) => {}
            }
        };
        self.update(true, |p| {
            p.lost = false;
            p.paused += since.elapsed();
        });
        resumed
    }

    /// Changes the progress, waking the frame when `now` or when it has not been woken
    /// for a while.
    fn update(&mut self, now: bool, change: impl FnOnce(&mut Progress)) {
        if let Ok(mut p) = self.progress.lock() {
            change(&mut p);
        }
        if now || self.woken.elapsed() >= WAKE_EVERY {
            self.woken = Instant::now();
            (self.wake)();
        }
    }

    fn fail(&mut self, rel: &[String], reason: impl Into<String>) {
        self.summary.failed.push(Note {
            path: rel.join("/"),
            reason: reason.into(),
        });
    }

    fn skip(&mut self, rel: &[String], reason: impl Into<String>) {
        self.summary.skipped.push(Note {
            path: rel.join("/"),
            reason: reason.into(),
        });
    }

    /// Everything to make, directories before what is in them. Each remote directory is
    /// walked once, even when links reach it twice.
    async fn walk(&mut self, job: &Job) -> Vec<Item> {
        let mut items = Vec::new();
        let mut seen = BTreeSet::new();
        let mut names = BTreeSet::new();
        let mut stack: Vec<(String, Vec<String>, Entry)> = job
            .entries
            .iter()
            .filter(|e| names.insert(e.name.clone()))
            .rev()
            .map(|e| (join(&job.dir, &e.name), vec![e.name.clone()], e.clone()))
            .collect();
        while let Some((remote, rel, mut entry)) = stack.pop() {
            if self.cancelled() {
                break;
            }
            if !self.paths.is_empty() {
                let path = rel.join("/");
                if !self.paths.iter().any(|failed| {
                    path == *failed || descendant(failed, &path) || descendant(&path, failed)
                }) {
                    continue;
                }
            }
            if !local::valid_name(&entry.name) {
                self.fail(&rel, "Invalid local filename");
                continue;
            }
            if !entry.is_dir {
                if (entry.size.is_none()
                    || (job.options.keep_modified && entry.modified.is_none())
                    || (job.options.keep_executable && entry.permissions.is_none()))
                    && self
                        .remote
                        .complete_file_metadata(&remote, &mut entry)
                        .await
                        .is_err()
                    && self.remote.closed().await
                {
                    stack.push((remote, rel, entry));
                    if !self.reconnect().await {
                        break;
                    }
                    continue;
                }
                items.push(Item::File { remote, rel, entry });
                continue;
            }
            if entry.link.is_some() && !job.options.follow_links {
                self.skip(&rel, NOT_FOLLOWED);
                continue;
            }
            let real = self
                .remote
                .canonicalize(remote.clone())
                .await
                .unwrap_or_else(|_| remote.clone());
            if !seen.insert(real.clone()) {
                continue;
            }
            match self.remote.list(remote.clone()).await {
                Err(_) if self.remote.closed().await => {
                    // Walked again on the new connection.
                    seen.remove(&real);
                    stack.push((remote, rel, entry));
                    if !self.reconnect().await {
                        break;
                    }
                }
                Ok(listing) => {
                    items.push(Item::Dir { rel: rel.clone() });
                    let dir = rel.join("/");
                    for path in &self.paths {
                        if let Some(rest) =
                            path.strip_prefix(&dir).and_then(|s| s.strip_prefix('/'))
                        {
                            let child = rest.split('/').next().unwrap_or(rest);
                            if !listing.entries.iter().any(|e| e.name == child) {
                                self.summary.failed.push(Note {
                                    path: path.clone(),
                                    reason: "No such file".into(),
                                });
                            }
                        }
                    }
                    let mut entries = listing.entries;
                    entries.sort_by(|a, b| b.name.cmp(&a.name));
                    for child in entries {
                        let mut child_rel = rel.clone();
                        child_rel.push(child.name.clone());
                        stack.push((join(&remote, &child.name), child_rel, child));
                    }
                }
                Err(reason) => self.fail(&rel, reason),
            }
        }
        items
    }

    async fn resolve(&mut self, dir: &Dir, path: &Path, name: &str) -> Result<Choice, Halt> {
        let suggestion = free_name(dir, name)
            .map_err(|e| Halt::Local(format!("Cannot find a free name: {e}")))?;
        match &self.policy {
            Some(Choice::Rename(_)) => return Ok(Choice::Rename(suggestion)),
            Some(choice) => return Ok(choice.clone()),
            None => {}
        }
        let (answer, reply) = oneshot::channel();
        let asked = Instant::now();
        let _ = self.sender.send(Event::Conflict(Conflict {
            path: path.to_path_buf(),
            name: name.to_owned(),
            suggestion: suggestion.clone(),
            answer,
        }));
        (self.wake)();
        let (choice, all) = reply.await.unwrap_or((Choice::Cancel, false));
        self.update(true, |p| p.paused += asked.elapsed());
        let hand_edited = matches!(&choice, Choice::Rename(n) if *n != suggestion);
        if all && !hand_edited && choice != Choice::Cancel {
            self.policy = Some(choice.clone());
        }
        Ok(choice)
    }

    /// Checks an initial name or one that another Download claimed during the read.
    async fn target(
        &mut self,
        dir: &Dir,
        parent_path: &Path,
        rel: &[String],
        mut name: String,
    ) -> Result<Option<(String, bool)>, Halt> {
        loop {
            match dir.symlink_metadata(&name) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Some((name, false))),
                Err(e) => return Err(Halt::Local(format!("Cannot inspect {name}: {e}"))),
                Ok(m) if m.is_dir() => {
                    self.fail(rel, "A local directory is in the way");
                    return Ok(None);
                }
                Ok(_) => match self.resolve(dir, &parent_path.join(&name), &name).await? {
                    Choice::Skip => {
                        self.skip(rel, "Already exists");
                        return Ok(None);
                    }
                    Choice::Overwrite => return Ok(Some((name, true))),
                    Choice::Rename(next) if local::valid_name(&next) => name = next,
                    Choice::Rename(_) => {
                        self.fail(rel, "Invalid local filename");
                        return Ok(None);
                    }
                    Choice::Cancel => return Err(Halt::Cancelled),
                },
            }
        }
    }

    async fn copy(
        &mut self,
        dir: &Dir,
        job: &Job,
        remote: &str,
        rel: &[String],
        entry: &Entry,
    ) -> Result<(), Halt> {
        let (name, parents) = rel.split_last().expect("a selected file has a name");
        // This full path is for display only. All writes use the directory handle.
        let parent_path = parents
            .iter()
            .fold(job.local_folder.clone(), |p, n| p.join(n));
        let Some((mut name, mut overwrite)) =
            self.target(dir, &parent_path, rel, name.clone()).await?
        else {
            return Ok(());
        };
        let mut shown = parents.to_vec();
        shown.push(name.clone());
        self.update(true, |p| p.current = Some(shown.join("/")));
        let (mut temp, written) = loop {
            match self
                .fetch(remote, dir, rel, &name, entry, job.options)
                .await?
            {
                Transfer::Complete(temp, written) => break (temp, written),
                Transfer::Failed => return Ok(()),
                Transfer::Lost if self.reconnect().await => {}
                Transfer::Lost => return Err(Halt::Cancelled),
            }
        };
        let result = loop {
            if self.cancelled() {
                break Err(Halt::Cancelled);
            }
            match temp.publish(&name, overwrite) {
                Ok(()) => {
                    self.summary.copied += 1;
                    self.update(true, |p| p.files_done += 1);
                    return Ok(());
                }
                Err(e) if !overwrite && e.kind() == io::ErrorKind::AlreadyExists => {
                    match self.target(dir, &parent_path, rel, name).await {
                        Ok(Some(target)) => {
                            (name, overwrite) = target;
                            *shown.last_mut().unwrap() = name.clone();
                            self.update(true, |p| p.current = Some(shown.join("/")));
                        }
                        Ok(None) => break Ok(()),
                        Err(halt) => break Err(halt),
                    }
                }
                Err(e) => break Err(Halt::Local(format!("Cannot write {name}: {e}"))),
            }
        };
        self.update(true, |p| p.bytes_done -= written);
        result
    }

    async fn fetch<'a>(
        &mut self,
        remote: &str,
        dir: &'a Dir,
        source: &[String],
        name: &str,
        entry: &Entry,
        options: Options,
    ) -> Result<Transfer<'a>, Halt> {
        let mut file = match self.remote.open(remote.to_owned()).await {
            Ok(file) => file,
            Err(_) if self.remote.closed().await => return Ok(Transfer::Lost),
            Err(reason) => {
                self.fail(source, reason);
                return Ok(Transfer::Failed);
            }
        };
        let local_error = |e: io::Error| Halt::Local(format!("Cannot write {name}: {e}"));
        let (temp, file_out) = local::Partial::create(dir).map_err(local_error)?;
        let mut out = tokio::fs::File::from_std(file_out);
        let mut buf = vec![0; CHUNK];
        let mut written = 0u64;
        let mut lost = false;
        let outcome = loop {
            if self.cancelled() {
                break Err(Halt::Cancelled);
            }
            let n = match file.read(&mut buf).await {
                Ok(0) => break Ok(true),
                Ok(n) => n,
                Err(_) if self.remote.closed().await => {
                    lost = true;
                    break Ok(false);
                }
                Err(e) => {
                    self.fail(source, e.to_string());
                    break Ok(false);
                }
            };
            if let Err(e) = tokio::io::AsyncWriteExt::write_all(&mut out, &buf[..n]).await {
                break Err(local_error(e));
            }
            written += n as u64;
            self.update(false, |p| p.bytes_done += n as u64);
        };
        drop(file);
        let outcome = match outcome {
            Ok(true) => tokio::io::AsyncWriteExt::flush(&mut out)
                .await
                .map(|()| true)
                .map_err(local_error),
            other => other,
        };
        let out = out.into_std().await;
        let outcome = outcome.and_then(|complete| {
            if complete {
                keep_metadata(&out, entry, options).map_err(local_error)?;
            }
            Ok(complete)
        });
        drop(out);
        if matches!(outcome, Ok(true)) {
            return Ok(Transfer::Complete(temp, written));
        }
        self.update(true, |p| p.bytes_done -= written);
        outcome.map(|_| {
            if lost {
                Transfer::Lost
            } else {
                Transfer::Failed
            }
        })
    }
}

/// Gives the written file the remote modification time and executable bit, as asked.
fn keep_metadata(out: &std::fs::File, entry: &Entry, options: Options) -> std::io::Result<()> {
    if options.keep_modified
        && let Some(secs) = entry.modified
    {
        out.set_modified(std::time::UNIX_EPOCH + Duration::from_secs(u64::from(secs)))?;
    }
    #[cfg(unix)]
    if options.keep_executable
        && let Some(mode) = entry.permissions
        && mode & 0o111 != 0
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mut perms = out.metadata()?.permissions();
        let now = perms.mode();
        // Executable wherever readable, as `chmod +x` under the umask would make it.
        perms.set_mode(now | ((now & 0o444) >> 2));
        out.set_permissions(perms)?;
    }
    Ok(())
}

fn descendant(path: &str, parent: &str) -> bool {
    path.strip_prefix(parent)
        .is_some_and(|suffix| suffix.starts_with('/'))
}

struct Report {
    summary: Summary,
    retry: Option<Job>,
}

/// A Session's Downloads: the one running, those waiting, in order, and how the last one
/// ended. Call [`Queue::poll`] every frame, whether or not the Session is on screen.
#[derive(Default)]
pub struct Queue {
    running: Option<Download>,
    waiting: VecDeque<(Job, Vec<String>)>,
    current_job: Option<Job>,
    reports: VecDeque<Report>,
    /// The running Download's question, until it is answered.
    pub conflict: Option<Conflict>,
    /// The last Download's summary, until it is dismissed.
    pub finished: Option<Summary>,
}

impl Queue {
    /// Adds `job`, starting it at once when nothing is running.
    pub fn push(&mut self, session: &ssh::Session, job: Job) {
        self.waiting.push_back((job, Vec::new()));
        self.poll(session);
    }

    /// Takes what the running Download has said, and starts the next one when it ends.
    /// Whether a Download ended in this call.
    pub fn poll(&mut self, session: &ssh::Session) -> bool {
        let mut ended = false;
        while let Some(event) = self.running.as_ref().and_then(Download::try_next) {
            match event {
                Event::Conflict(conflict) => self.conflict = Some(conflict),
                Event::Finished(summary) => {
                    self.running = None;
                    self.conflict = None;
                    let job = self.current_job.take();
                    if !summary.is_clean() {
                        let retry = job.filter(|_| !summary.failed.is_empty());
                        self.reports.push_back(Report {
                            summary: summary.clone(),
                            retry,
                        });
                    }
                    self.finished = Some(summary);
                    ended = true;
                }
            }
        }
        // The next waits while the connection is lost.
        if self.running.is_none()
            && session.lost().is_none()
            && let Some((job, paths)) = self.waiting.pop_front()
        {
            self.current_job = Some(job.clone());
            self.running = Some(start_entries(session, job, paths));
        }
        ended
    }

    pub fn summary(&self) -> Option<&Summary> {
        self.reports
            .front()
            .map(|r| &r.summary)
            .or(self.finished.as_ref())
    }

    pub fn can_retry(&self) -> bool {
        self.reports.front().is_some_and(|r| r.retry.is_some())
    }

    pub fn retry_failed(&mut self, session: &ssh::Session) {
        if let Some(report) = self.reports.front_mut()
            && let Some(job) = report.retry.take()
        {
            let paths = report
                .summary
                .failed
                .iter()
                .map(|note| note.path.clone())
                .collect();
            self.waiting.push_back((job, paths));
            self.poll(session);
        }
    }

    pub fn dismiss(&mut self) {
        self.reports.pop_front();
        if self.finished.as_ref().is_some_and(|s| !s.is_clean()) {
            self.finished = None;
        }
    }

    /// Hands the running Download `session`'s new connection after a drop, and starts
    /// the next one if none runs.
    pub fn resume(&mut self, session: &ssh::Session) {
        if let Some(download) = &self.running {
            download.resume(session);
        }
        self.poll(session);
    }

    /// The running Download.
    pub fn running(&self) -> Option<&Download> {
        self.running.as_ref()
    }

    /// Whether a Download is running or waiting.
    pub fn is_busy(&self) -> bool {
        self.running.is_some() || !self.waiting.is_empty()
    }

    /// How many Downloads wait behind the running one.
    pub fn queued(&self) -> usize {
        self.waiting.len()
    }

    /// Cancels the running Download; the next one then starts.
    pub fn cancel(&mut self) {
        if let Some(download) = &self.running {
            download.cancel();
        }
        if let Some(conflict) = self.conflict.take() {
            conflict.answer(Choice::Cancel, false);
        }
    }
}
