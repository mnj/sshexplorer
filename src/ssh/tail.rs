//! A bounded snapshot of the end of a file, or bytes appended since the last read.
use super::{Remote, Request, Session};
use std::io::SeekFrom;
use tokio::io::{AsyncReadExt as _, AsyncSeekExt as _};

pub(crate) const WINDOW: usize = 64 * 1024;
const OVERLAP: usize = 256;

#[derive(Clone, Default)]
pub(crate) struct Cursor {
    end: u64,
    modified: Option<u32>,
    suffix: Vec<u8>,
}

pub(crate) struct Chunk {
    pub replace: bool,
    pub bytes: Vec<u8>,
    pub cursor: Cursor,
}

impl Session {
    pub(crate) fn tail(&self, path: String, cursor: Option<Cursor>) -> Request<Chunk> {
        let (tx, answer) = std::sync::mpsc::channel();
        let remote = self.remote();
        let wake = self.waker();
        let task = self.runtime.spawn(async move {
            let _ = tx.send(read(&remote, path, cursor).await);
            wake();
        });
        Request {
            answer,
            task: Some(task.abort_handle()),
        }
    }
}

async fn read(remote: &Remote, path: String, previous: Option<Cursor>) -> Result<Chunk, String> {
    // Open by path on each poll to pick up rotation. Stat and read the same handle.
    let mut file = remote.open(path).await?;
    let attrs = file.metadata().await.map_err(|e| e.to_string())?;
    let size = attrs.size.ok_or("The host omitted the file size")?;
    let start = size.saturating_sub(WINDOW as u64);
    let mut offset = start;
    let mut replace = true;
    let mut suffix = Vec::new();
    if let Some(old) = previous
        && (start..=size).contains(&old.end)
        && (old.end != size || old.modified == attrs.mtime)
    {
        let anchor = old.end - old.suffix.len() as u64;
        file.seek(SeekFrom::Start(anchor))
            .await
            .map_err(|e| e.to_string())?;
        let mut check = Vec::new();
        (&mut file)
            .take(old.suffix.len() as u64)
            .read_to_end(&mut check)
            .await
            .map_err(|e| e.to_string())?;
        if check == old.suffix {
            offset = old.end;
            replace = false;
            suffix = check;
        }
    }
    if replace {
        file.seek(SeekFrom::Start(offset))
            .await
            .map_err(|e| e.to_string())?;
    }
    let mut bytes = Vec::new();
    file.take(size - offset)
        .read_to_end(&mut bytes)
        .await
        .map_err(|e| e.to_string())?;
    let end = offset + bytes.len() as u64;
    suffix.extend_from_slice(&bytes);
    let suffix = suffix.split_off(suffix.len().saturating_sub(OVERLAP));
    Ok(Chunk {
        replace,
        bytes,
        cursor: Cursor {
            end,
            modified: attrs.mtime,
            suffix,
        },
    })
}
