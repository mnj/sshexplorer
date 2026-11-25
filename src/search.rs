//! Search: names on the host at and below a directory, found by a remote `find` run on
//! the Session's own connection, off the egui frame thread.
//!
//! [`start`] returns a [`Search`] at once; the Matches arrive on it as [`Event`]s, in
//! batches as the host prints them, each followed by a call to the wake function, and
//! the last event is [`Event::Ended`]. [`Search::stop`], or dropping the Search, stops
//! it: the `find` on the host is killed by its pid, which the command prints first, since
//! SSH signals are not delivered by every server and closing the channel does not stop a
//! `find` that has nothing more to print.
//!
//! The text matches names only, case-insensitively: as a substring, or as a glob over
//! the whole name when it holds `*` or `?`. Dot-names match like any other. The command
//! is plain POSIX `sh` and `find` (`-iname`, `-type`, `-path`, `-prune`, `-exec`), so it
//! runs on GNU, busybox, BSD and macOS hosts alike, under whatever login shell the
//! account has: every argument is passed octal-escaped and decoded by `printf` on the
//! host, so nothing the operator types is ever read by a shell.
//!
//! "Follow symlinked directories" is done here rather than by `find -L`: each `find`
//! walks real directories only and reports the symlinks to directories it meets, and
//! the Search then walks each link's target that no walk so far has covered, by its
//! canonical path, pruning directories already walked. So no directory is searched
//! twice, and a symlink whose name matches is a Match whether it is followed or not.

use std::sync::{Arc, mpsc};
use std::time::Duration;

use tokio::sync::watch;

use crate::ssh::{self, Output, Remote};

/// What to search for, and where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
    /// The absolute directory the Search starts at: the Current directory.
    pub start: String,
    /// What the operator typed.
    pub text: String,
    /// "Follow symlinked directories".
    pub follow: bool,
    /// The match cap: the Search ends once it has found this many.
    pub cap: usize,
}

/// An entry whose name a Search found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    /// The path relative to the start, `/`-separated.
    pub path: String,
    pub is_dir: bool,
    /// A symlink to a directory: Download follows it only when asked.
    pub link: bool,
}

/// What a Search says, in order. The last is [`Event::Ended`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Found(Vec<Match>),
    Ended(Ended),
}

/// How a Search ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ended {
    pub how: How,
    /// Directories that could not be read and were skipped.
    pub unreadable: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum How {
    /// Every directory was searched.
    Finished,
    /// Stopped before the end: Stop, Escape, a new Search, or the Session closing.
    Stopped,
    /// Stopped at the match cap.
    Capped,
    /// The host cannot run the Search.
    Unavailable,
    /// The connection dropped.
    Lost,
}

/// A running Search. Dropping it stops it.
pub struct Search {
    events: mpsc::Receiver<Event>,
    stop: watch::Sender<bool>,
}

impl Search {
    /// The next event, if one has arrived. Never blocks.
    pub fn try_next(&self) -> Option<Event> {
        self.events.try_recv().ok()
    }

    /// Waits up to `timeout` for the next event: for tests, not the frame thread.
    pub fn recv_timeout(&self, timeout: Duration) -> Option<Event> {
        self.events.recv_timeout(timeout).ok()
    }

    /// Stops the Search. The Matches found so far stay; [`How::Stopped`] follows.
    pub fn stop(&self) {
        let _ = self.stop.send(true);
    }
}

/// Starts a Search on `session`'s connection.
pub fn start(session: &ssh::Session, query: Query) -> Search {
    let (sender, events) = mpsc::channel();
    let (stop, stopped) = watch::channel(false);
    let out = Out {
        sender,
        wake: session.waker(),
    };
    session.spawn(run(session.remote(), query, out, stopped));
    Search { events, stop }
}

/// The text as a `find -iname` pattern: a glob as given when it holds `*` or `?`,
/// otherwise a substring, with the glob characters it holds escaped.
pub fn pattern(text: &str) -> String {
    if text.contains(['*', '?']) {
        return text.to_owned();
    }
    let mut out = String::from("*");
    for c in text.chars() {
        if matches!(c, '[' | ']' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('*');
    out
}

/// The script the host's `sh` runs: decode every argument, print the shell's pid
/// (which `exec` hands on to `find`), enter the start directory, and run `find` there.
/// It holds no single quote or backslash, so it survives any login shell's quoting.
const SCRIPT: &str = "n=$#; while [ \"$n\" -gt 0 ]; do a=$(printf \"$1.\"); \
set -- \"$@\" \"${a%.}\"; shift; n=$((n-1)); done; echo $$; \
cd \"$1\" 2>/dev/null || exit 3; LC_ALL=C; export LC_ALL;";

/// The exit status the script gives when it cannot enter the start directory.
const NO_START: u32 = 3;

/// Choose an unbuffered native printer when available. Each arm refers to numbered
/// shell arguments: host paths and patterns only ever appear in the encoded argument list.
fn command(dir: &str, fast: &[String], portable: &[String]) -> String {
    let mut args = vec![dir];
    let mut line =
        format!("exec sh -c '{SCRIPT} if stdbuf -o0 find . -prune -printf \"\" >/dev/null 2>&1; ");
    for (prefix, expression) in [
        ("then exec stdbuf -o0 find .", fast),
        ("; else exec find .", portable),
    ] {
        line.push_str(prefix);
        for token in expression {
            let index = match args.iter().position(|&a| a == token) {
                Some(i) => i + 1,
                None => {
                    args.push(token);
                    args.len()
                }
            };
            line.push_str(&format!(" \"${{{index}}}\""));
        }
    }
    line.push_str("; fi' sh");
    for arg in args {
        line.push_str(" '");
        line.push_str(&escape(arg));
        line.push('\'');
    }
    line
}

/// `text` with every byte but ASCII letters and digits written as a `\ooo` octal
/// escape, which `printf` decodes.
fn escape(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() {
            out.push(b as char);
        } else {
            out.push_str(&format!("\\{b:03o}"));
        }
    }
    out
}

/// `find`'s expression: directories in `prunes` (relative, `./…`) are not entered, a
/// matching name prints its kind and path (`D` for a symlink to a directory), and when
/// following, every symlink to a directory is printed too, `L` when its name matches and
/// `l` when not.
fn expression(pattern: &str, follow: bool, prunes: &[String], native: bool) -> Vec<String> {
    let print = |kind: &'static str, native_format: &'static str| {
        if native {
            vec!["-printf", native_format]
        } else {
            vec!["-exec", "printf", kind, "{}", ";"]
        }
    };
    let is_dir_link = ["-type", "l", "-exec", "test", "-d", "{}", ";"];
    let mut args: Vec<&str> = Vec::new();
    if !prunes.is_empty() {
        args.push("(");
        for (i, prune) in prunes.iter().enumerate() {
            if i > 0 {
                args.push("-o");
            }
            args.extend(["-path", prune.as_str()]);
        }
        args.extend([")", "-prune", "-o"]);
    }
    if follow {
        args.extend(is_dir_link);
        args.extend(["(", "-iname", pattern]);
        args.extend(print("L%s\\0", "L%p\\0"));
        args.push("-o");
        args.extend(print("l%s\\0", "l%p\\0"));
        args.extend([")", "-o"]);
    }
    args.extend(["-iname", pattern, "(", "-type", "d"]);
    args.extend(print("d%s\\0", "d%p\\0"));
    args.push("-o");
    args.extend(is_dir_link);
    args.extend(print("D%s\\0", "D%p\\0"));
    args.push("-o");
    args.extend(print("f%s\\0", "f%p\\0"));
    args.push(")");
    args.into_iter().map(str::to_owned).collect()
}

/// `text` as a `find -path` pattern that matches only itself.
fn literal(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if matches!(c, '*' | '?' | '[' | ']' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// The sending end of a Search.
struct Out {
    sender: mpsc::Sender<Event>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl Out {
    fn send(&self, event: Event) {
        let _ = self.sender.send(event);
        (self.wake)();
    }
}

/// One directory to walk: where it is, and how its Matches are shown.
struct Root {
    /// The absolute path to enter.
    dir: String,
    /// What its Matches' paths start with: empty for the start, else the link's path.
    shown: String,
}

/// What one `find` said.
enum Walk {
    /// It ran to the end; `status` is its exit status.
    Done {
        status: Option<u32>,
        pid: bool,
    },
    Stopped,
    Capped,
}

async fn run(remote: Remote, query: Query, out: Out, stopped: watch::Receiver<bool>) {
    // A `find` that ended without an exit status, or a command that could not start,
    // may mean the connection dropped.
    let mut suspect = false;
    let mut ended = search(&remote, query, &out, stopped, &mut suspect).await;
    let ran_out = matches!(ended.how, How::Finished | How::Unavailable);
    if ran_out && (remote.is_closed() || (suspect && remote.closed().await)) {
        ended.how = How::Lost;
    }
    out.send(Event::Ended(ended));
}

async fn search(
    remote: &Remote,
    query: Query,
    out: &Out,
    mut stopped: watch::Receiver<bool>,
    suspect: &mut bool,
) -> Ended {
    let pattern = pattern(&query.text);
    let start = remote
        .canonicalize(query.start.clone())
        .await
        .unwrap_or_else(|_| query.start.clone());
    let mut visited = vec![start.clone()];
    let mut roots = std::collections::VecDeque::from([Root {
        dir: start,
        shown: String::new(),
    }]);
    let mut found = 0;
    let mut unreadable = 0;
    let mut first = true;
    let end = |how, unreadable| Ended { how, unreadable };
    while let Some(root) = roots.pop_front() {
        if stopped.has_changed().unwrap_or(true) {
            return end(How::Stopped, unreadable);
        }
        let prunes: Vec<String> = visited
            .iter()
            .filter_map(|v| below(&root.dir, v))
            .map(|rel| literal(&format!("./{rel}")))
            .collect();
        let fast = expression(&pattern, query.follow, &prunes, true);
        let portable = expression(&pattern, query.follow, &prunes, false);
        let exec = match remote.exec(&command(&root.dir, &fast, &portable)).await {
            Ok(exec) => exec,
            Err(_) if first => {
                *suspect = true;
                return end(How::Unavailable, 0);
            }
            Err(_) => {
                *suspect = true;
                unreadable += 1;
                continue;
            }
        };
        let mut links = Vec::new();
        let walk = walk(
            remote,
            exec,
            &root,
            &mut stopped,
            out,
            Counts {
                found: &mut found,
                cap: query.cap,
                unreadable: &mut unreadable,
            },
            &mut links,
        )
        .await;
        match walk {
            Walk::Stopped => return end(How::Stopped, unreadable),
            Walk::Capped => return end(How::Capped, unreadable),
            Walk::Done { status, pid } => {
                *suspect |= status.is_none();
                let missing = matches!(status, Some(126 | 127)) || !pid;
                if first && missing {
                    return end(How::Unavailable, 0);
                }
                if status == Some(NO_START) || (!first && missing) {
                    unreadable += 1;
                }
            }
        }
        first = false;
        for (path, shown) in links {
            let Ok(real) = remote.canonicalize(path).await else {
                unreadable += 1;
                continue;
            };
            if visited
                .iter()
                .any(|v| v == &real || below(v, &real).is_some())
            {
                continue;
            }
            visited.push(real.clone());
            roots.push_back(Root { dir: real, shown });
        }
    }
    end(How::Finished, unreadable)
}

/// `path` relative to `dir`, when it is below it.
fn below(dir: &str, path: &str) -> Option<String> {
    let prefix = if dir.ends_with('/') {
        dir.to_owned()
    } else {
        format!("{dir}/")
    };
    path.strip_prefix(&prefix)
        .filter(|rest| !rest.is_empty())
        .map(str::to_owned)
}

struct Counts<'a> {
    found: &'a mut usize,
    cap: usize,
    unreadable: &'a mut usize,
}

/// Reads one `find` to its end, sending its Matches and collecting the symlinked
/// directories it reports into `links` (absolute path, shown path).
async fn walk(
    remote: &Remote,
    mut exec: ssh::Exec,
    root: &Root,
    stopped: &mut watch::Receiver<bool>,
    out: &Out,
    counts: Counts<'_>,
    links: &mut Vec<(String, String)>,
) -> Walk {
    let mut records = Records::default();
    let mut status = None;
    loop {
        let output = tokio::select! {
            output = exec.next() => output,
            changed = stopped.changed() => {
                // A dropped Search stops too.
                let _ = changed;
                kill(remote, exec, records.pid).await;
                return Walk::Stopped;
            }
        };
        match output {
            None => break,
            Some(Output::Exit(code)) => status = Some(code),
            Some(Output::Stderr(data)) => {
                *counts.unreadable += data.iter().filter(|b| **b == b'\n').count();
            }
            Some(Output::Stdout(data)) => {
                let mut batch = Vec::new();
                let mut capped = false;
                for (kind, rel) in records.feed(&data) {
                    let shown = if root.shown.is_empty() {
                        rel.clone()
                    } else {
                        format!("{}/{rel}", root.shown)
                    };
                    if matches!(kind, b'l' | b'L') {
                        links.push((join(&root.dir, &rel), shown.clone()));
                    }
                    if kind == b'l' {
                        continue;
                    }
                    batch.push(Match {
                        path: shown,
                        is_dir: kind != b'f',
                        link: matches!(kind, b'L' | b'D'),
                    });
                    *counts.found += 1;
                    if *counts.found >= counts.cap {
                        capped = true;
                        break;
                    }
                }
                if !batch.is_empty() {
                    out.send(Event::Found(batch));
                }
                if capped {
                    kill(remote, exec, records.pid).await;
                    return Walk::Capped;
                }
            }
        }
    }
    Walk::Done {
        status,
        pid: records.pid.is_some(),
    }
}

/// Kills the `find` with `pid` on the host, and closes its channel.
async fn kill(remote: &Remote, exec: ssh::Exec, pid: Option<u32>) {
    if let Some(pid) = pid
        && let Ok(mut killing) = remote.exec(&format!("kill {pid}")).await
    {
        while killing.next().await.is_some() {}
    }
    exec.close().await;
}

fn join(dir: &str, rel: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{rel}")
    } else {
        format!("{dir}/{rel}")
    }
}

/// The command's output, split: first the pid line, then NUL-ended records of a kind
/// byte and a `./`-relative path.
#[derive(Default)]
struct Records {
    pid: Option<u32>,
    buffer: Vec<u8>,
}

impl Records {
    /// Takes `data` and gives the records it completes, as (kind, relative path).
    fn feed(&mut self, data: &[u8]) -> Vec<(u8, String)> {
        self.buffer.extend_from_slice(data);
        if self.pid.is_none() {
            let Some(end) = self.buffer.iter().position(|b| *b == b'\n') else {
                return Vec::new();
            };
            let line: Vec<u8> = self.buffer.drain(..=end).collect();
            self.pid = String::from_utf8_lossy(&line).trim().parse().ok();
        }
        let mut records = Vec::new();
        while let Some(end) = self.buffer.iter().position(|b| *b == 0) {
            let record: Vec<u8> = self.buffer.drain(..=end).collect();
            let record = &record[..record.len() - 1];
            let Some((&kind, path)) = record.split_first() else {
                continue;
            };
            let path = String::from_utf8_lossy(path);
            let rel = path.strip_prefix("./").unwrap_or(&path);
            if rel.is_empty() || rel == "." {
                continue;
            }
            records.push((kind, rel.to_owned()));
        }
        records
    }
}
