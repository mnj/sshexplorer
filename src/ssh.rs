//! The SSH stack: opening a Session's connection and talking SFTP over it, off the egui
//! frame thread.
//!
//! The stack is russh with russh-sftp (ADR-0002, issue #4), on a tokio runtime that
//! [`Ssh`] owns. The frame thread never awaits: [`Ssh::open`] returns an [`Opening`] at
//! once, and the connection's progress arrives on it as [`Event`]s, each followed by a
//! call to the wake function (`egui::Context::request_repaint` in the app). A question
//! for the operator (trust this host key? the password?) is an event carrying a prompt;
//! answering it, or dropping it to cancel, lets the connection go on.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::client::{self, AuthResult};
use russh::keys::PublicKeyOrCertificate;
use russh::keys::ssh_key::{Algorithm, PublicKey};
use russh::{Disconnect, Preferred};
use russh_sftp::client::SftpSession;
use tokio::sync::oneshot;

use crate::known_hosts::{self, Recorded, Verdict};

/// How long the TCP connect may take before the Session fails, unless the config's
/// `ConnectTimeout` says otherwise. The handshake itself has no limit, so a trust prompt
/// can wait for the operator.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// How often a keepalive goes out, unless the config's `ServerAliveInterval` says.
const KEEPALIVE: Duration = Duration::from_secs(30);

/// How to connect, besides where: the host-key files, and the timings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// The `known_hosts` file, which Trust appends to.
    pub known_hosts: PathBuf,
    /// The config's `UserKnownHostsFile`s, checked as well; Trust never writes to them.
    pub also_known_hosts: Vec<PathBuf>,
    /// How long the TCP connect may take; `None` for 30 s.
    pub connect_timeout: Option<Duration>,
    /// How often a keepalive goes out; `None` for every 30 s, zero for never.
    pub keepalive: Option<Duration>,
}

impl Options {
    /// Checking host keys against `known_hosts` alone, with the default timings.
    pub fn new(known_hosts: PathBuf) -> Options {
        Options {
            known_hosts,
            also_known_hosts: Vec::new(),
            connect_timeout: None,
            keepalive: None,
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
        };
        let folder = folder.trim().to_owned();
        self.runtime
            .spawn(open(target, options, folder, Arc::new(out)));
        Opening { events }
    }
}

/// A Session being opened: its events, in order. The last is [`Event::Opened`] or
/// [`Event::Failed`].
pub struct Opening {
    events: mpsc::Receiver<Event>,
}

impl Opening {
    /// A Session that fails before it connects: its one event is [`Event::Failed`].
    pub fn failed(failure: Failure) -> Opening {
        let (sender, events) = mpsc::channel();
        let _ = sender.send(Event::Failed(failure));
        Opening { events }
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
    /// The password is needed.
    Password(PasswordPrompt),
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
            Failure::Sftp(reason) | Failure::Unsupported(reason) => write!(f, "{reason}"),
        }
    }
}

/// An open Session's connection, with the listing of the folder it opened in.
pub struct Opened {
    pub session: Session,
    /// The Initial remote folder, or the home directory when that is empty.
    pub home: Listing,
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
}

/// An open Session's connection. Dropping it closes the connection.
pub struct Session {
    handle: Arc<client::Handle<HostCheck>>,
    sftp: Arc<SftpSession>,
    runtime: tokio::runtime::Handle,
    wake: Wake,
}

impl Session {
    /// Starts listing the directory at the absolute `path`. A symlinked directory is
    /// followed. The answer arrives on the returned request, and wakes the frame.
    pub fn list(&self, path: String) -> Request<Listing> {
        let (sender, answer) = mpsc::channel();
        let sftp = self.sftp.clone();
        let wake = self.wake.clone();
        self.runtime.spawn(async move {
            let listing = list(&sftp, path).await.map_err(|e| reason(&e));
            let _ = sender.send(listing);
            wake();
        });
        Request { answer }
    }
}

/// A request to the host: its answer, or why there is none.
pub struct Request<T> {
    answer: mpsc::Receiver<Result<T, String>>,
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
    sender: mpsc::Sender<Event>,
    wake: Wake,
}

impl Events {
    fn send(&self, event: Event) {
        let _ = self.sender.send(event);
        (self.wake)();
    }
}

async fn open(target: Target, options: Options, folder: String, out: Arc<Events>) {
    let failure = match connect(&target, options, &folder, &out).await {
        Ok(opened) => {
            out.send(Event::Opened(opened));
            return;
        }
        Err(failure) => failure,
    };
    out.send(Event::Failed(failure));
}

async fn connect(
    target: &Target,
    options: Options,
    folder: &str,
    out: &Arc<Events>,
) -> Result<Opened, Failure> {
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
        known_hosts: options.known_hosts,
        also_known_hosts: options.also_known_hosts,
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
    let mut handle = match client::connect_stream(Arc::new(config), stream, check).await {
        Ok(handle) => handle,
        Err(e) => {
            let refused = refused.lock().ok().and_then(|mut r| r.take());
            return Err(refused.unwrap_or_else(|| Failure::Connection(e.to_string())));
        }
    };

    authenticate(&mut handle, target, out).await?;

    let sftp = start_sftp(&handle)
        .await
        .map_err(|e| Failure::Sftp(format!("The SFTP subsystem did not start: {e}")))?;
    let home = list_folder(&sftp, folder).await.map_err(|e| {
        let folder = if folder.is_empty() {
            "the home directory"
        } else {
            folder
        };
        Failure::Sftp(format!("Could not list {folder}: {e}"))
    })?;
    Ok(Opened {
        session: Session {
            handle: Arc::new(handle),
            sftp: Arc::new(sftp),
            runtime: tokio::runtime::Handle::current(),
            wake: out.wake.clone(),
        },
        home,
    })
}

/// Asks for the password until the host accepts one, or the operator cancels.
async fn authenticate(
    handle: &mut client::Handle<HostCheck>,
    target: &Target,
    out: &Events,
) -> Result<(), Failure> {
    let mut rejected = false;
    loop {
        let (answer, reply) = oneshot::channel();
        out.send(Event::Password(PasswordPrompt {
            account: format!("{}@{}", target.account, target.address),
            rejected,
            answer,
        }));
        let Ok(Some(password)) = reply.await else {
            let _ = handle
                .disconnect(Disconnect::ByApplication, "Cancelled", "en")
                .await;
            return Err(Failure::Cancelled);
        };
        match handle
            .authenticate_password(&target.account, password)
            .await
        {
            Ok(AuthResult::Success) => return Ok(()),
            Ok(AuthResult::Failure { .. }) => rejected = true,
            Err(e) => return Err(Failure::Connection(e.to_string())),
        }
    }
}

async fn start_sftp(
    handle: &client::Handle<HostCheck>,
) -> Result<SftpSession, Box<dyn std::error::Error>> {
    let channel = handle.channel_open_session().await?;
    channel.request_subsystem(true, "sftp").await?;
    Ok(SftpSession::new(channel.into_stream()).await?)
}

/// Lists `folder`, absolute or relative to the home directory (empty: the home itself).
async fn list_folder(
    sftp: &SftpSession,
    folder: &str,
) -> Result<Listing, russh_sftp::client::error::Error> {
    let folder = if folder.is_empty() { "." } else { folder };
    let path = sftp.canonicalize(folder).await?;
    list(sftp, path).await
}

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
async fn list(
    sftp: &SftpSession,
    path: String,
) -> Result<Listing, russh_sftp::client::error::Error> {
    let mut entries = Vec::new();
    for dir_entry in sftp.read_dir(path.clone()).await? {
        let name = dir_entry.file_name();
        let mut attrs = dir_entry.metadata();
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
        });
    }
    Ok(Listing { path, entries })
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
                if reply.await != Ok(true) {
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
