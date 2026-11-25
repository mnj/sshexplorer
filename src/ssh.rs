//! The SSH stack: opening a Session's connection and talking SFTP over it, off the egui
//! frame thread.
//!
//! The stack is russh with russh-sftp (ADR-0002, issue #4), on a tokio runtime that
//! [`Ssh`] owns. The frame thread never awaits: [`Ssh::open`] returns an [`Opening`] at
//! once, and the connection's progress arrives on it as [`Event`]s, each followed by a
//! call to the wake function (`egui::Context::request_repaint` in the app). A question
//! for the operator (trust this host key? the password?) is an event carrying a prompt;
//! answering it, or dropping it to cancel, lets the connection go on.
//!
//! An open [`Session`] watches its connection: once it drops, by a transport error or
//! three unanswered keepalives, [`Session::lost`] says why and the frame is woken.
//! [`Session::reconnect`] opens it again the same way, sending the secrets it logged in
//! with before asking anything.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::client::{self, DisconnectReason};
use russh::keys::PublicKeyOrCertificate;
use russh::keys::ssh_key::{Algorithm, PublicKey};
use russh::{Disconnect, Preferred};
use russh_sftp::client::{RawSftpSession, SftpSession};
use tokio::sync::oneshot;

use crate::credentials::{self, Kind};
use crate::hosts::AuthMethod;
use crate::known_hosts::{self, Recorded, Verdict};

mod auth;
mod opening;

pub use auth::{NO_AGENT, PassphrasePrompt, Question, QuestionsPrompt, default_agent};

/// Default opening inactivity deadline. Time spent answering prompts is excluded.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// How often a keepalive goes out, unless the config's `ServerAliveInterval` says.
const KEEPALIVE: Duration = Duration::from_secs(30);

/// How to connect, besides where: the host-key files, the timings, and how to log in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// The `known_hosts` file, which Trust appends to.
    pub known_hosts: PathBuf,
    /// The config's `UserKnownHostsFile`s, checked as well; Trust never writes to them.
    pub also_known_hosts: Vec<PathBuf>,
    /// Maximum network inactivity while opening, excluding prompts; `None` for 30 s.
    pub connect_timeout: Option<Duration>,
    /// How often a keepalive goes out; `None` for every 30 s, zero for never.
    pub keepalive: Option<Duration>,
    /// Which methods to log in by.
    pub auth: AuthMethod,
    /// The Key file, else the config's `IdentityFile`s, tried in order.
    pub key_files: Vec<PathBuf>,
    /// The config's `IdentitiesOnly`: offer no agent identity but the key files'.
    pub identities_only: bool,
    /// The SSH agent's socket (a named pipe on Windows); `None` for no agent.
    pub agent: Option<PathBuf>,
    /// Where a remembering Host keeps its secret; `None` when nothing is remembered.
    pub remember: Option<Remember>,
}

/// A saved Host with Remember on: its secret is read from `store` when one is needed,
/// and the one that logged in is written there once the Session is open.
#[derive(Clone)]
pub struct Remember {
    pub store: Arc<dyn credentials::Store>,
    /// The Host's id, which names its entries.
    pub host: String,
}

impl std::fmt::Debug for Remember {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Remember")
            .field("host", &self.host)
            .finish()
    }
}

impl PartialEq for Remember {
    fn eq(&self, other: &Self) -> bool {
        self.host == other.host && Arc::ptr_eq(&self.store, &other.store)
    }
}

impl Eq for Remember {}

/// What a secret prompt says of the credential store.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Saved {
    /// Nothing: no store involved, no saved secret, or not the first prompt.
    #[default]
    Nothing,
    /// The secret the store returned was rejected.
    Rejected,
    /// The store could not return a secret.
    Unavailable,
}

impl Saved {
    /// The dialog's line for a secret of `kind`, if any.
    pub fn line(self, kind: Kind) -> Option<String> {
        match self {
            Saved::Nothing => None,
            Saved::Rejected => Some(format!(
                "The saved {} was rejected. Enter it to open this Session.",
                kind.name()
            )),
            Saved::Unavailable => Some(credentials::UNAVAILABLE.to_owned()),
        }
    }
}

impl Options {
    /// Checking host keys against `known_hosts` alone, with the default timings, logging
    /// in Automatically with no key file and no agent.
    pub fn new(known_hosts: PathBuf) -> Options {
        Options {
            known_hosts,
            also_known_hosts: Vec::new(),
            connect_timeout: None,
            keepalive: None,
            auth: AuthMethod::Automatic,
            key_files: Vec::new(),
            identities_only: false,
            agent: None,
            remember: None,
        }
    }
}

type Wake = Arc<dyn Fn() + Send + Sync>;

/// Where and as whom to connect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub address: String,
    pub port: u16,
    pub account: String,
}

/// The SSH stack: a tokio runtime on background threads, and the function that wakes the
/// frame thread when something arrives.
pub struct Ssh {
    runtime: tokio::runtime::Runtime,
    wake: Wake,
}

impl Ssh {
    /// Starts the runtime. `wake` is called after every event, from a background thread.
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("ssh")
            .enable_all()
            .build()
            .expect("the SSH runtime starts");
        Ssh {
            runtime,
            wake: Arc::new(wake),
        }
    }

    /// Starts opening a Session to `target`, checking its host key against the
    /// `known_hosts` file at `known_hosts`, to which Trust appends.
    pub fn open(&self, target: Target, known_hosts: PathBuf) -> Opening {
        self.open_in(target, known_hosts, "")
    }

    /// As [`Ssh::open`], with the Session opening in `folder`: absolute, or relative to
    /// the account's home directory, which an empty `folder` means.
    pub fn open_in(&self, target: Target, known_hosts: PathBuf, folder: &str) -> Opening {
        self.connect(target, Options::new(known_hosts), folder)
    }

    /// As [`Ssh::open_in`], with the host-key files and timings in `options`.
    pub fn connect(&self, target: Target, options: Options, folder: &str) -> Opening {
        let (sender, events) = mpsc::channel();
        let out = Events {
            sender,
            wake: self.wake.clone(),
            watch: Arc::default(),
            progress: Arc::default(),
        };
        let folder = folder.trim().to_owned();
        let progress = out.progress.clone();
        let task = self.runtime.spawn(open(
            target,
            options,
            Some(folder),
            auth::Held::new(),
            Arc::new(out),
        ));
        Opening {
            events,
            task: Some(task),
            progress,
        }
    }
}

/// A Session being opened: its events, in order. The last is [`Event::Opened`] or
/// [`Event::Failed`].
pub struct Opening {
    progress: Arc<Mutex<opening::Progress>>,
    task: Option<tokio::task::JoinHandle<()>>,
    events: mpsc::Receiver<Event>,
}

impl Drop for Opening {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
        self.progress.lock().unwrap().close();
    }
}

impl Opening {
    pub fn stage(&self) -> &'static str {
        self.progress.lock().unwrap().stage
    }

    /// A Session that fails before it connects: its one event is [`Event::Failed`].
    pub fn failed(failure: Failure) -> Opening {
        let (sender, events) = mpsc::channel();
        let _ = sender.send(Event::Failed(failure));
        Opening {
            events,
            task: None,
            progress: Arc::default(),
        }
    }

    /// The next event, if one has arrived. Never blocks.
    pub fn try_next(&self) -> Option<Event> {
        self.events.try_recv().ok()
    }

    /// Waits up to `timeout` for the next event: for tests, not the frame thread.
    pub fn recv_timeout(&self, timeout: Duration) -> Option<Event> {
        self.events.recv_timeout(timeout).ok()
    }
}

/// Something that happened while opening a Session.
pub enum Event {
    /// The host's key is not in `known_hosts`: trust it or cancel.
    TrustHostKey(TrustPrompt),
    /// The password is needed, for the password method or a keyboard-interactive round
    /// with one hidden prompt.
    Password(PasswordPrompt),
    /// A key file is locked: its passphrase is needed.
    Passphrase(PassphrasePrompt),
    /// A keyboard-interactive round asks several things (or one shown answer).
    Questions(QuestionsPrompt),
    /// The Session is open, with the folder it opened in listed.
    Opened(Opened),
    /// The Session did not open.
    Failed(Failure),
}

/// The question whether to trust an unknown host key. Dropping it cancels.
pub struct TrustPrompt {
    /// The name the key is recorded under: `address`, or `[address]:port` off port 22.
    pub host: String,
    /// The key's `SHA256:` fingerprint.
    pub fingerprint: String,
    answer: oneshot::Sender<bool>,
}

impl TrustPrompt {
    /// Appends the key to `known_hosts` and goes on connecting.
    pub fn trust(self) {
        let _ = self.answer.send(true);
    }

    /// Closes the connection; the Session fails with [`Failure::Cancelled`].
    pub fn cancel(self) {
        let _ = self.answer.send(false);
    }
}

/// The request for the account's password. Dropping it cancels.
pub struct PasswordPrompt {
    /// Who the password is for: `account@address`.
    pub account: String,
    /// Whether the host just rejected the password entered before.
    pub rejected: bool,
    /// What to say of the credential store.
    pub saved: Saved,
    answer: oneshot::Sender<Option<String>>,
}

impl PasswordPrompt {
    pub fn enter(self, password: String) {
        let _ = self.answer.send(Some(password));
    }

    /// Closes the connection; the Session fails with [`Failure::Cancelled`].
    pub fn cancel(self) {
        let _ = self.answer.send(None);
    }
}

/// Why a Session did not open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// The operator cancelled a prompt.
    Cancelled,
    /// The host's key differs from the one recorded. There is no override.
    HostKeyChanged(KeyMismatch),
    /// The host's key is marked `@revoked` in `known_hosts`.
    HostKeyRevoked(KeyMismatch),
    /// Trust could not write the key to `known_hosts`.
    KnownHostsWrite { file: PathBuf, reason: String },
    /// The connection failed or dropped: unreachable, refused, timed out, closed.
    Connection(String),
    /// The SFTP subsystem would not start, or the folder to open in could not be listed.
    Sftp(String),
    /// The Session needs something sshexplorer does not do, such as a proxy.
    Unsupported(String),
    /// No method logged in: no agent, no key file, or none the host accepts.
    Auth(String),
}

/// A host key that `known_hosts` refuses, and the line that refuses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyMismatch {
    pub host: String,
    /// The `SHA256:` fingerprint of the key the host offered.
    pub fingerprint: String,
    pub file: PathBuf,
    pub recorded: Recorded,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failure::Cancelled => write!(f, "Cancelled."),
            Failure::HostKeyChanged(m) => write!(
                f,
                "The host key for {} has changed, so the connection was refused.\n\
                 Recorded: {}\nOffered: {}\n{} line {}: {}",
                m.host,
                m.recorded.fingerprint,
                m.fingerprint,
                m.file.display(),
                m.recorded.line_no,
                m.recorded.line
            ),
            Failure::HostKeyRevoked(m) => write!(
                f,
                "The host key for {} ({}) is revoked, so the connection was refused.\n\
                 {} line {}: {}",
                m.host,
                m.fingerprint,
                m.file.display(),
                m.recorded.line_no,
                m.recorded.line
            ),
            Failure::KnownHostsWrite { file, reason } => {
                write!(
                    f,
                    "Could not add the host key to {}: {reason}",
                    file.display()
                )
            }
            Failure::Connection(reason) => write!(f, "Could not connect: {reason}"),
            Failure::Sftp(reason) | Failure::Unsupported(reason) | Failure::Auth(reason) => {
                write!(f, "{reason}")
            }
        }
    }
}

/// An open Session's connection, with the listing of the folder it opened in.
pub struct Opened {
    pub session: Session,
    /// The Initial remote folder, or the home directory when that is empty.
    pub home: Listing,
    /// Something the operator may dismiss: the credential store did not keep the secret.
    pub note: Option<String>,
}

/// One directory's entries, without `.` and `..`, in the order the host sent them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    /// The directory's absolute path.
    pub path: String,
    pub entries: Vec<Entry>,
}

/// One entry of a directory. A symlink is described by what it points at: a link to a
/// directory is a directory, with its target in `link`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    /// In bytes; `None` when the host did not say.
    pub size: Option<u64>,
    /// Seconds since the Unix epoch.
    pub modified: Option<u32>,
    /// The mode bits, file type included, as `st_mode`.
    pub permissions: Option<u32>,
    /// The link's target as the host stores it, when the entry is a symlink.
    pub link: Option<String>,
    /// The owner's and group's numeric ids.
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    /// The owner's and group's names, from the listing's `ls -l` line (SFTP v3's
    /// attributes carry only the ids); `None` when the host sent no such line.
    pub owner: Option<String>,
    pub group: Option<String>,
}

/// The owner and group an `ls -l` style line names, as OpenSSH's SFTP server writes a
/// directory listing's `longname`: `-rw-r--r--    1 alice    staff    1234 Jan  1 00:00 a`.
/// `None` for a line that does not look like one.
pub fn owner_and_group(longname: &str) -> Option<(String, String)> {
    let mut fields = longname.split_whitespace();
    let mode = fields.next()?;
    let links = fields.next()?;
    let looks_like_mode = mode.len() >= 10 && mode.is_char_boundary(10);
    if !looks_like_mode || !links.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let owner = fields.next()?;
    let group = fields.next()?;
    // The size must follow, or the line was something else.
    fields.next()?;
    Some((owner.to_owned(), group.to_owned()))
}

/// A Session's SFTP: the files side, and a second channel kept for listings, which read
/// each entry's `longname` (the files side drops it).
pub(crate) struct Sftp {
    files: SftpSession,
    lister: RawSftpSession,
}

impl std::ops::Deref for Sftp {
    type Target = SftpSession;

    fn deref(&self) -> &SftpSession {
        &self.files
    }
}

/// An open Session's connection. Dropping it closes the connection.
pub struct Session {
    handle: Arc<client::Handle<HostCheck>>,
    sftp: Arc<Sftp>,
    runtime: tokio::runtime::Handle,
    wake: Wake,
    watch: Arc<Watch>,
    /// How it was opened, to open it again.
    redial: Arc<Redial>,
}

/// Why a connection dropped, as the Handler heard it.
#[derive(Default)]
struct Watch {
    reason: Mutex<Option<String>>,
}

impl Watch {
    fn set(&self, reason: String) {
        if let Ok(mut r) = self.reason.lock() {
            *r = Some(reason);
        }
    }

    fn reason(&self) -> Option<String> {
        self.reason.lock().ok().and_then(|r| r.clone())
    }
}

/// What reconnecting needs: where, how, and the secrets that logged in.
struct Redial {
    target: Target,
    options: Options,
    held: auth::Held,
}

/// The reason for a drop when the connection gave none.
const CLOSED: &str = "the host closed the connection";

/// How long [`Session`]'s watch waits between looks at the connection.
const WATCH_EVERY: Duration = Duration::from_millis(200);

/// A drop as the strip says it.
fn drop_reason(reason: &DisconnectReason<russh::Error>) -> String {
    match reason {
        DisconnectReason::ReceivedDisconnect(_) => CLOSED.into(),
        DisconnectReason::Error(e) => match e {
            russh::Error::KeepaliveTimeout => "the host stopped answering".into(),
            russh::Error::InactivityTimeout => "the connection was idle too long".into(),
            russh::Error::Disconnect | russh::Error::HUP => CLOSED.into(),
            russh::Error::IO(io) if io.kind() == std::io::ErrorKind::UnexpectedEof => CLOSED.into(),
            other => other.to_string(),
        },
    }
}

impl Session {
    /// Why the connection dropped, once it has: then the Session is Disconnected.
    pub fn lost(&self) -> Option<String> {
        self.handle
            .is_closed()
            .then(|| self.watch.reason().unwrap_or_else(|| CLOSED.to_owned()))
    }

    /// Opens the connection again as it was opened, checking the host key again and
    /// sending the secrets it logged in with (or the stored ones under Remember) before
    /// asking. Its [`Opened`] lists nothing: the caller refreshes what it shows.
    pub fn reconnect(&self) -> Opening {
        let (sender, events) = mpsc::channel();
        let out = Arc::new(Events {
            sender,
            wake: self.wake.clone(),
            watch: Arc::default(),
            progress: Arc::default(),
        });
        let redial = self.redial.clone();
        let progress = out.progress.clone();
        let task = self.runtime.spawn(async move {
            open(
                redial.target.clone(),
                redial.options.clone(),
                None,
                redial.held.clone(),
                out,
            )
            .await
        });
        Opening {
            events,
            task: Some(task),
            progress,
        }
    }

    /// The connection's SFTP side, for work that runs on the SSH runtime, such as a
    /// Download. Start that work with [`Session::spawn`].
    pub fn remote(&self) -> Remote {
        Remote {
            sftp: self.sftp.clone(),
            handle: self.handle.clone(),
        }
    }

    /// Runs `work` on the SSH runtime.
    pub fn spawn(&self, work: impl Future<Output = ()> + Send + 'static) {
        self.runtime.spawn(work);
    }

    /// The function that wakes the frame thread.
    pub fn waker(&self) -> Arc<dyn Fn() + Send + Sync> {
        self.wake.clone()
    }

    /// Starts reading up to `len` bytes of the file at the absolute `path`, from byte
    /// `offset`. Fewer arrive only when the file ends first. The answer wakes the frame;
    /// dropping the request stops the read.
    pub fn read(&self, path: String, offset: u64, len: usize) -> Request<Vec<u8>> {
        let (sender, answer) = mpsc::channel();
        let remote = self.remote();
        let wake = self.wake.clone();
        let task = self.runtime.spawn(async move {
            let answer = remote.read(path, offset, len).await;
            if let Err(e) = &answer
                && !matches!(e.as_str(), "No such file" | "Permission denied")
            {
                // A failure caused by a drop is seen together with it.
                remote.closed().await;
            }
            let _ = sender.send(answer);
            wake();
        });
        Request {
            answer,
            task: Some(task.abort_handle()),
        }
    }

    /// Resolves an entered path on the host. A file opens its parent with the file focused.
    pub(crate) fn locate(&self, path: String) -> Request<(Listing, Option<String>)> {
        let (sender, answer) = mpsc::channel();
        let sftp = self.sftp.clone();
        let wake = self.wake.clone();
        self.runtime.spawn(async move {
            let result = async {
                let path = sftp.canonicalize(path).await?;
                let attrs = sftp.metadata(&path).await?;
                let (dir, focus) = if attrs.is_dir() {
                    (path, None)
                } else if let Some((dir, name)) = crate::folder::parent(&path) {
                    (dir, Some(name))
                } else {
                    (path, None)
                };
                list(&sftp, dir).await.map(|listing| (listing, focus))
            }
            .await
            .map_err(|e| reason(&e));
            let _ = sender.send(result);
            wake();
        });
        Request {
            answer,
            // A list already in progress must close its remote directory handle.
            task: None,
        }
    }

    /// Starts listing the directory at the absolute `path`. A symlinked directory is
    /// followed. The answer arrives on the returned request, and wakes the frame.
    pub fn list(&self, path: String) -> Request<Listing> {
        let (sender, answer) = mpsc::channel();
        let sftp = self.sftp.clone();
        let handle = self.handle.clone();
        let wake = self.wake.clone();
        self.runtime.spawn(async move {
            let listing = list(&sftp, path).await;
            if let Err(e) = &listing {
                settle(&handle, e).await;
            }
            let _ = sender.send(listing.map_err(|e| reason(&e)));
            wake();
        });
        Request { answer, task: None }
    }
}

/// After a request failed other than by the host's answer, waits briefly for the
/// connection to report that it closed, so that the failure is seen with the drop.
async fn settle(handle: &client::Handle<HostCheck>, error: &russh_sftp::client::error::Error) {
    if matches!(error, russh_sftp::client::error::Error::Status(_)) {
        return;
    }
    for _ in 0..20 {
        if handle.is_closed() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// A Session's SFTP connection, to use from tasks on the SSH runtime. Each failure is
/// the reason as the operator reads it: "No such file", "Permission denied", or the
/// host's own words.
#[derive(Clone)]
pub struct Remote {
    sftp: Arc<Sftp>,
    /// For commands, which run beside SFTP on the same connection.
    handle: Arc<client::Handle<HostCheck>>,
}

/// `text` with every byte but ASCII letters and digits written as a `\ooo` octal
/// escape, which `printf` decodes.
pub(crate) fn shell_arg(text: &str) -> String {
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

pub(crate) const EXEC_REFUSED: &str = "The host refused to run the command";

/// A command running on the host: what it prints, as it prints it, and how it ended.
/// Dropping it closes its channel.
pub struct Exec {
    channel: Option<russh::Channel<client::Msg>>,
    runtime: tokio::runtime::Handle,
}

impl Drop for Exec {
    fn drop(&mut self) {
        if let Some(channel) = self.channel.take() {
            self.runtime.spawn(async move {
                let _ = channel.close().await;
            });
        }
    }
}

/// One piece of what a command does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Output {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    /// The exit status; a command killed by a signal ends without one.
    Exit(u32),
}

impl Exec {
    async fn message(&mut self) -> Option<russh::ChannelMsg> {
        let message = self.channel.as_mut()?.wait().await;
        if matches!(message, None | Some(russh::ChannelMsg::Close)) {
            self.channel.take();
            return None;
        }
        message
    }

    /// The next piece of output, or `None` once the channel has closed.
    pub async fn next(&mut self) -> Option<Output> {
        loop {
            match self.message().await? {
                russh::ChannelMsg::Data { data } => return Some(Output::Stdout(data.to_vec())),
                russh::ChannelMsg::ExtendedData { data, ext: 1 } => {
                    return Some(Output::Stderr(data.to_vec()));
                }
                russh::ChannelMsg::ExitStatus { exit_status } => {
                    return Some(Output::Exit(exit_status));
                }
                _ => {}
            }
        }
    }

    /// Closes the channel.
    pub async fn close(mut self) {
        if let Some(channel) = &self.channel {
            let _ = channel.close().await;
            self.channel.take();
        }
    }
}

/// A remote file open for reading.
pub type RemoteFile = russh_sftp::client::fs::File;

impl Remote {
    /// The entries of the directory at the absolute `path`, as [`Session::list`] gives them.
    pub async fn list(&self, path: String) -> Result<Listing, String> {
        list(&self.sftp, path)
            .await
            .map_err(|e| operator_reason(&e))
    }

    /// Fills gaps in listing or Search metadata from the file's target.
    pub(crate) async fn complete_file_metadata(
        &self,
        path: &str,
        entry: &mut Entry,
    ) -> Result<(), String> {
        let attrs = self
            .sftp
            .metadata(path)
            .await
            .map_err(|e| operator_reason(&e))?;
        entry.size = entry.size.or(attrs.size);
        entry.modified = entry.modified.or(attrs.mtime);
        entry.permissions = entry.permissions.or(attrs.permissions);
        Ok(())
    }

    /// The absolute path with every symlink resolved.
    pub async fn canonicalize(&self, path: String) -> Result<String, String> {
        self.sftp
            .canonicalize(path)
            .await
            .map_err(|e| operator_reason(&e))
    }

    /// Runs `command` through the account's login shell. Fails when no channel opens or
    /// the host refuses to run commands.
    pub async fn exec(&self, command: &str) -> Result<Exec, String> {
        let channel = self
            .handle
            .channel_open_session()
            .await
            .map_err(|e| e.to_string())?;
        let mut exec = Exec {
            channel: Some(channel),
            runtime: tokio::runtime::Handle::current(),
        };
        exec.channel
            .as_ref()
            .unwrap()
            .exec(true, command)
            .await
            .map_err(|e| e.to_string())?;
        loop {
            match exec.message().await {
                Some(russh::ChannelMsg::Success) => return Ok(exec),
                Some(russh::ChannelMsg::Failure) | None => return Err(EXEC_REFUSED.to_owned()),
                _ => {}
            }
        }
    }

    /// Whether the connection has closed, now.
    pub fn is_closed(&self) -> bool {
        self.handle.is_closed()
    }

    /// Whether the connection has closed: after a request failed, waits briefly for the
    /// close to show, so that a failure caused by a drop is seen as one.
    pub async fn closed(&self) -> bool {
        for _ in 0..20 {
            if self.handle.is_closed() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        self.handle.is_closed()
    }

    /// Opens the file at `path` for reading.
    pub async fn open(&self, path: String) -> Result<RemoteFile, String> {
        self.sftp.open(path).await.map_err(|e| operator_reason(&e))
    }

    /// Up to `len` bytes of the file at `path` from `offset`.
    pub async fn read(&self, path: String, offset: u64, len: usize) -> Result<Vec<u8>, String> {
        use tokio::io::{AsyncReadExt as _, AsyncSeekExt as _};
        let mut file = self.open(path).await?;
        if offset > 0 {
            file.seek(std::io::SeekFrom::Start(offset))
                .await
                .map_err(|e| e.to_string())?;
        }
        let mut data = Vec::with_capacity(len.min(1 << 20));
        (&mut file)
            .take(len as u64)
            .read_to_end(&mut data)
            .await
            .map_err(|e| e.to_string())?;
        Ok(data)
    }
}

/// An SFTP failure as a Download's Details say it: "No such file", "Permission denied",
/// or the host's own words.
fn operator_reason(error: &russh_sftp::client::error::Error) -> String {
    use russh_sftp::client::error::Error;
    use russh_sftp::protocol::StatusCode;
    match error {
        Error::Status(status) => match status.status_code {
            StatusCode::NoSuchFile => "No such file".to_owned(),
            StatusCode::PermissionDenied => "Permission denied".to_owned(),
            _ if !status.error_message.is_empty() => status.error_message.clone(),
            code => code.to_string(),
        },
        other => other.to_string(),
    }
}

/// A request to the host: its answer, or why there is none. Dropping a request whose
/// work can be stopped (a read) stops it.
pub struct Request<T> {
    answer: mpsc::Receiver<Result<T, String>>,
    /// The task answering it, stopped when the request is dropped.
    task: Option<tokio::task::AbortHandle>,
}

impl<T> Request<T> {
    /// The answer, if it has arrived. Never blocks.
    pub fn try_take(&self) -> Option<Result<T, String>> {
        self.answer.try_recv().ok()
    }

    /// Waits up to `timeout` for the answer: for tests, not the frame thread.
    pub fn recv_timeout(&self, timeout: Duration) -> Option<Result<T, String>> {
        self.answer.recv_timeout(timeout).ok()
    }
}

impl<T> Drop for Request<T> {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let handle = self.handle.clone();
        let sftp = self.sftp.clone();
        self.runtime.spawn(async move {
            let _ = sftp.close().await;
            let _ = handle
                .disconnect(Disconnect::ByApplication, "Session closed", "en")
                .await;
        });
    }
}

/// The sending end of an Opening.
struct Events {
    progress: Arc<Mutex<opening::Progress>>,
    sender: mpsc::Sender<Event>,
    wake: Wake,
    /// Why the connection being opened dropped, once open.
    watch: Arc<Watch>,
}

impl Events {
    fn send(&self, event: Event) {
        let _ = self.sender.send(event);
        (self.wake)();
    }
}

async fn open(
    target: Target,
    options: Options,
    folder: Option<String>,
    held: auth::Held,
    out: Arc<Events>,
) {
    let limit = options.connect_timeout.unwrap_or(CONNECT_TIMEOUT);
    let result = tokio::select! {
        result = connect(&target, options, folder.as_deref(), &held, &out) => result,
        failure = out.timeout(limit) => Err(failure),
    };
    let failure = match result {
        Ok(opened) => {
            out.progress.lock().unwrap().socket.take();
            out.send(Event::Opened(opened));
            return;
        }
        Err(failure) => failure,
    };
    out.progress.lock().unwrap().close();
    out.send(Event::Failed(failure));
}

/// Opens the connection and lists `folder` (an empty one: the home directory); with no
/// `folder`, as a reconnect does, nothing is listed.
async fn connect(
    target: &Target,
    options: Options,
    folder: Option<&str>,
    held: &auth::Held,
    out: &Arc<Events>,
) -> Result<Opened, Failure> {
    let mut handle = dial(target, &options, out).await?;
    out.stage("Authenticating");
    let logged_in = auth::authenticate(&mut handle, target, &options, held, out).await?;
    let kept = logged_in.kept;

    out.stage("Starting SFTP");
    let sftp = start_sftp(&handle)
        .await
        .map_err(|e| Failure::Sftp(format!("The SFTP subsystem did not start: {e}")))?;
    out.stage("Opening folder");
    let home = match folder {
        Some(folder) => list_folder(&sftp, folder).await.map_err(|e| {
            let folder = if folder.is_empty() {
                "the home directory"
            } else {
                folder
            };
            Failure::Sftp(format!("Could not list {folder}: {e}"))
        })?,
        None => Listing {
            path: String::new(),
            entries: Vec::new(),
        },
    };
    // Only now, with the Session open, is a newly entered secret kept.
    let mut note = None;
    if let (Some(remember), Some((kind, secret))) = (&options.remember, kept) {
        let store = remember.store.clone();
        let key = credentials::Key::new(remember.host.clone(), kind);
        let written = tokio::task::spawn_blocking(move || store.set(&key, &secret)).await;
        if !matches!(written, Ok(Ok(()))) {
            note = Some(credentials::NOT_KEPT.to_owned());
        }
    }
    let handle = Arc::new(handle);
    watch(&handle, out.wake.clone());
    Ok(Opened {
        session: Session {
            handle,
            sftp: Arc::new(sftp),
            runtime: tokio::runtime::Handle::current(),
            wake: out.wake.clone(),
            watch: out.watch.clone(),
            redial: Arc::new(Redial {
                target: target.clone(),
                options,
                held: logged_in.used,
            }),
        },
        home,
        note,
    })
}

/// Wakes the frame once the connection closes, however it closes. The watch ends then,
/// or when the Session is dropped.
fn watch(handle: &Arc<client::Handle<HostCheck>>, wake: Wake) {
    let handle = Arc::downgrade(handle);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(WATCH_EVERY).await;
            let Some(handle) = handle.upgrade() else {
                return;
            };
            if handle.is_closed() {
                // The Handler may still be noting why.
                tokio::time::sleep(Duration::from_millis(20)).await;
                wake();
                return;
            }
        }
    });
}

/// Connects to `target` and checks its host key: everything before logging in.
async fn dial(
    target: &Target,
    options: &Options,
    out: &Arc<Events>,
) -> Result<client::Handle<HostCheck>, Failure> {
    let recorded: Vec<Algorithm> = std::iter::once(&options.known_hosts)
        .chain(&options.also_known_hosts)
        .flat_map(|file| {
            let text = std::fs::read_to_string(file).unwrap_or_default();
            known_hosts::recorded_algorithms(&text, &target.address, target.port)
        })
        .collect();
    let keepalive = options.keepalive.unwrap_or(KEEPALIVE);
    let config = client::Config {
        preferred: Preferred {
            key: prefer(&recorded).into(),
            ..Preferred::default()
        },
        keepalive_interval: (!keepalive.is_zero()).then_some(keepalive),
        keepalive_max: 3,
        ..client::Config::default()
    };
    let refused = Arc::new(Mutex::new(None));
    let check = HostCheck {
        address: target.address.clone(),
        port: target.port,
        known_hosts: options.known_hosts.clone(),
        also_known_hosts: options.also_known_hosts.clone(),
        out: out.clone(),
        refused: refused.clone(),
    };
    let address = (target.address.as_str(), target.port);
    let stream = match tokio::time::timeout(
        options.connect_timeout.unwrap_or(CONNECT_TIMEOUT),
        tokio::net::TcpStream::connect(address),
    )
    .await
    {
        Ok(Ok(stream)) => stream,
        Ok(Err(e)) => return Err(Failure::Connection(e.to_string())),
        Err(_) => return Err(Failure::Connection("timed out".into())),
    };
    out.stage("Negotiating SSH");
    let stream = out
        .opening_stream(stream)
        .map_err(|e| Failure::Connection(e.to_string()))?;
    match client::connect_stream(Arc::new(config), stream, check).await {
        Ok(handle) => Ok(handle),
        Err(e) => {
            let refused = refused.lock().ok().and_then(|mut r| r.take());
            Err(refused.unwrap_or_else(|| Failure::Connection(e.to_string())))
        }
    }
}

async fn start_sftp(
    handle: &client::Handle<HostCheck>,
) -> Result<Sftp, Box<dyn std::error::Error>> {
    let channel = handle.channel_open_session().await?;
    channel.request_subsystem(true, "sftp").await?;
    let files = SftpSession::new(channel.into_stream()).await?;
    let channel = handle.channel_open_session().await?;
    channel.request_subsystem(true, "sftp").await?;
    let lister = RawSftpSession::new(channel.into_stream());
    lister.init().await?;
    Ok(Sftp { files, lister })
}

/// Lists `folder`, absolute or relative to the home directory (empty: the home itself).
async fn list_folder(
    sftp: &Sftp,
    folder: &str,
) -> Result<Listing, russh_sftp::client::error::Error> {
    let folder = if folder.is_empty() { "." } else { folder };
    let path = sftp.canonicalize(folder).await?;
    list(sftp, path).await
}

/// The reason [`Session::list`] gives for a directory that does not exist.
pub const NO_SUCH_FILE: &str = "no such file";

/// Why a request failed, as a listing's reason row says it: "permission denied" for a
/// status from the host, whose own message often only repeats the status.
fn reason(error: &russh_sftp::client::error::Error) -> String {
    match error {
        russh_sftp::client::error::Error::Status(status) => {
            status.status_code.to_string().to_lowercase()
        }
        other => other.to_string(),
    }
}

/// The entries of the directory at `path`. A symlink is described by its target, which
/// is looked up with `stat`; a link whose target cannot be read is not a directory.
async fn list(sftp: &Sftp, path: String) -> Result<Listing, russh_sftp::client::error::Error> {
    let mut entries = Vec::new();
    for file in read_dir(&sftp.lister, &path).await? {
        let name = file.filename;
        let names = owner_and_group(&file.longname);
        let mut attrs = file.attrs;
        let mut link = None;
        if attrs.is_symlink() {
            let at = crate::folder::join(&path, &name);
            link = sftp.read_link(at.clone()).await.ok();
            if let Ok(target) = sftp.metadata(at).await {
                attrs = target;
            }
        }
        entries.push(Entry {
            name,
            is_dir: attrs.is_dir(),
            size: attrs.size,
            modified: attrs.mtime,
            permissions: attrs.permissions,
            link,
            uid: attrs.uid,
            gid: attrs.gid,
            owner: names.as_ref().map(|(owner, _)| owner.clone()),
            group: names.map(|(_, group)| group),
        });
    }
    Ok(Listing { path, entries })
}

/// The directory at `path`'s entries as the host sends them, `.` and `..` left out.
async fn read_dir(
    lister: &RawSftpSession,
    path: &str,
) -> Result<Vec<russh_sftp::protocol::File>, russh_sftp::client::error::Error> {
    use russh_sftp::client::error::Error;
    use russh_sftp::protocol::StatusCode;
    let handle = lister.opendir(path).await?.handle;
    let mut files = Vec::new();
    loop {
        match lister.readdir(handle.clone()).await {
            Ok(name) => files.extend(name.files),
            Err(Error::Status(status)) if status.status_code == StatusCode::Eof => break,
            Err(e) => {
                let _ = lister.close(handle).await;
                return Err(e);
            }
        }
    }
    lister.close(handle).await?;
    files.retain(|f| f.filename != "." && f.filename != "..");
    Ok(files)
}

/// The host-key algorithms to offer: those already recorded for the host first, then the
/// rest of russh's defaults, so that a host known by one key type is not asked for another.
fn prefer(recorded: &[Algorithm]) -> Vec<Algorithm> {
    let defaults = Preferred::default().key.into_owned();
    let (mut first, rest): (Vec<Algorithm>, Vec<Algorithm>) = defaults
        .into_iter()
        .partition(|a| recorded.iter().any(|r| same_kind(r, a)));
    first.extend(rest);
    first
}

/// Whether two algorithms are the same key type: RSA is one type under every hash.
fn same_kind(a: &Algorithm, b: &Algorithm) -> bool {
    matches!((a, b), (Algorithm::Rsa { .. }, Algorithm::Rsa { .. })) || a == b
}

/// The client handler: checks the host key against `known_hosts` and the config's
/// `UserKnownHostsFile`s, asking the operator about an unknown one.
struct HostCheck {
    address: String,
    port: u16,
    known_hosts: PathBuf,
    also_known_hosts: Vec<PathBuf>,
    out: Arc<Events>,
    /// Why the key was refused, for `connect` to report in place of russh's error.
    refused: Arc<Mutex<Option<Failure>>>,
}

impl HostCheck {
    fn refuse(&self, failure: Failure) -> Result<bool, russh::Error> {
        if let Ok(mut refused) = self.refused.lock() {
            *refused = Some(failure);
        }
        Ok(false)
    }

    fn mismatch(&self, key: &PublicKey, file: &Path, recorded: Recorded) -> KeyMismatch {
        KeyMismatch {
            host: known_hosts::host_name(&self.address, self.port),
            fingerprint: known_hosts::fingerprint(key),
            file: file.to_path_buf(),
            recorded,
        }
    }

    /// What the host-key files say of `key`, and the file that says it: revoked in any
    /// file is revoked, else recorded in any is known, else a different key is changed.
    fn verdict(&self, key: &PublicKey) -> (Verdict, PathBuf) {
        let mut verdicts: Vec<(Verdict, PathBuf)> = std::iter::once(&self.known_hosts)
            .chain(&self.also_known_hosts)
            .map(|file| {
                let text = std::fs::read_to_string(file).unwrap_or_default();
                let verdict = known_hosts::check(&text, &self.address, self.port, key);
                (verdict, file.clone())
            })
            .collect();
        let rank = |v: &Verdict| match v {
            Verdict::Revoked(_) => 0,
            Verdict::Known => 1,
            Verdict::Changed(_) => 2,
            Verdict::Unknown => 3,
        };
        verdicts.sort_by_key(|(v, _)| rank(v));
        verdicts.swap_remove(0)
    }
}

impl client::Handler for HostCheck {
    type Error = russh::Error;

    async fn disconnected(
        &mut self,
        reason: DisconnectReason<Self::Error>,
    ) -> Result<(), Self::Error> {
        self.out.watch.set(drop_reason(&reason));
        (self.out.wake)();
        match reason {
            DisconnectReason::ReceivedDisconnect(_) => Ok(()),
            DisconnectReason::Error(e) => Err(e),
        }
    }

    async fn check_server_key(
        &mut self,
        offered: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let PublicKeyOrCertificate::PublicKey { key, .. } = offered else {
            return self.refuse(Failure::Connection(
                "the host offered a certificate, which sshexplorer does not check".into(),
            ));
        };
        let (verdict, file) = self.verdict(key);
        match verdict {
            Verdict::Known => Ok(true),
            Verdict::Changed(recorded) => {
                let mismatch = self.mismatch(key, &file, recorded);
                self.refuse(Failure::HostKeyChanged(mismatch))
            }
            Verdict::Revoked(recorded) => {
                let mismatch = self.mismatch(key, &file, recorded);
                self.refuse(Failure::HostKeyRevoked(mismatch))
            }
            Verdict::Unknown => {
                let (answer, reply) = oneshot::channel();
                self.out.send(Event::TrustHostKey(TrustPrompt {
                    host: known_hosts::host_name(&self.address, self.port),
                    fingerprint: known_hosts::fingerprint(key),
                    answer,
                }));
                if self.out.answer(reply).await != Ok(true) {
                    return self.refuse(Failure::Cancelled);
                }
                match known_hosts::append(&self.known_hosts, &self.address, self.port, key) {
                    Ok(()) => Ok(true),
                    Err(e) => self.refuse(Failure::KnownHostsWrite {
                        file: self.known_hosts.clone(),
                        reason: e.to_string(),
                    }),
                }
            }
        }
    }
}
