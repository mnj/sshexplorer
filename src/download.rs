//! Download: copying the Selection into a Session's Local folder, in the background.
//!
//! [`start`] runs one Download on the SSH runtime and returns a [`Download`] at once.
//! Discovery feeds up to four concurrent file copies. Totals become known at the end
//! of the walk. A file is copied flat into the Local folder.
//! A directory is recreated with its relative structure. An existing local directory is merged into;
//! only a file that would replace an existing local file is a conflict, which pauses the
//! Download on a [`Conflict`] until it is answered.
//!
//! Each file is written under a temporary name beside its target and renamed into place
//! when complete, so no partial file ever sits under the real name. Cancelling keeps the
//! finished files and removes all staged files. A remote failure on one entry (it
//! vanished, permission denied, a read error) is recorded and the Download goes on; a
//! local write error stops it.
//!
//! A symlinked directory is followed only when [`Options::follow_links`] is on; one that
//! is not is skipped as "symlink, not followed". A link to a file is copied as the file.
//! No local symlinks are made. [`Queue`] runs a Session's Downloads one after another.
//!
//! When the connection drops, a Download pauses: incomplete files are discarded, and
//! once [`Download::resume`] hands it the new connection it retries those files.

mod compressed;
mod discovery;
mod local;
mod transfer;

use discovery::Found;
use futures_util::{FutureExt as _, StreamExt as _, future::BoxFuture, stream::FuturesUnordered};
pub(crate) use local::valid_name;

use cap_std::fs::Dir;
use std::collections::VecDeque;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use tokio::sync::{oneshot, watch};

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
    /// Use remote zstd for large text files when the host permits it.
    pub compress: bool,
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
            compress: false,
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
    /// One active file, relative to the Local folder.
    pub current: Option<String>,
    /// Files currently scheduled or being copied.
    pub active_files: usize,
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
            active_files: 0,
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
    /// Local file or directory to show in the file manager.
    pub destination: Option<PathBuf>,
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
    cancel: watch::Sender<()>,
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
        self.cancel.send_replace(());
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
    let (cancel, cancelled) = watch::channel(());
    let (resume, resumed) = tokio::sync::mpsc::unbounded_channel();
    let run = Run {
        remote: session.remote(),
        resumed,
        generation: 0,
        progress: progress.clone(),
        unavailable: Arc::default(),
        sender,
        wake: session.waker(),
        active: Vec::new(),
        paths,
        policy: None,
        summary: Summary {
            destination: None,
            copied: 0,
            skipped: Vec::new(),
            failed: Vec::new(),
            end: End::Finished,
        },
    };
    session.spawn(run.go(job, cancelled));
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

const PARALLEL_FILES: usize = 4;
type Completed = (transfer::File, Result<transfer::Staged, transfer::Error>);
type Flights = FuturesUnordered<BoxFuture<'static, Completed>>;

/// The task's state.
struct Run {
    /// For a retry, copy only these failed paths, traversing their ancestors.
    paths: Vec<String>,
    remote: Remote,
    generation: u64,
    /// New connections, after drops.
    resumed: tokio::sync::mpsc::UnboundedReceiver<Remote>,
    progress: Arc<Mutex<Progress>>,
    unavailable: Arc<AtomicBool>,
    sender: mpsc::Sender<Event>,
    wake: Arc<dyn Fn() + Send + Sync>,
    active: Vec<String>,
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
    async fn go(mut self, job: Job, mut cancelled: watch::Receiver<()>) {
        let result = tokio::select! {
            biased;
            _ = cancelled.changed() => Err(Halt::Cancelled),
            result = self.run(&job) => result,
        };
        self.summary.end = match result {
            Ok(()) => End::Finished,
            Err(Halt::Cancelled) => End::Cancelled,
            Err(Halt::Local(reason)) => End::Stopped(reason),
        };
        self.update(|p| {
            p.current = None;
            p.active_files = 0;
            p.lost = false;
        });
        let _ = self.sender.send(Event::Finished(self.summary));
        (self.wake)();
    }

    async fn run(&mut self, job: &Job) -> Result<(), Halt> {
        let (tx, mut rx) = tokio::sync::mpsc::channel(PARALLEL_FILES);
        let paths = std::mem::take(&mut self.paths);
        let remote = self.remote.clone();
        let progress = self.progress.clone();
        let wake = self.wake.clone();
        let walk = async move {
            let (files, bytes) = discovery::walk(job, &paths, remote, tx).await?;
            if let Ok(mut p) = progress.lock() {
                p.files_total = Some(files);
                p.bytes_total = bytes;
            }
            wake();
            Ok(())
        };
        tokio::try_join!(walk, self.copy_items(job, &mut rx))?;
        Ok(())
    }

    async fn copy_items(
        &mut self,
        job: &Job,
        rx: &mut tokio::sync::mpsc::Receiver<Found>,
    ) -> Result<(), Halt> {
        let mut parents = Vec::new();
        let mut flights = Flights::new();
        loop {
            while flights.len() >= PARALLEL_FILES {
                self.finish_one(&mut flights, job).await?;
            }
            let found = tokio::select! {
                completed = flights.next(), if !flights.is_empty() => {
                    self.finish(completed.unwrap(), &mut flights, job).await?;
                    continue;
                }
                found = rx.recv() => found,
            };
            let item = match found {
                Some(Found::Item(item)) => item,
                Some(Found::Failed(note)) => {
                    self.summary.failed.push(note);
                    continue;
                }
                Some(Found::Skipped(note)) => {
                    self.summary.skipped.push(note);
                    continue;
                }
                Some(Found::Lost(generation, reply)) => {
                    self.drain(&mut flights, job).await?;
                    if self.generation == generation && !self.reconnect().await {
                        return Err(Halt::Cancelled);
                    }
                    let _ = reply.send((self.remote.clone(), self.generation));
                    continue;
                }
                None => break,
            };
            if parents.is_empty() {
                let root = &job.local_folder;
                std::fs::create_dir_all(root)
                    .map_err(|e| Halt::Local(format!("Cannot create {}: {e}", root.display())))?;
                let dir = Dir::open_ambient_dir(root, cap_std::ambient_authority())
                    .map_err(|e| Halt::Local(format!("Cannot open {}: {e}", root.display())))?;
                parents.push(Arc::new(dir));
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
            match item {
                Item::Dir { ref rel } => match local::child(parent, name) {
                    Ok(Some(dir)) => {
                        if self.summary.destination.is_none() {
                            self.summary.destination = Some(if job.entries.len() == 1 {
                                job.local_folder.join(&rel[0])
                            } else {
                                job.local_folder.clone()
                            });
                        }
                        parents.push(Arc::new(dir));
                    }
                    Ok(None) => self.fail(rel, "A local file is in the way"),
                    Err(e) => {
                        return Err(Halt::Local(format!("Cannot create {}: {e}", rel.join("/"))));
                    }
                },
                Item::File { remote, entry, rel } => {
                    let name = entry.name.clone();
                    let mut file = transfer::File {
                        dir: parent.clone(),
                        path: remote,
                        rel,
                        entry,
                        name,
                        overwrite: false,
                    };
                    // Publish earlier files before asking a conflict question. A cancel
                    // at the prompt keeps already-copied siblings in traversal order.
                    if !matches!(file.dir.symlink_metadata(&file.name), Err(e) if e.kind() == io::ErrorKind::NotFound)
                    {
                        if self.policy.is_none() {
                            self.drain(&mut flights, job).await?;
                        }
                        if !self.choose_target(&mut file, job).await? {
                            continue;
                        }
                    }

                    self.schedule(&flights, file, job.options);
                }
            }
        }
        self.drain(&mut flights, job).await
    }

    async fn choose_target(&mut self, file: &mut transfer::File, job: &Job) -> Result<bool, Halt> {
        let parent = file.rel[..file.rel.len() - 1]
            .iter()
            .fold(job.local_folder.clone(), |dir, name| dir.join(name));
        if let Some((name, overwrite)) = self
            .target(&file.dir, &parent, &file.rel, file.name.clone())
            .await?
        {
            file.name = name;
            file.overwrite = overwrite;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn schedule(&mut self, flights: &Flights, file: transfer::File, options: Options) {
        let (remote, progress, wake, unavailable) = (
            self.remote.clone(),
            self.progress.clone(),
            self.wake.clone(),
            self.unavailable.clone(),
        );
        self.active.push(file.local_name());
        self.update_active();
        flights.push(
            async move {
                let result = file
                    .fetch(&remote, options, progress, wake, &unavailable)
                    .await;
                (file, result)
            }
            .boxed(),
        );
    }

    async fn drain(&mut self, flights: &mut Flights, job: &Job) -> Result<(), Halt> {
        while !flights.is_empty() {
            self.finish_one(flights, job).await?;
        }
        Ok(())
    }

    async fn finish_one(&mut self, flights: &mut Flights, job: &Job) -> Result<(), Halt> {
        if let Some(completed) = flights.next().await {
            self.finish(completed, flights, job).await?;
        }
        Ok(())
    }

    async fn finish(
        &mut self,
        completed: Completed,
        flights: &mut Flights,
        job: &Job,
    ) -> Result<(), Halt> {
        let mut retry = Vec::new();
        let mut next = Some(completed);
        while let Some((mut file, result)) = next {
            let path = file.local_name();
            if let Some(index) = self.active.iter().position(|name| *name == path) {
                self.active.swap_remove(index);
            }
            self.update_active();
            match result {
                Ok(mut staged) => loop {
                    match staged.partial.publish(&file.name, file.overwrite) {
                        Ok(()) => {
                            staged.published();
                            if self.summary.destination.is_none() {
                                self.summary.destination =
                                    Some(if job.entries.len() == 1 && file.rel.len() == 1 {
                                        job.local_folder.join(&file.name)
                                    } else {
                                        job.local_folder.clone()
                                    });
                            }
                            self.summary.copied += 1;
                            self.update(|p| p.files_done += 1);
                            break;
                        }
                        Err(e) if !file.overwrite && e.kind() == io::ErrorKind::AlreadyExists => {
                            if !self.choose_target(&mut file, job).await? {
                                break;
                            }
                        }
                        Err(e) => {
                            return Err(Halt::Local(format!("Cannot write {}: {e}", file.name)));
                        }
                    }
                },
                Err(transfer::Error::Local(e)) => {
                    return Err(Halt::Local(format!("Cannot write {}: {e}", file.name)));
                }
                Err(transfer::Error::Remote(e)) => {
                    if self.remote.closed().await {
                        retry.push(file);
                    } else {
                        self.fail(&file.rel, e);
                    }
                }
                Err(transfer::Error::Lost) => retry.push(file),
            }
            if retry.is_empty() {
                break;
            }
            // Collect all results from this connection before pausing. Only files
            // that lost their read go back into the pool after reconnect.
            next = flights.next().await;
        }
        if !retry.is_empty() {
            if !self.reconnect().await {
                return Err(Halt::Cancelled);
            }
            for file in retry {
                self.schedule(flights, file, job.options);
            }
        }
        Ok(())
    }

    /// The connection dropped: waits, paused, for a live one. False when cancelled.
    async fn reconnect(&mut self) -> bool {
        let since = Instant::now();
        self.update(|p| p.lost = true);
        let resumed = loop {
            match self.resumed.recv().await {
                Some(remote) if remote.is_closed() => continue,
                Some(remote) => {
                    self.generation += 1;
                    self.remote = remote;
                    self.unavailable.store(false, Ordering::Relaxed);
                    break true;
                }
                None => break false,
            }
        };
        self.update(|p| {
            p.lost = false;
            p.paused += since.elapsed();
        });
        resumed
    }

    fn update_active(&self) {
        self.update(|p| {
            p.active_files = self.active.len();
            p.current = self.active.first().cloned();
        });
    }

    fn update(&self, change: impl FnOnce(&mut Progress)) {
        if let Ok(mut p) = self.progress.lock() {
            change(&mut p);
        }
        (self.wake)();
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
        self.update(|p| p.paused += asked.elapsed());
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
    last_destination: Option<PathBuf>,
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
                    if let Some(path) = &summary.destination {
                        self.last_destination = Some(path.clone());
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

    pub fn waiting_jobs(&self) -> impl Iterator<Item = &Job> {
        self.waiting.iter().map(|(job, _)| job)
    }

    pub fn remove_waiting(&mut self, index: usize) {
        self.waiting.remove(index);
    }

    pub fn current_job(&self) -> Option<&Job> {
        self.current_job.as_ref()
    }

    pub fn last_destination(&self) -> Option<&Path> {
        self.last_destination.as_deref()
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
