//! The Scratch folder: where Preview keeps the bytes it fetched from a host.
//!
//! In memory (the default) the bytes are a buffer. In the system temp directory or a
//! directory the operator chose, they are a file only the operator can read (0600 on
//! Unix; on Windows the per-user temp directory's ACL), named with [`PREFIX`] so that
//! nothing else in a chosen directory is ever touched. A kept file is deleted when its
//! [`Kept`] is dropped: when the preview changes, when its Session closes and at exit.
//! Files a crash left behind are cleared by [`clear_leftovers`] at the next launch.
//!
//! Each instance holds a lock on `sshexplorer-scratch-<pid>.lock` in every directory it
//! keeps files in, for as long as it runs. Clearing leaves the files of an instance
//! whose lock is still held, so a second instance does not delete what the first shows.

use std::borrow::Cow;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::settings::{PreviewSettings, ScratchChoice};

/// The start of every scratch file's name.
pub const PREFIX: &str = "sshexplorer-scratch-";

/// Where fetched bytes go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Place {
    Memory,
    /// Files in this directory, which is created when the first one is kept.
    Dir(PathBuf),
}

impl Place {
    /// The place the Preview settings choose. A Chosen directory left empty keeps the
    /// bytes in memory.
    pub fn of(settings: &PreviewSettings) -> Place {
        match settings.scratch {
            ScratchChoice::Memory => Place::Memory,
            ScratchChoice::Temp => Place::Dir(temp_dir()),
            ScratchChoice::Chosen => match &settings.scratch_dir {
                Some(dir) if !dir.as_os_str().is_empty() => Place::Dir(dir.clone()),
                _ => Place::Memory,
            },
        }
    }

    /// Keeps `bytes` here.
    pub fn keep(&self, bytes: Vec<u8>) -> io::Result<Kept> {
        let Place::Dir(dir) = self else {
            return Ok(Kept::Memory(bytes));
        };
        std::fs::create_dir_all(dir)?;
        hold_lock(dir)?;
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!("{PREFIX}{}-{n}", std::process::id()));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(&path)?;
        let kept = Kept::File {
            path,
            len: bytes.len(),
        };
        file.write_all(&bytes)?;
        Ok(kept)
    }
}

/// The system temp directory's own folder for sshexplorer, one per OS account.
pub fn temp_dir() -> PathBuf {
    let account = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default();
    std::env::temp_dir().join(format!("{PREFIX}{account}"))
}

/// Bytes kept in the Scratch folder. Dropping it deletes a kept file.
#[derive(Debug)]
pub enum Kept {
    Memory(Vec<u8>),
    File { path: PathBuf, len: usize },
}

impl Kept {
    /// The bytes, read back from the file when they are kept in one.
    pub fn bytes(&self) -> io::Result<Cow<'_, [u8]>> {
        match self {
            Kept::Memory(bytes) => Ok(Cow::Borrowed(bytes)),
            Kept::File { path, .. } => std::fs::read(path).map(Cow::Owned),
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Kept::Memory(bytes) => bytes.len(),
            Kept::File { len, .. } => *len,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The file the bytes are kept in, when they are.
    pub fn path(&self) -> Option<&Path> {
        match self {
            Kept::Memory(_) => None,
            Kept::File { path, .. } => Some(path),
        }
    }
}

impl Drop for Kept {
    fn drop(&mut self) {
        if let Kept::File { path, .. } = self {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Deletes the scratch files left in the places `settings` can name: the temp folder,
/// and the Chosen directory if one is set. Only files named with [`PREFIX`] go.
pub fn clear_leftovers(settings: &PreviewSettings) {
    clear_dir(&temp_dir());
    if let Some(dir) = &settings.scratch_dir {
        clear_dir(dir);
    }
}

/// The lock file of instance `pid` in `dir`.
fn lock_path(dir: &Path, pid: u32) -> PathBuf {
    dir.join(format!("{PREFIX}{pid}.lock"))
}

/// Takes this instance's lock in `dir`, once, and holds it until exit.
fn hold_lock(dir: &Path) -> io::Result<()> {
    static HELD: Mutex<Vec<(PathBuf, std::fs::File)>> = Mutex::new(Vec::new());
    let mut held = HELD.lock().unwrap_or_else(|e| e.into_inner());
    if held.iter().any(|(d, _)| d == dir) {
        return Ok(());
    }
    let file = std::fs::File::create(lock_path(dir, std::process::id()))?;
    file.lock()?;
    held.push((dir.to_owned(), file));
    Ok(())
}

/// The pid in a scratch file's name (`<PREFIX><pid>-<n>` or `<PREFIX><pid>.lock`).
fn pid_of(name: &str) -> Option<u32> {
    let rest = name.strip_prefix(PREFIX)?;
    let end = rest.find(['-', '.']).unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// Whether instance `pid` still runs: it holds its lock in `dir`.
fn running(dir: &Path, pid: u32) -> bool {
    let Ok(file) = std::fs::File::open(lock_path(dir, pid)) else {
        return false;
    };
    match file.try_lock() {
        Ok(()) => false,
        Err(std::fs::TryLockError::WouldBlock) => true,
        // A lock that cannot be checked is taken as held: nothing is lost by waiting.
        Err(std::fs::TryLockError::Error(_)) => true,
    }
}

/// Deletes the scratch files in `dir` that no running instance holds.
pub fn clear_dir(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut alive = std::collections::HashMap::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with(PREFIX) || !entry.file_type().is_ok_and(|t| t.is_file()) {
            continue;
        }
        let held = pid_of(&name).is_some_and(|pid| {
            *alive
                .entry(pid)
                .or_insert_with(|| running(dir, pid))
        });
        if !held {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}
