//! One file in flight. A staged file accounts for its bytes until it is published
//! or dropped, including when another transfer stops the whole download.
use super::{CHUNK, Options, Progress, WAKE_EVERY, compressed, keep_metadata, local};
use crate::ssh::{Entry, Remote};
use cap_std::fs::Dir;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

pub(super) struct File {
    pub dir: Arc<Dir>,
    pub path: String,
    pub rel: Vec<String>,
    pub entry: Entry,
    pub name: String,
    pub overwrite: bool,
}

pub(super) enum Error {
    Lost,
    Remote(String),
    Local(String),
}

pub(super) struct Staged {
    pub partial: local::Partial,
    bytes: u64,
    progress: Arc<Mutex<Progress>>,
}

impl Staged {
    pub fn published(&mut self) {
        self.bytes = 0;
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        self.progress.lock().unwrap().bytes_done -= self.bytes;
    }
}

pub(super) struct Writer {
    output: tokio::fs::File,
    staged: Staged,
    wake: Arc<dyn Fn() + Send + Sync>,
    last_wake: Instant,
}

impl Writer {
    fn new(
        file: &File,
        progress: Arc<Mutex<Progress>>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> io::Result<Self> {
        let (partial, output) = local::Partial::create(file.dir.clone())?;
        Ok(Self {
            output: output.into(),
            staged: Staged {
                partial,
                bytes: 0,
                progress,
            },
            wake,
            last_wake: Instant::now(),
        })
    }

    pub async fn write(&mut self, data: &[u8]) -> Result<(), Error> {
        self.output
            .write_all(data)
            .await
            .map_err(|e| Error::Local(e.to_string()))?;
        self.staged.bytes += data.len() as u64;
        self.staged.progress.lock().unwrap().bytes_done += data.len() as u64;
        if self.last_wake.elapsed() >= WAKE_EVERY {
            (self.wake)();
            self.last_wake = Instant::now();
        }
        Ok(())
    }

    async fn finish(mut self, entry: &Entry, options: Options) -> io::Result<Staged> {
        self.output.flush().await?;
        let file = self.output.into_std().await;
        keep_metadata(&file, entry, options)?;
        Ok(self.staged)
    }
}

impl File {
    pub fn local_name(&self) -> String {
        let mut rel = self.rel.clone();
        *rel.last_mut().expect("a file has a name") = self.name.clone();
        rel.join("/")
    }

    pub async fn fetch(
        &self,
        remote: &Remote,
        options: Options,
        progress: Arc<Mutex<Progress>>,
        wake: Arc<dyn Fn() + Send + Sync>,
        unavailable: &AtomicBool,
    ) -> Result<Staged, Error> {
        let local_error = |e: io::Error| Error::Local(e.to_string());
        let writer = || Writer::new(self, progress.clone(), wake.clone()).map_err(local_error);
        if options.compress
            && compressed::eligible(&self.entry)
            && !unavailable.load(Ordering::Relaxed)
        {
            let mut out = writer()?;
            match compressed::copy(
                remote,
                &self.path,
                self.entry.size.unwrap(),
                &mut out,
                unavailable,
            )
            .await
            {
                Ok(()) => return out.finish(&self.entry, options).await.map_err(local_error),
                Err(Error::Remote(_)) if remote.is_closed() => return Err(Error::Lost),
                // A fresh staging file starts at byte zero on SFTP fallback.
                Err(Error::Remote(_)) => {}
                Err(err) => return Err(err),
            }
        }
        let mut source = remote
            .open(self.path.clone())
            .await
            .map_err(Error::Remote)?;
        let mut out = writer()?;
        let mut buffer = vec![0; CHUNK];
        loop {
            let n = source
                .read(&mut buffer)
                .await
                .map_err(|e| Error::Remote(e.to_string()))?;
            if n == 0 {
                break;
            }
            out.write(&buffer[..n]).await?;
        }
        out.finish(&self.entry, options).await.map_err(local_error)
    }
}
