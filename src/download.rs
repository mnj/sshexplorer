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

use std::collections::{BTreeSet, VecDeque};
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
    /// `None` while counting.
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
    let stem_end = name
        .char_indices()
        .skip(1)
        .find(|(_, c)| *c == '.')
        .map_or(name.len(), |(i, _)| i);
    let (stem, ext) = name.split_at(stem_end);
    format!("{stem} ({n}){ext}")
}

/// The first `name (n).ext` not taken in `dir`.
fn free_name(dir: &Path, name: &str) -> String {
    (1..)
        .map(|n| numbered(name, n))
        .find(|candidate| std::fs::symlink_metadata(dir.join(candidate)).is_err())
        .unwrap_or_else(|| name.to_owned())
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

/// The task's state.
struct Run {
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
        let bytes = items
            .iter()
            .map(|i| match i {
                Item::File { entry, .. } => entry.size.unwrap_or(0),
                Item::Dir { .. } => 0,
            })
            .sum();
        self.update(true, |p| {
            p.files_total = Some(files);
            p.bytes_total = Some(bytes);
        });
        for item in items {
            if self.cancelled() {
                self.summary.end = End::Cancelled;
                break;
            }
            let result = match item {
                Item::Dir { rel } => self.make_dir(&job.local_folder, &rel),
                Item::File { remote, rel, entry } => {
                    self.copy(&job.local_folder, &remote, &rel, &entry, job.options)
                        .await
                }
            };
            match result {
                Ok(()) => {}
                Err(Halt::Cancelled) => {
                    self.summary.end = End::Cancelled;
                    break;
                }
                Err(Halt::Local(reason)) => {
                    self.summary.end = End::Stopped(reason);
                    break;
                }
            }
        }
        self.update(true, |p| p.current = None);
        let _ = self.sender.send(Event::Finished(self.summary));
        (self.wake)();
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
        while let Some((remote, rel, entry)) = stack.pop() {
            if self.cancelled() {
                break;
            }
            if !entry.is_dir {
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

    fn make_dir(&mut self, local: &Path, rel: &[String]) -> Result<(), Halt> {
        let path = rel.iter().fold(local.to_path_buf(), |p, n| p.join(n));
        match std::fs::symlink_metadata(&path) {
            Ok(m) if m.is_dir() => Ok(()),
            Ok(_) => {
                self.fail(rel, "A local file is in the way");
                Ok(())
            }
            Err(_) => std::fs::create_dir_all(&path)
                .map_err(|e| Halt::Local(format!("Cannot create {}: {e}", path.display()))),
        }
    }

    /// Asks about, or applies the standing answer to, the existing file `path`.
    async fn resolve(&mut self, path: &Path) -> Choice {
        let dir = path.parent().unwrap_or(Path::new(""));
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let suggestion = free_name(dir, &name);
        match &self.policy {
            Some(Choice::Rename(_)) => return Choice::Rename(suggestion),
            Some(choice) => return choice.clone(),
            None => {}
        }
        let (answer, reply) = oneshot::channel();
        let asked = Instant::now();
        let _ = self.sender.send(Event::Conflict(Conflict {
            path: path.to_path_buf(),
            name,
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
        choice
    }

    async fn copy(
        &mut self,
        local: &Path,
        remote: &str,
        rel: &[String],
        entry: &Entry,
        options: Options,
    ) -> Result<(), Halt> {
        let mut target = rel.iter().fold(local.to_path_buf(), |p, n| p.join(n));
        let mut shown = rel.to_vec();
        let mut overwrite = false;
        loop {
            match std::fs::symlink_metadata(&target) {
                Err(_) => break,
                Ok(m) if m.is_dir() => {
                    self.fail(&shown, "A local directory is in the way");
                    return Ok(());
                }
                Ok(_) if overwrite => break,
                Ok(_) => match self.resolve(&target).await {
                    Choice::Skip => {
                        self.skip(&shown, "Already exists");
                        return Ok(());
                    }
                    Choice::Overwrite => overwrite = true,
                    Choice::Rename(name) => {
                        target.set_file_name(&name);
                        if let Some(last) = shown.last_mut() {
                            *last = name;
                        }
                    }
                    Choice::Cancel => return Err(Halt::Cancelled),
                },
            }
        }
        let shown_text = shown.join("/");
        self.update(true, |p| p.current = Some(shown_text));
        loop {
            match self.fetch(remote, &target, &shown, entry, options).await {
                Some(result) => return result,
                None if self.reconnect().await => {}
                None => return Err(Halt::Cancelled),
            }
        }
    }

    /// Copies the remote file to `target` through its temp file. `None` when the
    /// connection dropped: the temp file is gone and its bytes no longer count.
    async fn fetch(
        &mut self,
        remote: &str,
        target: &Path,
        shown: &[String],
        entry: &Entry,
        options: Options,
    ) -> Option<Result<(), Halt>> {
        let mut file = match self.remote.open(remote.to_owned()).await {
            Ok(file) => file,
            Err(_) if self.remote.closed().await => return None,
            Err(reason) => {
                self.fail(shown, reason);
                return Some(Ok(()));
            }
        };
        let name = target
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let temp = target.with_file_name(format!(".{name}.sshexplorer-part"));
        let local_error = |e: std::io::Error| Halt::Local(format!("Cannot write {name}: {e}"));
        let mut out = match tokio::fs::File::create(&temp).await {
            Ok(out) => out,
            Err(e) => return Some(Err(local_error(e))),
        };
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
                    self.fail(shown, e.to_string());
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
        let placed = match outcome {
            Ok(true) => keep_metadata(&out, entry, options)
                .and_then(|()| {
                    drop(out);
                    std::fs::rename(&temp, target)
                })
                .map(|()| true)
                .map_err(local_error),
            other => {
                drop(out);
                other
            }
        };
        if let Ok(true) = placed {
            self.summary.copied += 1;
            self.update(true, |p| p.files_done += 1);
            return Some(Ok(()));
        }
        let _ = std::fs::remove_file(&temp);
        // The bytes of a file not kept no longer count.
        self.update(true, |p| p.bytes_done -= written.min(p.bytes_done));
        (!lost).then(|| placed.map(|_| ()))
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

/// A Session's Downloads: the one running, those waiting, in order, and how the last one
/// ended. Call [`Queue::poll`] every frame, whether or not the Session is on screen.
#[derive(Default)]
pub struct Queue {
    running: Option<Download>,
    waiting: VecDeque<Job>,
    /// The running Download's question, until it is answered.
    pub conflict: Option<Conflict>,
    /// The last Download's summary, until it is dismissed.
    pub finished: Option<Summary>,
}

impl Queue {
    /// Adds `job`, starting it at once when nothing is running.
    pub fn push(&mut self, session: &ssh::Session, job: Job) {
        self.waiting.push_back(job);
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
                    self.finished = Some(summary);
                    ended = true;
                }
            }
        }
        // The next waits while the connection is lost.
        if self.running.is_none()
            && session.lost().is_none()
            && let Some(job) = self.waiting.pop_front()
        {
            self.running = Some(start(session, job));
        }
        ended
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
