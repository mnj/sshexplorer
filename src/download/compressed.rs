//! Optional single-frame zstd transport. Publish only after the decoder, expected
//! length, and remote command's exit status all agree that the file is complete.
use super::CHUNK;
use super::transfer::{Error, Writer};
use crate::kind::Kind;
use crate::ssh::{self, Entry, Output, Remote};
use std::sync::atomic::{AtomicBool, Ordering};
use zstd::stream::raw::{Decoder, Operation};

const MIN_SIZE: u64 = 1024 * 1024;

pub(super) fn eligible(entry: &Entry) -> bool {
    entry.size.is_some_and(|n| n >= MIN_SIZE)
        && matches!(
            Kind::of(&entry.name, entry.is_dir),
            Kind::Text | Kind::Source | Kind::Markdown
        )
}

pub(super) async fn copy(
    remote: &Remote,
    path: &str,
    size: u64,
    out: &mut Writer,
    unavailable: &AtomicBool,
) -> Result<(), Error> {
    // Match Search's shell-independent argument encoding. The sentinel preserves
    // trailing newlines in the filename through POSIX command substitution.
    let arg = ssh::shell_arg(path);
    let command =
        format!(r#"exec sh -c 'p=$(printf "$1."); exec zstd -q -1 -c -- "${{p%.}}"' sh '{arg}'"#);
    let mut exec = remote.exec(&command).await.map_err(|reason| {
        if reason == ssh::EXEC_REFUSED {
            unavailable.store(true, Ordering::Relaxed);
        }
        Error::Remote(reason)
    })?;
    let mut decoder = Decoder::new().map_err(|e| Error::Remote(e.to_string()))?;
    decoder
        .set_parameter(zstd::zstd_safe::DParameter::WindowLogMax(23))
        .map_err(|e| Error::Remote(e.to_string()))?;
    let mut buffer = vec![0; CHUNK];
    let mut total = 0;
    let mut complete = false;
    let mut exit = None;
    while let Some(event) = exec.next().await {
        match event {
            Output::Stdout(data) if !data.is_empty() => {
                let mut input = data.as_slice();
                loop {
                    if complete {
                        return Err(Error::Remote("Trailing zstd data".into()));
                    }
                    let step = decoder
                        .run_on_buffers(input, &mut buffer)
                        .map_err(|e| Error::Remote(e.to_string()))?;
                    if !input.is_empty() && step.bytes_read == 0 && step.bytes_written == 0 {
                        return Err(Error::Remote("Invalid zstd stream".into()));
                    }
                    if step.bytes_written as u64 > size - total {
                        return Err(Error::Remote("Remote size changed".into()));
                    }
                    if step.bytes_written != 0 {
                        out.write(&buffer[..step.bytes_written]).await?;
                        total += step.bytes_written as u64;
                    }
                    input = &input[step.bytes_read..];
                    complete = step.remaining == 0;
                    if input.is_empty() && (complete || step.bytes_written < buffer.len()) {
                        break;
                    }
                }
            }
            Output::Exit(code) => exit = Some(code),
            _ => {}
        }
    }
    if matches!(exit, Some(126 | 127)) {
        unavailable.store(true, Ordering::Relaxed);
    }
    if exit == Some(0) && complete && total == size {
        Ok(())
    } else {
        Err(Error::Remote("Incomplete compressed download".into()))
    }
}
