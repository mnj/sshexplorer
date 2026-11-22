//! The test SSH/SFTP server: an in-process russh server on `127.0.0.1` that serves a
//! temporary directory over SFTP, for the tests that connect.
//!
//! Run it from any integration test with `mod common;` and
//! `common::fixture::Fixture::start()`. It needs nothing installed and runs under plain
//! `cargo test`. Each fixture listens on its own free port and has its own tokio runtime,
//! separate from the app's, which stops when the fixture is dropped.
//!
//! - **Accounts.** One account, [`ACCOUNT`], with the password [`PASSWORD`]. Every other
//!   password is rejected at once (no rejection delay).
//! - **Host key.** [`HOST_KEY`] by default; [`Fixture::start_with`] takes another, such as
//!   [`OTHER_KEY`], and a test that wants a changed key records the other key in its
//!   `known_hosts`.
//! - **Files.** The filesystem is `root()`, a temp directory; the remote path `/x` is
//!   `root()/x`. The account's home is `/home/tester` and starts with [`HOME_ENTRIES`].
//!   Add files with `std::fs` under `root()`.
//! - **Limits.** SFTP semantics are the fixture's own (no `longname`; `stat` follows
//!   links, while `lstat` and `readdir` describe a link itself, as OpenSSH does). For
//!   real OpenSSH behaviour, run an unprivileged `sshd` on a high port (public key only)
//!   or a podman container behind `#[ignore]`; see the implementer notes.

#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use russh::keys::PrivateKey;
use russh::server::{Auth, ChannelOpenHandle, Msg, Session};
use russh::{Channel, ChannelId};
use russh_sftp::protocol::{
    Attrs, File, FileAttributes, Handle, Name, SftpHandle, Status, StatusCode, Version,
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

/// A running test server. Dropping it stops the server.
pub struct Fixture {
    port: u16,
    host_key: PrivateKey,
    root: tempfile::TempDir,
    runtime: Option<tokio::runtime::Runtime>,
}

impl Fixture {
    /// Starts a server with [`HOST_KEY`].
    pub fn start() -> Self {
        Self::start_with(HOST_KEY)
    }

    /// Starts a server whose host key is the OpenSSH private key `host_key`.
    pub fn start_with(host_key: &str) -> Self {
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
        let config = Arc::new(russh::server::Config {
            auth_rejection_time: Duration::ZERO,
            auth_rejection_time_initial: Some(Duration::ZERO),
            keys: vec![host_key.clone()],
            ..Default::default()
        });
        let served = root.path().to_path_buf();
        runtime.spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let handler = SshHandler {
                    root: served.clone(),
                    channels: Arc::default(),
                };
                let config = config.clone();
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
        }
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
}

impl russh::server::Handler for SshHandler {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        if user == ACCOUNT && password == PASSWORD {
            Ok(Auth::Accept)
        } else {
            Ok(Auth::reject())
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
                    next: 0,
                };
                russh_sftp::server::run(channel.into_stream(), sftp).await;
            }
            _ => session.channel_failure(channel_id)?,
        }
        Ok(())
    }
}

/// SFTP over the served directory: enough for listing (realpath, stat, lstat, readlink,
/// opendir, readdir).
struct SftpHandler {
    root: PathBuf,
    /// Open directory handles: the entries still to send, or None once sent.
    dirs: HashMap<String, Option<Vec<File>>>,
    next: u64,
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
        Ok(Name {
            id,
            files: vec![File::dummy(Self::absolute(&path))],
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

    async fn close(&mut self, id: u32, handle: SftpHandle) -> Result<Status, Self::Error> {
        self.dirs
            .remove(String::from_utf8_lossy(handle.as_bytes()).as_ref());
        Ok(Self::ok(id))
    }
}
