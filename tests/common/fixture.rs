//! The test SSH/SFTP server: an in-process russh server on `127.0.0.1` that serves a
//! temporary directory over SFTP, for the tests that connect.
//!
//! Run it from any integration test with `mod common;` and
//! `common::fixture::Fixture::start()`. It needs nothing installed and runs under plain
//! `cargo test`. Each fixture listens on its own free port and has its own tokio runtime,
//! separate from the app's, which stops when the fixture is dropped.
//!
//! - **Accounts.** One account, [`ACCOUNT`], with the password [`PASSWORD`]. Every other
//!   password is rejected at once (no rejection delay). [`Fixture::start_accepting`]
//!   chooses the methods instead: authorized public keys, the password, and
//!   keyboard-interactive rounds ([`Keyboard`]). [`Fixture::attempts`] lists what the
//!   client tried, in order.
//! - **Host key.** [`HOST_KEY`] by default; [`Fixture::start_with`] takes another, such as
//!   [`OTHER_KEY`], and a test that wants a changed key records the other key in its
//!   `known_hosts`.
//! - **Files.** The filesystem is `root()`, a temp directory; the remote path `/x` is
//!   `root()/x`. The account's home is `/home/tester` and starts with [`HOME_ENTRIES`].
//!   Add files with `std::fs` under `root()`.
//! - **Reads.** Files open for reading only (open, read, fstat, close). A test that needs
//!   a Download or a read to be in flight slows every read with [`Fixture::slow_reads`].
//! - **Drops.** [`Fixture::drop_connections`] kills every open connection, as a network
//!   failure does; [`Fixture::stall`] stops answering on them, so keepalives go
//!   unanswered; [`Fixture::refuse`] closes new connections at once.
//!   [`Fixture::set_password`] changes [`ACCOUNT`]'s password.
//! - **Search.** An exec request runs the command under the local `sh`, with its
//!   start directory (the first argument after `sh -c '…' sh`, octal-escaped) mapped
//!   under `root()`, so the host's own `find` does the walking.
//!   [`Fixture::refuse_search`] refuses every exec request, and [`Fixture::hold_search`]
//!   keeps each Search running after `find` has printed everything, until it is killed.
//! - **Limits.** SFTP semantics are the fixture's own (no `longname`; `stat` follows
//!   links, while `lstat` and `readdir` describe a link itself, as OpenSSH does). For
//!   real OpenSSH behaviour, run an unprivileged `sshd` on a high port (public key only)
//!   or a podman container behind `#[ignore]`; see the implementer notes.

#![allow(dead_code)]

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use russh::keys::{PrivateKey, PublicKey};
use russh::server::{
    Auth, ChannelOpenHandle, Msg, PublicKeyOffer, Response, Session, VerifiedPublicKey,
};
use russh::{Channel, ChannelId};
use russh::{MethodKind, MethodSet};
use russh_sftp::protocol::{
    Attrs, Data, File, FileAttributes, Handle, Name, OpenFlags, SftpHandle, Status, StatusCode,
    Version,
};
use tokio::sync::Mutex;

pub const ACCOUNT: &str = "tester";
pub const PASSWORD: &str = "open sesame";
pub const HOME: &str = "/home/tester";
/// What the home directory holds at the start: two directories and two files.
pub const HOME_ENTRIES: &[&str] = &["docs", "notes.txt", "photos", "readme.md"];

/// The fixture's host key by default (an Ed25519 key made with `ssh-keygen`).
pub const HOST_KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
QyNTUxOQAAACDcYBUftfhyWSJphxkuwv9yRbNVFJO0qOysCX3YIhcCuAAAAIi4cT3YuHE9
2AAAAAtzc2gtZWQyNTUxOQAAACDcYBUftfhyWSJphxkuwv9yRbNVFJO0qOysCX3YIhcCuA
AAAECy4p/787C/zBNbvRhnzL84s2dJuOGSsb7H49hZ1vtNHtxgFR+1+HJZImmHGS7C/3JF
s1UUk7So7KwJfdgiFwK4AAAAAAECAwQF
-----END OPENSSH PRIVATE KEY-----
";
/// [`HOST_KEY`]'s fingerprint, as `ssh-keygen -lf` printed it.
pub const HOST_KEY_FINGERPRINT: &str = "SHA256:SNTYyYQ/dyYFDMjxVe4lzYHHBthrTPrxk7/btmdgrwk";

/// Another Ed25519 key, for a host whose key has changed.
pub const OTHER_KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
QyNTUxOQAAACC6cqxouB5XH3uk8V2QqeypbT8vI0JrepeNG+T+hXMFtAAAAIiYh/aVmIf2
lQAAAAtzc2gtZWQyNTUxOQAAACC6cqxouB5XH3uk8V2QqeypbT8vI0JrepeNG+T+hXMFtA
AAAEAscGvoqJkS/0PUQJRR3suoSyGc+XQ3q5PF0K75p+37grpyrGi4Hlcfe6TxXZCp7Klt
Py8jQmt6l40b5P6FcwW0AAAAAAECAwQF
-----END OPENSSH PRIVATE KEY-----
";
/// [`OTHER_KEY`]'s fingerprint, as `ssh-keygen -lf` printed it.
pub const OTHER_KEY_FINGERPRINT: &str = "SHA256:GNYCzvgTWmWZ+F5IPHky4My19zkrk8cFjh2/AN16Osw";

/// The one-time code the [`Keyboard::PasswordAndCode`] round asks for.
pub const CODE: &str = "424242";

/// What the server accepts.
#[derive(Clone, Default)]
pub struct Accepts {
    /// [`PASSWORD`] by the "password" method.
    pub password: bool,
    /// Public keys authorized for [`ACCOUNT`].
    pub keys: Vec<PublicKey>,
    pub keyboard: Keyboard,
    /// The server's own attempt limit (OpenSSH's `MaxAuthTries`), after which it closes
    /// the connection; `None` for russh's default of 10.
    pub max_tries: Option<usize>,
}

/// The keyboard-interactive rounds the server runs.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Keyboard {
    #[default]
    Off,
    /// One round with one echo-off prompt, "Password: ", answered by [`PASSWORD`].
    Password,
    /// One round with two prompts, "Password: " (echo off) and "Verification code: "
    /// (echoed), answered by [`PASSWORD`] and [`CODE`].
    PasswordAndCode,
}

/// A running test server. Dropping it stops the server.
pub struct Fixture {
    port: u16,
    host_key: PrivateKey,
    root: tempfile::TempDir,
    runtime: Option<tokio::runtime::Runtime>,
    /// How long each SFTP read waits before answering, in milliseconds.
    read_delay: Arc<AtomicU64>,
    attempts: Attempts,
    /// A second handle on each open connection's socket, shut by `drop_connections`.
    connections: Arc<std::sync::Mutex<Vec<std::net::TcpStream>>>,
    stalled: Arc<AtomicBool>,
    refusing: Arc<AtomicBool>,
    password: Arc<std::sync::Mutex<String>>,
    /// How exec requests are answered: [`EXEC_RUN`], [`EXEC_REFUSE`] or [`EXEC_HOLD`].
    exec: Arc<AtomicU8>,
    /// Holds the `find` wrapper [`Fixture::hold_search`] puts first on the `PATH`.
    bin: tempfile::TempDir,
}

const EXEC_RUN: u8 = 0;
const EXEC_REFUSE: u8 = 1;
const EXEC_HOLD: u8 = 2;

/// What clients tried, in order: "password", "keyboard-interactive", or "publickey "
/// and the key's `SHA256:` fingerprint.
type Attempts = Arc<std::sync::Mutex<Vec<String>>>;

impl Fixture {
    /// Starts a server with [`HOST_KEY`].
    pub fn start() -> Self {
        Self::start_with(HOST_KEY)
    }

    /// Starts a server with [`HOST_KEY`] that accepts what `accepts` says.
    pub fn start_accepting(accepts: Accepts) -> Self {
        Self::serve(HOST_KEY, accepts)
    }

    /// Starts a server whose host key is the OpenSSH private key `host_key`.
    pub fn start_with(host_key: &str) -> Self {
        Self::serve(
            host_key,
            Accepts {
                password: true,
                ..Accepts::default()
            },
        )
    }

    fn serve(host_key: &str, accepts: Accepts) -> Self {
        let host_key = PrivateKey::from_openssh(host_key).expect("fixture host key");
        let root = tempfile::tempdir().expect("fixture root");
        let home = root.path().join(HOME.trim_start_matches('/'));
        for dir in ["docs", "photos"] {
            std::fs::create_dir_all(home.join(dir)).unwrap();
        }
        std::fs::write(home.join("notes.txt"), "notes\n").unwrap();
        std::fs::write(home.join("readme.md"), "# readme\n").unwrap();
        std::fs::write(home.join(".profile"), "# hidden\n").unwrap();

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let listener = runtime
            .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut methods = Vec::new();
        if !accepts.keys.is_empty() {
            methods.push(MethodKind::PublicKey);
        }
        if accepts.keyboard != Keyboard::Off {
            methods.push(MethodKind::KeyboardInteractive);
        }
        if accepts.password {
            methods.push(MethodKind::Password);
        }
        let offered = MethodSet::from(methods.as_slice());
        let config = Arc::new(russh::server::Config {
            methods: offered.clone(),
            auth_rejection_time: Duration::ZERO,
            auth_rejection_time_initial: Some(Duration::ZERO),
            keys: vec![host_key.clone()],
            max_auth_attempts: accepts.max_tries.unwrap_or(10),
            ..Default::default()
        });
        let served = root.path().to_path_buf();
        let read_delay = Arc::new(AtomicU64::new(0));
        let delay = read_delay.clone();
        let attempts = Attempts::default();
        let tried = attempts.clone();
        let connections: Arc<std::sync::Mutex<Vec<std::net::TcpStream>>> = Arc::default();
        let stalled = Arc::new(AtomicBool::new(false));
        let refusing = Arc::new(AtomicBool::new(false));
        let password = Arc::new(std::sync::Mutex::new(PASSWORD.to_owned()));
        let (open, stall, refuse, secret) = (
            connections.clone(),
            stalled.clone(),
            refusing.clone(),
            password.clone(),
        );
        let exec = Arc::new(AtomicU8::new(EXEC_RUN));
        let bin = tempfile::tempdir().expect("fixture bin");
        let find = find_on_path().expect("find is installed");
        let wrapper = bin.path().join("find");
        std::fs::write(
            &wrapper,
            format!("#!/bin/sh\n'{}' \"$@\"\nexec sleep 600\n", find.display()),
        )
        .unwrap();
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let execs = exec.clone();
        let held_path = bin.path().to_path_buf();
        runtime.spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                if refuse.load(Ordering::SeqCst) {
                    drop(stream);
                    continue;
                }
                let handler = SshHandler {
                    root: served.clone(),
                    channels: Arc::default(),
                    read_delay: delay.clone(),
                    accepts: accepts.clone(),
                    methods: offered.clone(),
                    attempts: tried.clone(),
                    password: secret.clone(),
                    exec: execs.clone(),
                    bin: held_path.clone(),
                };
                let config = config.clone();
                // A second handle on the socket, to shut it from outside the session.
                let Ok(std) = stream.into_std() else {
                    continue;
                };
                let Ok(killer) = std.try_clone() else {
                    continue;
                };
                let Ok(stream) = tokio::net::TcpStream::from_std(std) else {
                    continue;
                };
                open.lock().unwrap().push(killer);
                let stream = Gated {
                    stream,
                    stalled: stall.clone(),
                };
                tokio::spawn(async move {
                    if let Ok(session) = russh::server::run_stream(config, stream, handler).await {
                        let _ = session.await;
                    }
                });
            }
        });
        Fixture {
            port,
            host_key,
            root,
            runtime: Some(runtime),
            read_delay,
            attempts,
            connections,
            stalled,
            refusing,
            password,
            exec,
            bin,
        }
    }

    /// Refuses every exec request from now on, as a host with no shell does.
    pub fn refuse_search(&self) {
        self.exec.store(EXEC_REFUSE, Ordering::SeqCst);
    }

    /// Keeps each Search from now on running once `find` has printed everything, until
    /// the client kills it.
    pub fn hold_search(&self) {
        self.exec.store(EXEC_HOLD, Ordering::SeqCst);
    }

    /// Kills every open connection, as a network failure would: the client sees its
    /// connection close without a word from the server. New connections are served.
    pub fn drop_connections(&self) {
        self.stalled.store(false, Ordering::SeqCst);
        for socket in self.connections.lock().unwrap().drain(..) {
            let _ = socket.shutdown(std::net::Shutdown::Both);
        }
    }

    /// Stops reading from every connection, open or new, so nothing is answered, until
    /// `drop_connections`.
    pub fn stall(&self) {
        self.stalled.store(true, Ordering::SeqCst);
    }

    /// Whether new connections are closed as soon as they are accepted.
    pub fn refuse(&self, refuse: bool) {
        self.refusing.store(refuse, Ordering::SeqCst);
    }

    /// Changes [`ACCOUNT`]'s password from [`PASSWORD`].
    pub fn set_password(&self, password: &str) {
        *self.password.lock().unwrap() = password.to_owned();
    }

    /// Makes every SFTP read from now on wait `delay` before it answers, so that a
    /// transfer stays in flight long enough to watch or cancel.
    pub fn slow_reads(&self, delay: Duration) {
        self.read_delay
            .store(delay.as_millis() as u64, Ordering::SeqCst);
    }

    /// What clients have tried so far, in order (see [`Attempts`]).
    pub fn attempts(&self) -> Vec<String> {
        self.attempts.lock().unwrap().clone()
    }

    pub fn address(&self) -> &'static str {
        "127.0.0.1"
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// The directory the server serves as `/`.
    pub fn root(&self) -> &Path {
        self.root.path()
    }

    /// The host's public key as a `known_hosts` line for this fixture's `[address]:port`.
    pub fn known_hosts_line(&self) -> String {
        Self::known_hosts_line_for(self, &self.host_key)
    }

    /// A `known_hosts` line recording `key` (an OpenSSH private key) for this fixture.
    pub fn known_hosts_line_with(&self, key: &str) -> String {
        Self::known_hosts_line_for(self, &PrivateKey::from_openssh(key).unwrap())
    }

    fn known_hosts_line_for(&self, key: &PrivateKey) -> String {
        let public = key.public_key().to_openssh().unwrap();
        format!("[{}]:{} {}", self.address(), self.port, public.trim_end())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

struct SshHandler {
    root: PathBuf,
    channels: Arc<Mutex<HashMap<ChannelId, Channel<Msg>>>>,
    read_delay: Arc<AtomicU64>,
    accepts: Accepts,
    /// The methods offered, which every refusal offers again, as OpenSSH does.
    methods: MethodSet,
    attempts: Attempts,
    password: Arc<std::sync::Mutex<String>>,
    exec: Arc<AtomicU8>,
    bin: PathBuf,
}

/// A connection's stream, which stops reading while `stalled`.
struct Gated {
    stream: tokio::net::TcpStream,
    stalled: Arc<AtomicBool>,
}

impl tokio::io::AsyncRead for Gated {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if self.stalled.load(Ordering::SeqCst) {
            // Look again later; nothing is read meanwhile.
            let waker = cx.waker().clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(50)).await;
                waker.wake();
            });
            return Poll::Pending;
        }
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl tokio::io::AsyncWrite for Gated {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

impl SshHandler {
    fn reject(&self) -> Auth {
        Auth::Reject {
            proceed_with_methods: Some(self.methods.clone()),
            partial_success: false,
        }
    }

    fn tried(&self, what: String) {
        self.attempts.lock().unwrap().push(what);
    }

    fn authorized(&self, user: &str, key: &PublicKey) -> bool {
        user == ACCOUNT
            && self
                .accepts
                .keys
                .iter()
                .any(|k| k.key_data() == key.key_data())
    }
}

impl russh::server::Handler for SshHandler {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        self.tried("password".into());
        let expected = self.password.lock().unwrap().clone();
        if self.accepts.password && user == ACCOUNT && password == expected {
            Ok(Auth::Accept)
        } else {
            Ok(self.reject())
        }
    }

    async fn auth_publickey_offered(
        &mut self,
        user: &str,
        offer: PublicKeyOffer<'_>,
    ) -> Result<Auth, Self::Error> {
        let key = offer.public_key;
        self.tried(format!("publickey {}", key.fingerprint(Default::default())));
        if self.authorized(user, key) {
            Ok(Auth::Accept)
        } else {
            Ok(self.reject())
        }
    }

    async fn auth_publickey(
        &mut self,
        user: &str,
        key: VerifiedPublicKey<'_>,
    ) -> Result<Auth, Self::Error> {
        if self.authorized(user, key.public_key) {
            Ok(Auth::Accept)
        } else {
            Ok(self.reject())
        }
    }

    async fn auth_keyboard_interactive<'a>(
        &'a mut self,
        user: &str,
        _submethods: &str,
        response: Option<Response<'a>>,
    ) -> Result<Auth, Self::Error> {
        let prompts: &'static [(Cow<'static, str>, bool)] = match self.accepts.keyboard {
            Keyboard::Off => return Ok(self.reject()),
            Keyboard::Password => &[(Cow::Borrowed("Password: "), false)],
            Keyboard::PasswordAndCode => &[
                (Cow::Borrowed("Password: "), false),
                (Cow::Borrowed("Verification code: "), true),
            ],
        };
        let Some(response) = response else {
            self.tried("keyboard-interactive".into());
            return Ok(Auth::Partial {
                name: "".into(),
                instructions: "".into(),
                prompts: Cow::Borrowed(prompts),
            });
        };
        let answers: Vec<String> = response
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .collect();
        let expected: &[&str] = match self.accepts.keyboard {
            Keyboard::Password => &[PASSWORD],
            _ => &[PASSWORD, CODE],
        };
        if user == ACCOUNT && answers == expected {
            Ok(Auth::Accept)
        } else {
            Ok(self.reject())
        }
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.channels.lock().await.insert(channel.id(), channel);
        reply.accept().await;
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.close(channel)?;
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel_id: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let mode = self.exec.load(Ordering::SeqCst);
        let channel = self.channels.lock().await.remove(&channel_id);
        let Some(channel) = channel.filter(|_| mode != EXEC_REFUSE) else {
            session.channel_failure(channel_id)?;
            return Ok(());
        };
        session.channel_success(channel_id)?;
        let command = map_start(&String::from_utf8_lossy(data), &self.root);
        let mut child = tokio::process::Command::new("sh");
        child
            .arg("-c")
            .arg(command)
            .current_dir(&self.root)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        if mode == EXEC_HOLD {
            let path = std::env::var("PATH").unwrap_or_default();
            child.env("PATH", format!("{}:{path}", self.bin.display()));
        }
        tokio::spawn(run_exec(child, channel));
        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        channel_id: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let channel = self.channels.lock().await.remove(&channel_id);
        match channel {
            Some(channel) if name == "sftp" => {
                session.channel_success(channel_id)?;
                let sftp = SftpHandler {
                    root: self.root.clone(),
                    dirs: HashMap::new(),
                    files: HashMap::new(),
                    next: 0,
                    read_delay: self.read_delay.clone(),
                };
                russh_sftp::server::run(channel.into_stream(), sftp).await;
            }
            _ => session.channel_failure(channel_id)?,
        }
        Ok(())
    }
}

/// Runs an exec request's command, sending its output on `channel`, and kills it when
/// the client closes the channel first.
async fn run_exec(mut command: tokio::process::Command, channel: Channel<Msg>) {
    use tokio::io::AsyncReadExt as _;
    let (mut read, write) = channel.split();
    let Ok(mut child) = command.spawn() else {
        let _ = write.exit_status(127).await;
        let _ = write.eof().await;
        let _ = write.close().await;
        return;
    };
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let (mut out, mut err) = ([0u8; 4096], [0u8; 4096]);
    let (mut out_open, mut err_open) = (true, true);
    loop {
        tokio::select! {
            n = stdout.read(&mut out), if out_open => match n {
                Ok(0) | Err(_) => out_open = false,
                Ok(n) => { let _ = write.data_bytes(out[..n].to_vec()).await; }
            },
            n = stderr.read(&mut err), if err_open => match n {
                Ok(0) | Err(_) => err_open = false,
                Ok(n) => { let _ = write.extended_data_bytes(1, err[..n].to_vec()).await; }
            },
            status = child.wait(), if !out_open && !err_open => {
                let code = status.ok().and_then(|s| s.code()).unwrap_or(143);
                let _ = write.exit_status(code as u32).await;
                let _ = write.eof().await;
                let _ = write.close().await;
                return;
            },
            msg = read.wait() => match msg {
                None | Some(russh::ChannelMsg::Close) | Some(russh::ChannelMsg::Eof) => {
                    let _ = child.kill().await;
                    return;
                }
                _ => {}
            },
        }
    }
}

/// The command with the start directory of a Search (the first argument after
/// `sh -c '…' sh`, octal-escaped) mapped under `root`. Any other command is unchanged.
fn map_start(command: &str, root: &Path) -> String {
    let marker = "' sh '";
    let Some(at) = command.find(marker) else {
        return command.to_owned();
    };
    let begin = at + marker.len();
    let Some(len) = command[begin..].find('\'') else {
        return command.to_owned();
    };
    let start = unescape(&command[begin..begin + len]);
    let local = root.join(String::from_utf8_lossy(&start).trim_start_matches('/'));
    let escaped: String = local
        .as_os_str()
        .as_encoded_bytes()
        .iter()
        .map(|b| format!("\\{b:03o}"))
        .collect();
    format!("{}{escaped}{}", &command[..begin], &command[begin + len..])
}

/// `printf`'s reading of `\ooo` octal escapes.
fn unescape(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\'
            && i + 4 <= bytes.len()
            && let Ok(b) = u8::from_str_radix(&text[i + 1..i + 4], 8)
        {
            out.push(b);
            i += 4;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

/// The `find` the fixture's `sh` would run.
fn find_on_path() -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join("find"))
        .find(|path| path.is_file())
}

/// SFTP over the served directory: enough for listing (realpath, stat, lstat, readlink,
/// opendir, readdir) and reading files (open, read, fstat, close).
struct SftpHandler {
    root: PathBuf,
    /// Open directory handles: the entries still to send, or None once sent.
    dirs: HashMap<String, Option<Vec<File>>>,
    /// Open file handles, for reading.
    files: HashMap<String, std::fs::File>,
    next: u64,
    read_delay: Arc<AtomicU64>,
}

impl SftpHandler {
    /// The remote path `path`, made absolute from the home and normalised.
    fn absolute(path: &str) -> String {
        let joined = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("{HOME}/{path}")
        };
        let mut parts: Vec<&str> = Vec::new();
        for part in joined.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    parts.pop();
                }
                p => parts.push(p),
            }
        }
        format!("/{}", parts.join("/"))
    }

    fn local(&self, path: &str) -> PathBuf {
        self.root.join(Self::absolute(path).trim_start_matches('/'))
    }

    fn ok(id: u32) -> Status {
        Status {
            id,
            status_code: StatusCode::Ok,
            error_message: "Ok".into(),
            language_tag: "en-US".into(),
        }
    }
}

fn status(error: std::io::Error) -> StatusCode {
    match error.kind() {
        std::io::ErrorKind::NotFound => StatusCode::NoSuchFile,
        std::io::ErrorKind::PermissionDenied => StatusCode::PermissionDenied,
        _ => StatusCode::Failure,
    }
}

impl russh_sftp::server::Handler for SftpHandler {
    type Error = StatusCode;

    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported
    }

    async fn init(
        &mut self,
        _version: u32,
        _extensions: HashMap<String, String>,
    ) -> Result<Version, Self::Error> {
        Ok(Version::new())
    }

    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        // As OpenSSH does, symlinks are resolved when the path exists.
        let mut real = Self::absolute(&path);
        if let (Ok(root), Ok(target)) = (
            std::fs::canonicalize(&self.root),
            std::fs::canonicalize(self.local(&path)),
        ) && let Ok(inside) = target.strip_prefix(&root)
        {
            real = format!("/{}", inside.to_string_lossy());
        }
        Ok(Name {
            id,
            files: vec![File::dummy(real)],
        })
    }

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        let metadata = std::fs::metadata(self.local(&path)).map_err(status)?;
        Ok(Attrs {
            id,
            attrs: FileAttributes::from(&metadata),
        })
    }

    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        let metadata = std::fs::symlink_metadata(self.local(&path)).map_err(status)?;
        Ok(Attrs {
            id,
            attrs: FileAttributes::from(&metadata),
        })
    }

    async fn readlink(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        let target = std::fs::read_link(self.local(&path)).map_err(status)?;
        Ok(Name {
            id,
            files: vec![File::dummy(target.to_string_lossy().into_owned())],
        })
    }

    async fn opendir(&mut self, id: u32, path: String) -> Result<Handle, Self::Error> {
        let mut files = Vec::new();
        for entry in std::fs::read_dir(self.local(&path)).map_err(status)? {
            let entry = entry.map_err(status)?;
            // As OpenSSH does, a link is described by itself, not its target.
            let metadata = std::fs::symlink_metadata(entry.path()).map_err(status)?;
            let name = entry.file_name().to_string_lossy().into_owned();
            files.push(File::new(name, FileAttributes::from(&metadata)));
        }
        self.next += 1;
        let handle = format!("dir{}", self.next);
        self.dirs.insert(handle.clone(), Some(files));
        Ok(Handle {
            id,
            handle: SftpHandle::from(&handle),
        })
    }

    async fn readdir(&mut self, id: u32, handle: SftpHandle) -> Result<Name, Self::Error> {
        let key = String::from_utf8_lossy(handle.as_bytes()).into_owned();
        match self.dirs.get_mut(&key) {
            Some(files @ Some(_)) => Ok(Name {
                id,
                files: files.take().unwrap_or_default(),
            }),
            Some(None) => Err(StatusCode::Eof),
            None => Err(StatusCode::Failure),
        }
    }

    async fn open(
        &mut self,
        id: u32,
        filename: String,
        pflags: OpenFlags,
        _attrs: FileAttributes,
    ) -> Result<Handle, Self::Error> {
        if pflags.bits() != OpenFlags::READ.bits() {
            return Err(StatusCode::PermissionDenied);
        }
        let local = self.local(&filename);
        if local.is_dir() {
            return Err(StatusCode::Failure);
        }
        let file = std::fs::File::open(local).map_err(status)?;
        self.next += 1;
        let handle = format!("file{}", self.next);
        self.files.insert(handle.clone(), file);
        Ok(Handle {
            id,
            handle: SftpHandle::from(&handle),
        })
    }

    async fn read(
        &mut self,
        id: u32,
        handle: SftpHandle,
        offset: u64,
        len: u32,
    ) -> Result<Data, Self::Error> {
        use std::io::{Read as _, Seek as _, SeekFrom};
        let delay = self.read_delay.load(Ordering::SeqCst);
        if delay > 0 {
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }
        let key = String::from_utf8_lossy(handle.as_bytes()).into_owned();
        let file = self.files.get_mut(&key).ok_or(StatusCode::Failure)?;
        file.seek(SeekFrom::Start(offset)).map_err(status)?;
        let mut data = vec![0; len as usize];
        let mut filled = 0;
        while filled < data.len() {
            match file.read(&mut data[filled..]).map_err(status)? {
                0 => break,
                n => filled += n,
            }
        }
        if filled == 0 {
            return Err(StatusCode::Eof);
        }
        data.truncate(filled);
        Ok(Data { id, data })
    }

    async fn fstat(&mut self, id: u32, handle: SftpHandle) -> Result<Attrs, Self::Error> {
        let key = String::from_utf8_lossy(handle.as_bytes()).into_owned();
        let file = self.files.get(&key).ok_or(StatusCode::Failure)?;
        let metadata = file.metadata().map_err(status)?;
        Ok(Attrs {
            id,
            attrs: FileAttributes::from(&metadata),
        })
    }

    async fn close(&mut self, id: u32, handle: SftpHandle) -> Result<Status, Self::Error> {
        let key = String::from_utf8_lossy(handle.as_bytes()).into_owned();
        self.dirs.remove(&key);
        self.files.remove(&key);
        Ok(Self::ok(id))
    }
}
